import { type ReactNode, useState } from "react";

import type { Completion } from "@zuihitsu/wire/types/Completion.ts";
import type { ImagePart } from "@zuihitsu/wire/types/ImagePart.ts";
import type { Message } from "@zuihitsu/wire/types/Message.ts";
import type { PromptSectionKind } from "@zuihitsu/wire/types/PromptSectionKind.ts";
import type { ToolCall } from "@zuihitsu/wire/types/ToolCall.ts";
import type { CacheVerdict } from "../../lib/model/cachePath.ts";
import type { ModelInteraction } from "../../lib/model/interactions.ts";
import type { DigestStatus } from "@zuihitsu/wire/wasm/console_wasm.js";
import { type Warmth, causeLabel } from "./turnUtilities.ts";
import {
  type AttributedRow,
  type CallAttribution,
  type RowCost,
  type TokenProvenance,
  estimateTokens,
  rowProvenance,
  rowWeight,
} from "../../lib/model/tokenAttribution.ts";
import { resolveSections } from "../../lib/model/promptSections.ts";
import { useBlobSource } from "../../lib/view/blobSource.ts";
import { formatTokens } from "../../lib/format/format.ts";
import { Excerpt } from "../../components/primitives.tsx";
import { Lua } from "../../components/Lua.tsx";
import { ThinkingMarkdown } from "../../components/ThinkingMarkdown.tsx";
import { TurnMarkdown } from "./TurnMarkdown.tsx";

/// One denominator the panel reads sizes against: the model's context window when the log records
/// it, else the compaction budget.
export interface Denominator {
  value: number | null;
  label: string;
}

