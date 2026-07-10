import type { PromptSectionKind } from "../../types/PromptSectionKind.ts";
import type { PromptSectionSpan } from "../../types/PromptSectionSpan.ts";

/// Where a section boundary came from: recorded spans on the `ModelCalled` event (exact), or the
/// header-parsing fallback for records written before capture (inferred).
export type SectionProvenance = "recorded" | "inferred";

/// One resolved section of a system prompt, in JS string (UTF-16 code unit) offsets so callers can
/// slice the string directly.
export interface ResolvedSection {
  kind: PromptSectionKind;
  start: number;
  end: number;
  provenance: SectionProvenance;
}

/// The markdown headers `assemble()` emits, in emission order. The vocabulary block carries no
/// header of its own (its `# Tags`/`# Relations` headings are baked into the caller-supplied
/// string), so it is recognized as the residue between the api-reference block and the brief.
const HEADERS: Array<{ kind: PromptSectionKind; header: string }> = [
  { kind: "Identity", header: "\n\n# Who you are\n\n" },
  { kind: "ApiReference", header: "\n\n# What you can do\n\n" },
  { kind: "Brief", header: "\n\n# What you know right now\n\n" },
  { kind: "CurrentTime", header: "\n\n# Current time\n\n" },
];

/// The headers the vocabulary residue may start with, distinguishing it from a trailing stretch of
/// the api-reference block.
const VOCABULARY_STARTS = ["\n\n# Tags\n", "\n\n# Relations\n"];

/// Resolve a system prompt's sections: recorded spans pass through exactly (converted from byte
/// offsets to JS string indices), and an empty span list falls back to parsing the known headers,
/// with every resulting section marked inferred. Both paths tile the string — the heuristic folds
/// unrecognized residue into the preceding section rather than dropping characters.
export function resolveSections(system: string, recorded: PromptSectionSpan[]): ResolvedSection[] {
  if (system.length === 0) return [];
  if (recorded.length > 0) return convertRecorded(system, recorded);
  return inferSections(system);
}

/// Recorded spans are byte offsets into the UTF-8 encoding; JS strings index UTF-16 code units. A
/// single walk over the string accumulates the byte position per code point (by arithmetic on the
/// code point — encoding each character would allocate per char) and maps each span boundary to its
/// string index.
function convertRecorded(system: string, recorded: PromptSectionSpan[]): ResolvedSection[] {
  const boundaries = new Set<number>();
  for (const span of recorded) {
    boundaries.add(span.start);
    boundaries.add(span.end);
  }
  const byteToIndex = new Map<number, number>();
  let byte = 0;
  let index = 0;
  byteToIndex.set(0, 0);
  for (const char of system) {
    const point = char.codePointAt(0) ?? 0;
    byte += point < 0x80 ? 1 : point < 0x800 ? 2 : point < 0x10000 ? 3 : 4;
    index += char.length;
    if (boundaries.has(byte)) byteToIndex.set(byte, index);
  }
  return recorded.map((span) => ({
    kind: span.kind,
    // A boundary that falls off the map (a malformed span) clamps to the end rather than lying.
    start: byteToIndex.get(span.start) ?? system.length,
    end: byteToIndex.get(span.end) ?? system.length,
    provenance: "recorded",
  }));
}

/// The header-parsing fallback for pre-capture records: scaffold is everything before the first
/// recognized header, each header opens its section, and the headerless vocabulary block is carved
/// out of the api-reference block when its residue starts with a vocabulary heading.
function inferSections(system: string): ResolvedSection[] {
  const cuts: Array<{ kind: PromptSectionKind; start: number }> = [{ kind: "Scaffold", start: 0 }];
  let from = 0;
  for (const { kind, header } of HEADERS) {
    const at = system.indexOf(header, from);
    if (at === -1) continue;
    cuts.push({ kind, start: at });
    from = at + header.length;
  }

  // Carve the vocabulary residue out of the api-reference block: it is the suffix of that block
  // starting at the first vocabulary heading.
  const api = cuts.find((cut) => cut.kind === "ApiReference");
  if (api) {
    const apiEnd = cuts.find((cut) => cut.start > api.start)?.start ?? system.length;
    const candidates = VOCABULARY_STARTS.map((header) => system.indexOf(header, api.start)).filter(
      (at) => at !== -1 && at < apiEnd,
    );
    if (candidates.length > 0) {
      cuts.push({ kind: "Vocabulary", start: Math.min(...candidates) });
    }
  }

  cuts.sort((a, b) => a.start - b.start);
  const sections: ResolvedSection[] = [];
  for (let i = 0; i < cuts.length; i += 1) {
    const end = i + 1 < cuts.length ? cuts[i + 1].start : system.length;
    if (end > cuts[i].start) {
      sections.push({ kind: cuts[i].kind, start: cuts[i].start, end, provenance: "inferred" });
    }
  }
  return sections;
}
