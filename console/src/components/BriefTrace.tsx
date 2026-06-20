import {
  type BriefSectionTrace,
  type EntryTrace,
  decisionInfo,
  sectionLabel,
} from "../lib/brief.ts";
import { Eyebrow } from "./primitives.tsx";

/// Renders a brief composition trace's sections: each memory the composer considered and, per entry,
/// whether it reached the brief and why — surfaced (sage), passed the predicate but trimmed by recency
/// (faint), or filtered by a visibility verdict (clay, with the reason). The frozen brief text the
/// agent saw is shown by the caller alongside, so this renders the sections alone.
export function BriefSections({ sections }: { sections: BriefSectionTrace[] }) {
  return (
    <div className="mt-4 flex flex-col gap-5">
      {sections.map((section, index) => (
        <Section key={index} section={section} />
      ))}
    </div>
  );
}

function Section({ section }: { section: BriefSectionTrace }) {
  return (
    <div>
      <div className="flex items-baseline gap-2">
        <Eyebrow>{sectionLabel(section.kind)}</Eyebrow>
        <span className="font-mono text-2xs text-ink-soft">{section.memory}</span>
        {section.confidential && <span className="font-mono text-2xs text-clay">confidential</span>}
      </div>
      {section.entries.length === 0 ? (
        <p className="mt-1.5 font-mono text-2xs text-ink-faint">no entries</p>
      ) : (
        <ul className="mt-2 flex flex-col gap-2">
          {section.entries.map((entry, index) => (
            <EntryRow key={index} entry={entry} />
          ))}
        </ul>
      )}
    </div>
  );
}

function EntryRow({ entry }: { entry: EntryTrace }) {
  const { visible, reason } = decisionInfo(entry.decision);
  // Three fates: surfaced, visible-but-trimmed by the recency window, or filtered by the predicate.
  const tone = entry.in_brief ? "in" : visible ? "trimmed" : "filtered";
  const note =
    tone === "in" ? reason : tone === "trimmed" ? `${reason} · beyond recency window` : reason;

  return (
    <li className="flex gap-2.5">
      <span
        className={
          "mt-0.5 shrink-0 font-mono text-2xs " +
          (tone === "in" ? "text-sage" : tone === "filtered" ? "text-clay" : "text-ink-faint")
        }
      >
        {tone === "in" ? "▸" : tone === "filtered" ? "✕" : "·"}
      </span>
      <div>
        <p
          className={
            "text-sm leading-relaxed " +
            (tone === "in" ? "text-ink" : tone === "filtered" ? "text-ink-faint" : "text-ink-soft")
          }
        >
          {entry.text}
        </p>
        <p
          className={"font-mono text-2xs " + (tone === "filtered" ? "text-clay" : "text-ink-faint")}
        >
          {note}
        </p>
      </div>
    </li>
  );
}