/// Where a call's context went, Bound-style: a stacked bar whose segments are the prompt's parts in
/// send order (free space as bare ground), then one row per part — swatch, name, token count with
/// its provenance badge, and percent of the window — every row expandable to the exact content it
/// measures, with the message history indented beneath its rollup and the call's completion footing
/// the list. Base calls and continuations render identically: a continuation's shared prefix is
/// itemized like any other prompt, apportioned within its measured lump.
export function ContextSectionList({
  interaction,
  attribution,
  denominator,
  verdict,
}: {
  interaction: ModelInteraction;
  attribution: CallAttribution;
  denominator: Denominator;
  verdict?: CacheVerdict;
}) {
  const [expanded, setExpanded] = useState<ReadonlySet<string>>(new Set());

  // The row whose content broke the prefix cache, when the verdict names one — the brief that
  // re-froze, the tool set that changed, or the rebuilt history. A first call has no culprit;
  // everything was new.
  const coldAt = verdict?.path === "cold" ? coldRowKey(verdict) : null;
  const coldTitle = verdict ? `The prefix cache broke here — ${causeLabel(verdict)}.` : undefined;

  const sections = resolveSections(interaction.system, interaction.systemSections);
  const inferred = sections.some((section) => section.provenance === "inferred");
  const systemRows = attribution.rows.filter((row) => row.sectionKind !== undefined);
  const toolsRow = attribution.rows.find((row) => row.key === "tools");
  const messageRows = attribution.rows.filter((row) => row.messageIndex !== undefined);
  const historyTokens = messageRows.reduce((sum, row) => sum + rowWeight(row.cost), 0);
  // The bar and the percents normalize against the denominator when known, else the call's own
  // total — the parts stay proportionate either way.
  const scale = denominator.value ?? attribution.total;
  const freeTokens = denominator.value !== null ? Math.max(0, scale - attribution.total) : 0;
  const completionTokens =
    interaction.usage.completion_tokens ??
    estimateTokens((interaction.reasoning ?? "") + completionText(interaction.completion));

  const isOpen = (key: string) => expanded.has(key);
  function toggle(key: string) {
    setExpanded((previous) => {
      const next = new Set(previous);
      if (!next.delete(key)) next.add(key);
      return next;
    });
  }

  function sectionText(kind: PromptSectionKind | undefined): string | null {
    if (kind === undefined) return null;
    const section = sections.find((candidate) => candidate.kind === kind);
    return section ? interaction.system.slice(section.start, section.end) : null;
  }

  // One part list drives both the stacked bar and the top-level rows, so they cannot disagree.
  const parts: Part[] = [
    ...systemRows.map((row) => ({
      key: row.key,
      label: row.label,
      cost: row.cost,
      provenance: rowProvenance(row.cost, attribution.totalProvenance),
      swatch: sectionSwatch(row.sectionKind),
      cold: row.key === coldAt,
      coldTitle,
      detail: () => sectionText(row.sectionKind),
    })),
    ...(toolsRow
      ? [
          {
            key: "tools",
            label: "tools",
            cost: toolsRow.cost,
            provenance: rowProvenance(toolsRow.cost, attribution.totalProvenance),
            swatch: "bg-clay",
            cold: coldAt === "tools",
            coldTitle,
            detail: () => JSON.stringify(interaction.tools, null, 2),
          },
        ]
      : []),
    ...(messageRows.length > 0
      ? [
          {
            key: "history",
            label: `history · ${messageRows.length} message${messageRows.length === 1 ? "" : "s"}`,
            cost: { kind: "measured" as const, tokens: historyTokens },
            provenance: rollupProvenance(messageRows, attribution.totalProvenance),
            swatch: "bg-ink-faint",
            cold: coldAt === "history",
            coldTitle,
          },
        ]
      : []),
  ];

  // The rows below the bar are grouped by measured block: each block states what the provider
  // charged for it, and its members state their own measurement or their share of it.
  const rowByKey = new Map(attribution.rows.map((row) => [row.key, row]));
  const blockRows: RowSpec[] = attribution.blocks.flatMap((block): RowSpec[] => {
    const members = block.rows
      .map((key) => rowByKey.get(key))
      .filter((row): row is AttributedRow => row !== undefined);
    const open = isOpen(block.key);
    return [
      {
        key: block.key,
        label: `${block.label} · ${members.length} part${members.length === 1 ? "" : "s"}`,
        cost: { kind: "measured", tokens: block.tokens },
        provenance: attribution.totalProvenance === "estimated" ? "estimated" : "measured",
        swatch: "bg-ink-faint",
        toggles: true,
        hint: "What the provider charged for this region of the prompt.",
      },
      ...(open
        ? members.map((row) => ({
            key: `${block.key}/${row.key}`,
            label: row.label,
            cost: row.cost,
            provenance: rowProvenance(row.cost, attribution.totalProvenance),
            swatch:
              row.sectionKind !== undefined
                ? sectionSwatch(row.sectionKind)
                : row.key === "tools"
                  ? "bg-clay"
                  : "bg-ink-faint opacity-50",
            child: true,
            cold: row.key === coldAt,
            coldTitle,
            detail:
              row.messageIndex !== undefined
                ? () => <MessageDetail message={interaction.messages[row.messageIndex ?? -1]} />
                : row.sectionKind !== undefined
                  ? () => sectionText(row.sectionKind)
                  : row.key === "tools"
                    ? () => JSON.stringify(interaction.tools, null, 2)
                    : undefined,
            alwaysDetailed: row.messageIndex !== undefined,
          }))
        : []),
    ];
  });

  const rows: RowSpec[] = [
    ...blockRows,
    {
      key: "completion",
      label: "completion",
      cost: { kind: "measured" as const, tokens: completionTokens },
      provenance: interaction.usage.completion_tokens !== null ? "measured" : "estimated",
      swatch: "bg-sage",
      toggles: true,
      detail: () => (
        <CompletionDetail completion={interaction.completion} reasoning={interaction.reasoning} />
      ),
      hint: "What this call generated — the next call's context grows by it.",
    },
    ...(denominator.value !== null
      ? [
          {
            key: "free",
            label: `free space (${denominator.label})`,
            cost: { kind: "measured" as const, tokens: freeTokens },
            swatch: "border border-line",
            faint: true,
          },
        ]
      : []),
  ];

  return (
    <div className="mt-2 border-l border-line pl-4">
      {inferred && (
        <p className="mb-1.5 font-mono text-2xs text-ink-faint italic">
          Section boundaries are inferred from the prompt's headers; this call was recorded before
          sections were captured.
        </p>
      )}

      <StackedBar segments={parts} scale={scale} freeTokens={freeTokens} />

      <div className="mt-1.5 flex flex-col">
        {rows.map((row) => (
          <Row
            key={row.key}
            spec={row}
            scale={scale}
            open={row.alwaysDetailed || isOpen(row.key)}
            onToggle={row.toggles ? () => toggle(row.key) : undefined}
          />
        ))}
      </div>
    </div>
  );
}

