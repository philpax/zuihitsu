//! The inference prompt renderer — assembles the memory, its numbered statements, existing links,
//! registered relations, and candidate targets into the model's prompt.

use crate::{
    graph::{EntryView, MemoryView, RelationView},
    time::{self, Timestamp},
};

use super::{CANDIDATE_CAP, relations::ExistingLink};

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
    let mut prompt = format!(
        "Memory: {}\nCurrent time: {}\n\nStatements:\n",
        memory.name.as_str(),
        time::format_datetime(now),
    );
    for (index, entry) in entries.iter().enumerate() {
        prompt.push_str(&format!("{}. {}\n", index + 1, entry.text));
    }
    prompt.push_str("\nExisting links:\n");
    if existing_links.is_empty() {
        prompt.push_str("  (none)\n");
    } else {
        for link in existing_links {
            prompt.push_str(&format!(
                "- {} —{}→ {}\n",
                link.from.as_str(),
                link.relation.as_str(),
                link.to.as_str()
            ));
        }
    }
    prompt.push_str("\nRegistered relations:\n");
    if relations.is_empty() {
        prompt.push_str("  (none)\n");
    } else {
        for relation in relations {
            prompt.push_str(&format!(
                "- {name}/{inverse} — a link \"A {name} B\" restates as \"B {inverse} A\" \
                 (from: {from}, to: {to}, symmetric: {symmetric}, reflexive: {reflexive}): {desc}\n",
                name = relation.name.as_str(),
                inverse = relation.inverse.as_str(),
                from = relation.from_card.as_str(),
                to = relation.to_card.as_str(),
                symmetric = relation.symmetric,
                reflexive = relation.reflexive,
                desc = relation.description,
            ));
        }
    }
    prompt.push_str("\nCandidate memories (resolve relationships to these handles):\n");
    for candidate in candidates.iter().take(CANDIDATE_CAP) {
        prompt.push_str(&format!(
            "- {} — {}\n",
            candidate.name.as_str(),
            candidate.description
        ));
    }
    prompt.push_str("\nIdentify relationships in the statements that link this memory to one of the candidates.\n");
    prompt
}
