//! The inference prompt renderer — assembles the memory, its numbered statements, existing links,
//! registered relations, and candidate targets into the model's prompt. The constant framing lives
//! in `prompt.md`; the per-item lines are runtime data, generated here and substituted into the
//! frame's markers.

use crate::{
    agent::{
        body_of, render_placeholders,
        turn::link_inference::{CANDIDATE_CAP, relations::ExistingLink},
    },
    graph::{EntryView, MemoryView, RelationView},
    time::{self, Timestamp},
};

/// Render the inference prompt: the memory and its numbered statements, its existing links, the
/// registered relations, and the candidate target memories by handle and description.
pub(super) fn render_prompt(
    memory: &MemoryView,
    entries: &[EntryView],
    existing_links: &[ExistingLink],
    relations: &[RelationView],
    candidates: &[MemoryView],
    now: Timestamp,
) -> String {
    // The frame's `{{statements}}`/`{{links}}`/`{{relations}}`/`{{candidates}}` markers hold the
    // generated per-item lines. Each block keeps its trailing newline (so the frame's own newline
    // after the marker supplies the blank line the original pushed before the next header), and the
    // final instruction paragraph gains its own trailing newline after substitution, exactly as the
    // pre-migration output did.
    let mut statements = String::new();
    for (index, entry) in entries.iter().enumerate() {
        statements.push_str(&format!("{}. {}\n", index + 1, entry.text));
    }

    let mut links = String::new();
    if existing_links.is_empty() {
        links.push_str("  (none)\n");
    } else {
        for link in existing_links {
            links.push_str(&format!(
                "- {} —{}→ {}\n",
                link.from.as_str(),
                link.relation.as_str(),
                link.to.as_str()
            ));
        }
    }

    let mut relations_lines = String::new();
    if relations.is_empty() {
        relations_lines.push_str("  (none)\n");
    } else {
        for relation in relations {
            relations_lines.push_str(&render_placeholders(
                body_of(include_str!("relations_line.md")),
                &[
                    ("name", relation.name.as_str()),
                    ("inverse", relation.inverse.as_str()),
                    ("from", relation.from_card.as_str()),
                    ("to", relation.to_card.as_str()),
                    ("symmetric", &relation.symmetric.to_string()),
                    ("reflexive", &relation.reflexive.to_string()),
                    ("desc", &relation.description),
                ],
            ));
            relations_lines.push('\n');
        }
    }

    let mut candidates_lines = String::new();
    for candidate in candidates.iter().take(CANDIDATE_CAP) {
        candidates_lines.push_str(&format!(
            "- {} — {}\n",
            candidate.name.as_str(),
            candidate.description
        ));
    }

    let mut prompt = render_placeholders(
        body_of(include_str!("prompt.md")),
        &[
            ("memory", memory.name.as_str()),
            ("now", &time::format_datetime(now)),
            ("statements", &statements),
            ("links", &links),
            ("relations", &relations_lines),
            ("candidates", &candidates_lines),
        ],
    );
    prompt.push('\n');
    prompt
}