/// One attributable part of the prompt, shared by the bar and the row list.
interface Part {
  key: string;
  label: string;
  /// What this part costs: a measurement, or a share of the block it sits in. The bar sizes by
  /// [`rowWeight`] either way — a bar is a shape — while only a measurement prints a token count.
  cost: RowCost;
  swatch: string;
  provenance?: TokenProvenance;
  cold?: boolean;
  /// The cache-break marker's tooltip, carrying the attributed cause.
  coldTitle?: string;
  /// The exact content this part measures, built lazily when the row opens.
  detail?: () => ReactNode;
}

/// A row below the bar: a part, plus the non-part rows (messages, completion, free space) and
/// their display modifiers.
interface RowSpec extends Part {
  toggles?: boolean;
  child?: boolean;
  faint?: boolean;
  hint?: string;
  /// Render the detail whenever visible (history's messages), rather than behind a toggle.
  alwaysDetailed?: boolean;
}

/// The one-line summary a context disclosure's heading carries: the measured cache warmth, the
/// prompt tokens in, the completion tokens out, the re-prefill cost, a slim fill toward the
/// compaction budget with its percent, and the digest verification. Inline elements only, since it
/// renders inside the disclosure button's summary span.
export function ContextHeading({
  warm,
  tokensIn,
  tokensOut,
  reprefilled,
  budget,
  digest,
}: {
  warm: Warmth | null;
  tokensIn: number | null;
  tokensOut: number | null;
  reprefilled: number | null;
  budget: number | null;
  digest?: DigestStatus;
}) {
  const fraction =
    budget !== null && budget > 0 && tokensIn !== null ? Math.min(1, tokensIn / budget) : null;
  return (
    <>
      {warm && (
        <span
          className={
            warm.tone === "sage"
              ? "text-sage"
              : warm.tone === "clay"
                ? "text-clay"
                : "text-ink-soft"
          }
          title={warm.title}
        >
          {warm.label}
        </span>
      )}
      {tokensIn !== null && (
        <span title="Prompt tokens this call read — the whole context it was sent.">
          {" "}
          · {formatTokens(tokensIn)} ↓
        </span>
      )}
      {tokensOut !== null && tokensOut > 0 && (
        <span title="Tokens generated — the reasoning and the message.">
          {" "}
          · {formatTokens(tokensOut)} ↑
        </span>
      )}
      {reprefilled !== null && reprefilled > 0 && (
        <span title="Prompt tokens the provider had to encode fresh this call — the slice past the shared cache prefix. On a turn's later steps this is the appended messages; the generated reasoning is never resent, so its cache entries are dropped rather than reused.">
          {" "}
          · {formatTokens(reprefilled)} ↻
        </span>
      )}
      {fraction !== null && budget !== null && tokensIn !== null ? (
        <>
          {" · "}
          <span
            className="inline-block h-1 w-16 translate-y-px bg-oat align-baseline"
            title={`Fill toward the compaction budget (${formatTokens(budget)}) — the point where the agent re-segments.`}
          >
            <span
              className={`block h-1 ${fraction >= 0.8 ? "bg-clay" : "bg-sage"}`}
              style={{ width: `${fraction * 100}%` }}
            />
          </span>
          <span>
            {" "}
            {Math.round(fraction * 100)}% ({formatTokens(tokensIn)}/{formatTokens(budget)})
          </span>
        </>
      ) : (
        // Only a truly unrecorded budget reads as unknown; a recorded zero is silently unmetered.
        tokensIn !== null && budget === null && <span> · budget unknown</span>
      )}
      {digest === "verified" && (
        <span
          className="text-sage"
          title="The prompt reconstructed from the recorded deltas hashes to the digest recorded at send time — this display provably matches the wire request."
        >
          {" "}
          · ✓
        </span>
      )}
      {digest === "mismatch" && (
        <span
          className="text-clay"
          title="The reconstruction does not hash to the digest recorded at send time — what is shown here may not be what was sent."
        >
          {" "}
          · ⚠ digest mismatch
        </span>
      )}
    </>
  );
}

/// The breakdown row a cold verdict blames: the diverging system section, the changed tool set, or
/// the rebuilt history. `null` when nothing is blamable (a first call — everything was new).
function coldRowKey(verdict: CacheVerdict): string | null {
  switch (verdict.cause) {
    case "system-changed":
      return verdict.divergence?.sectionKind ? `section:${verdict.divergence.sectionKind}` : null;
    case "tools-changed":
      return "tools";
    case "tool-ids-reminted":
    case "new-session":
    case "buffer-rewritten":
      return "history";
    default:
      return null;
  }
}

/// The Bound-style stacked bar: one flex segment per part, proportioned against the scale, free
/// space as bare ground behind a hairline. Each segment carries its own tooltip.
function StackedBar({
  segments,
  scale,
  freeTokens,
}: {
  segments: Part[];
  scale: number;
  freeTokens: number;
}) {
  if (scale <= 0) return null;
  return (
    <div className="flex h-3.5 w-full border border-line bg-oat/40">
      {segments
        .filter((segment) => rowWeight(segment.cost) > 0)
        .map((segment) => (
          <div
            key={segment.key}
            className={
              `min-w-0.5 border-r border-paper/60 last:border-r-0 ${segment.swatch}` +
              (segment.cold ? " ring-1 ring-clay ring-inset" : "")
            }
            style={{ flexBasis: `${(rowWeight(segment.cost) / scale) * 100}%` }}
            title={
              `${segment.label}: ${segmentSize(segment.cost, scale)}` +
              (segment.cold ? " — the cache broke here" : "")
            }
          />
        ))}
      {freeTokens > 0 && (
        <div
          className="min-w-0"
          style={{ flexBasis: `${(freeTokens / scale) * 100}%` }}
          title={`free space: ${freeTokens.toLocaleString()} tokens (${((freeTokens / scale) * 100).toFixed(1)}%)`}
        />
      )}
    </div>
  );
}

/// One list row: rectangle swatch, name, right-aligned tabular tokens with the provenance badge,
/// right-aligned percent. Expandable rows toggle the exact content they measure.
function Row({
  spec,
  scale,
  open,
  onToggle,
}: {
  spec: RowSpec;
  scale: number;
  open: boolean;
  onToggle?: () => void;
}) {
  const detail = open ? (spec.detail?.() ?? null) : null;
  const body = (
    <>
      <span aria-hidden className={`h-3 w-1.5 shrink-0 ${spec.swatch}`} />
      <span
        className={`min-w-0 flex-1 truncate text-left ${spec.faint ? "text-ink-faint" : "text-ink-soft"}`}
      >
        {onToggle && (
          <span className="mr-1 inline-block w-2 text-ink-faint">{open ? "▾" : "▸"}</span>
        )}
        {spec.label}
        {spec.cold && (
          <span
            className="ml-1.5 text-clay"
            title={
              spec.coldTitle ??
              "This is where the prompt stopped matching the previous call's — the prefix cache broke here."
            }
          >
            ⚡ cache broke here
          </span>
        )}
      </span>
      {spec.provenance === "estimated" && (
        <span className={badgeClass(spec.provenance)} title={badgeTitle(spec.provenance)}>
          estimated
        </span>
      )}
      <span className="w-14 text-right text-ink-soft tabular-nums">
        {spec.cost.kind === "measured" ? formatTokens(spec.cost.tokens) : ""}
      </span>
      <span className="w-12 text-right text-ink-faint tabular-nums" title={spec.hint}>
        {spec.cost.kind === "share"
          ? `${spec.cost.percent.toFixed(1)}%`
          : scale > 0
            ? `${((spec.cost.tokens / scale) * 100).toFixed(1)}%`
            : ""}
      </span>
    </>
  );
  return (
    <div className={spec.child ? "pl-5" : ""}>
      {onToggle ? (
        <button
          onClick={onToggle}
          aria-expanded={open}
          className="flex w-full items-center gap-2 border-b border-line/50 py-1 font-mono text-2xs transition-colors hover:text-ink"
        >
          {body}
        </button>
      ) : (
        <div className="flex items-center gap-2 border-b border-line/50 py-1 font-mono text-2xs">
          {body}
        </div>
      )}
      {detail != null &&
        detail !== "" &&
        (typeof detail === "string" ? <Excerpt className="my-1.5">{detail}</Excerpt> : detail)}
    </div>
  );
}

/// A history message, pretty-printed: its content as the transcript's Markdown, the images it showed
/// the model, and each tool call's Lua (falling back to raw arguments when a call carries no script).
/// Set off like an excerpt so it reads as quoted material.
///
/// The images are the point of this view: the text says only that a file was attached, and the
/// recorded request carries each image's address.
function MessageDetail({ message }: { message: Message | undefined }) {
  if (!message) return null;
  const images = message.images ?? [];
  return (
    <div className="my-1.5 flex flex-col gap-2 border-l border-line bg-oat/40 px-3 py-2">
      {message.content && <TurnMarkdown text={message.content} />}
      {images.length > 0 && <MessageImages images={images} />}
      {message.tool_calls.map((call) => (
        <ToolCallDetail key={call.id} call={call} />
      ))}
      {!message.content && images.length === 0 && message.tool_calls.length === 0 && (
        <p className="font-mono text-xs text-ink-faint">(empty)</p>
      )}
    </div>
  );
}

/// The images one message showed the model. One whose bytes this frame cannot reach states its
/// address instead, so the reader still learns a picture was there.
function MessageImages({ images }: { images: ImagePart[] }) {
  const source = useBlobSource();
  return (
    <div className="flex flex-wrap items-start gap-2">
      {images.map((image, index) => {
        const url = source.urlFor(image);
        // Keyed by position: identical bytes share an address, and may ride a message twice.
        if (url === null) {
          return (
            <span key={index} className="font-mono text-2xs text-ink-faint">
              image · {image.mime} · {image.blob.slice(0, 12)}…
            </span>
          );
        }
        return (
          <a key={index} href={url} target="_blank" rel="noreferrer" className="block">
            <img
              src={url}
              alt={`image shown to the model (${image.mime})`}
              loading="lazy"
              className="max-h-40 max-w-full rounded-xs border border-line object-contain"
            />
          </a>
        );
      })}
    </div>
  );
}

function ToolCallDetail({ call }: { call: ToolCall }) {
  let script: string | null = null;
  try {
    const args: unknown = JSON.parse(call.arguments);
    if (
      typeof args === "object" &&
      args !== null &&
      "script" in args &&
      typeof (args as { script: unknown }).script === "string"
    ) {
      script = (args as { script: string }).script;
    }
  } catch {
    // Unparseable arguments render raw below.
  }
  return script ? (
    <Lua code={script} />
  ) : (
    <p className="font-mono text-xs whitespace-pre-wrap text-ink-soft">
      → {call.name}({call.arguments})
    </p>
  );
}

/// What the call generated, pretty-printed like the transcript: the reasoning first (it is part of
/// the generation and of `completion_tokens`, even though it never re-enters the context), then a
/// reply as Markdown, tool calls as highlighted Lua, or silence named.
function CompletionDetail({
  completion,
  reasoning,
}: {
  completion: Completion;
  reasoning: string | null;
}) {
  return (
    <div className="my-1.5 flex flex-col gap-2 border-l border-line bg-oat/40 px-3 py-2">
      {reasoning && (
        <div>
          <p
            className="mb-1 font-mono text-2xs tracking-widest text-ink-faint uppercase"
            title="Generated and counted in the completion tokens, but never resent — the next call's prompt carries only the message below."
          >
            reasoning
          </p>
          <div className="font-serif">
            <ThinkingMarkdown text={reasoning} />
          </div>
        </div>
      )}
      {completion === "Silent" ? (
        <p className="text-sm text-ink-faint italic">stayed silent</p>
      ) : "Reply" in completion ? (
        <TurnMarkdown text={completion.Reply} />
      ) : (
        completion.ToolCalls.map((call) => <ToolCallDetail key={call.id} call={call} />)
      )}
    </div>
  );
}

/// The completion's text for the estimate fallback when the provider reported no completion count.
function completionText(completion: Completion): string {
  if (completion === "Silent") return "";
  if ("Reply" in completion) return completion.Reply;
  return completion.ToolCalls.map((call) => call.arguments).join("");
}

/// A bar segment's tooltip: tokens for a measurement, a share of its block otherwise — the bar's
/// geometry is proportionate either way, but only a measured segment names a number.
function segmentSize(cost: RowCost, scale: number): string {
  if (cost.kind === "measured") {
    return `${cost.tokens.toLocaleString()} tokens (${((cost.tokens / scale) * 100).toFixed(1)}% of the window)`;
  }
  return `${cost.percent.toFixed(1)}% of the measured block it sits in`;
}

function rollupProvenance(rows: AttributedRow[], total: TokenProvenance): TokenProvenance {
  const each = rows.map((row) => rowProvenance(row.cost, total));
  if (each.every((provenance) => provenance === "measured")) return "measured";
  if (each.some((provenance) => provenance === "estimated")) return "estimated";
  return "share";
}

function badgeClass(provenance: TokenProvenance): string {
  switch (provenance) {
    case "measured":
      return "text-2xs text-sage";
    case "share":
      return "text-2xs text-ink-faint";
    case "estimated":
      return "text-2xs italic text-ink-faint";
  }
}

function badgeTitle(provenance: TokenProvenance): string {
  switch (provenance) {
    case "measured":
      return "Measured from the provider's reported token counts.";
    case "share":
      return "A share of the measured block it sits in, by character. Nothing measured this row on its own, so it states a ratio rather than a token count.";
    case "estimated":
      return "Estimated at four characters per token; the provider reported no usage here.";
  }
}

function sectionSwatch(kind: PromptSectionKind | undefined): string {
  // A row with no section (a tool set, the message history) carries no swatch colour of its own.
  if (kind === undefined) return "bg-line";
  switch (kind) {
    case "Scaffold":
      return "bg-ink";
    case "Identity":
      return "bg-sage";
    case "ApiReference":
      return "bg-line-strong";
    case "Vocabulary":
      return "bg-oat-deep";
    case "Brief":
      return "bg-sage-soft";
    case "CurrentTime":
      return "bg-clay-soft";
    default: {
      // Exhaustive over PromptSectionKind: a new section kind fails typecheck here until it is named.
      const unhandled: never = kind;
      return unhandled;
    }
  }
}
