//! System-prompt assembly (spec §System prompt).
//!
//! The frozen system prompt is assembled from the **scaffold** template (the durable, operational
//! framing — how the agent acts, never who it is), the agent's **identity** drawn from `self`, the
//! build-derived **API description** (rendered from the running binary, so the prompt and the
//! implementation cannot drift — see [`crate::agent::lua::render_api_reference`]), and the declared
//! **current time**. The remaining spec source — the per-session **contextual brief** — arrives
//! with the conversation/brief machinery; this composer leaves room for it rather than restating it.
//!
//! Identity is drawn from `self`'s content **entries**, verbatim — not its description. Entries are
//! immutable and append-only, so the authored persona never drifts, while the self still evolves as
//! the agent appends further self-observations; the regenerable description is a lossy summary and
//! is deliberately not the source of the agent's voice.
//!
//! Assembly is a pure function of already-fetched inputs, so the caller owns the store/graph/clock
//! reads (and their error handling) and this stays trivially testable.

use std::fmt::Write as _;

use crate::{
    graph::{EntryView, RelationView, TagVocabularyEntry},
    ids::Namespace,
    time::{self, Timestamp},
};

/// Explains the `<memory>:method(...)` notation used throughout the API description, so the agent
/// calls a handle method as `handle:method(...)` rather than pasting the placeholder literally (the
/// model otherwise tends to write `dave:<memory>:append(...)`, conflating the placeholder with its
/// handle). Prepended to the API description.
fn method_notation_legend() -> String {
    let person = Namespace::Person.prefix();
    format!(
        "Methods are written `<memory>:method(...)`, where `<memory>` \
         stands for a memory handle you hold — the result of `memory.create` or `memory.get`. Call the \
         method directly on that handle: e.g. with `local m = memory.get(\"{person}...\")`, write \
         `m:append(\"...\")` (not `m:<memory>:append(...)`). The `memory.*`, `tags.*`, `links.*`, \
         `calendar.*`, `context.*`, and `block.*` calls are module functions, called with a dot."
    )
}

/// Compose the system prompt from the `scaffold` body, the agent's `identity` (the `self` memory's
/// content entries, verbatim), the `api_reference` block (the build's callable Lua API, rendered by
/// [`crate::agent::lua::render_api_reference`]), the runtime `vocabulary` block (the current tag
/// vocabulary and registered relations — runtime data, so composed by the caller from graph reads;
/// see [`render_tag_vocabulary`]), the session's frozen contextual `brief` (composed by
/// [`crate::memory::brief::compose`] and captured on `SessionStarted`), and the session's start time
/// `now`. The vocabulary sits with the API description: both tell the agent what it can call and with
/// what labels (spec §The API description is injected into the system prompt).
pub fn assemble(
    scaffold: &str,
    identity: &[EntryView],
    api_reference: &str,
    vocabulary: &str,
    brief: &str,
    now: Timestamp,
) -> String {
    let mut prompt = String::with_capacity(
        scaffold.len() + api_reference.len() + vocabulary.len() + brief.len() + 256,
    );
    prompt.push_str(scaffold);

    if !identity.is_empty() {
        prompt.push_str("\n\n# Who you are\n\n");
        for (index, entry) in identity.iter().enumerate() {
            if index > 0 {
                prompt.push_str("\n\n");
            }
            prompt.push_str(&entry.text);
        }
    }

    if !api_reference.is_empty() {
        prompt.push_str("\n\n# What you can do\n\n");
        prompt.push_str(&method_notation_legend());
        prompt.push_str("\n\n");
        prompt.push_str(api_reference);
    }

    if !vocabulary.is_empty() {
        prompt.push_str("\n\n");
        prompt.push_str(vocabulary);
    }

    if !brief.is_empty() {
        prompt.push_str("\n\n# What you know right now\n\n");
        prompt.push_str(brief);
    }

    prompt.push_str("\n\n# Current time\n\nThe session begins on ");
    prompt.push_str(&time::format_datetime(now));
    prompt.push('.');
    prompt
}

/// The runtime vocabulary block for [`assemble`]: the current tag vocabulary and the registered link
/// relations, each rendered as its own section and joined, or the empty string when both are empty.
/// The caller reads both from the graph (they are runtime data, not build-derived) and hands the
/// result to `assemble` to sit beside the API description.
pub fn render_vocabulary(tags: &[TagVocabularyEntry], relations: &[RelationView]) -> String {
    [
        render_tag_vocabulary(tags),
        render_relation_registry(relations),
    ]
    .into_iter()
    .filter(|section| !section.is_empty())
    .collect::<Vec<_>>()
    .join("\n\n")
}

/// Render the current tag vocabulary as a prompt section, or the empty string when no tags exist. Each
/// line is `name — purpose (N uses)`, so the agent knows which tags it may apply with `mem:tag` (and
/// can see which are already in use).
pub fn render_tag_vocabulary(tags: &[TagVocabularyEntry]) -> String {
    if tags.is_empty() {
        return String::new();
    }
    let mut out = String::from(
        "# Tags\n\nTags you can apply with <memory>:tag (create new ones with tags.create):",
    );
    for tag in tags {
        let uses = if tag.count == 1 {
            "1 use".to_owned()
        } else {
            format!("{} uses", tag.count)
        };
        let _ = write!(
            out,
            "\n- {} — {} ({uses})",
            tag.name.as_str(),
            tag.description
        );
    }
    out
}

/// Render the registered link relations as a prompt section, or the empty string when none exist.
/// Each line is `name / inverse — from-to[, symmetric][, reflexive]`, so the agent knows which
/// relations `mem:link` accepts (register new ones with `links.register`).
pub fn render_relation_registry(relations: &[RelationView]) -> String {
    if relations.is_empty() {
        return String::new();
    }
    let mut out = String::from(
        "# Relations\n\nRelations you can link with <memory>:link (register new ones with links.register):",
    );
    for relation in relations {
        let mut traits = String::new();
        if relation.symmetric {
            traits.push_str(", symmetric");
        }
        if relation.reflexive {
            traits.push_str(", reflexive");
        }
        let _ = write!(
            out,
            "\n- {} / {} — {}-to-{}{traits}",
            relation.name.as_str(),
            relation.inverse.as_str(),
            relation.from_card.as_str().to_lowercase(),
            relation.to_card.as_str().to_lowercase(),
        );
    }
    out
}

// Gated on `lua` (not just `sqlite`) because the assertion exercises `render_api_reference`, which
// is part of the Lua API surface.
#[cfg(test)]
mod tests {
    //! The scaffold framing, the agent's identity drawn from `self` (seeded as its description at
    //! genesis), and the declared current time are composed into one prompt.
    use super::{assemble, render_vocabulary};
    use crate::{
        agent::{
            genesis::{self, SeedSelf},
            lua::render_api_reference,
            templates::latest_template,
        },
        clock::ManualClock,
        event::PromptTemplateName,
        graph::Graph,
        store::MemoryStore,
        time::Timestamp,
    };

    #[test]
    fn assembles_scaffold_identity_and_time() {
        let mut store = MemoryStore::new();
        let seed = SeedSelf {
            agent_name: "Kestrel".to_owned(),
            persona: "A discreet companion with a long memory.".to_owned(),
            seed_entries: Vec::new(),
        };
        genesis::rollout(
            &mut store,
            &ManualClock::new(Timestamp::from_millis(1_000)),
            &seed,
        )
        .unwrap();
        let mut graph = Graph::open_in_memory().unwrap();
        graph.materialize_from(&store).unwrap();

        let scaffold = latest_template(&store, PromptTemplateName::Scaffold)
            .unwrap()
            .unwrap()
            .body;
        let self_memory = graph.self_memory().unwrap().unwrap();
        let identity = graph.entries_local(self_memory.id).unwrap();
        let api = render_api_reference();
        let vocabulary =
            render_vocabulary(&graph.all_tags().unwrap(), &graph.all_relations().unwrap());
        let brief = "<participant name=\"phil\">a friend</participant>";
        let prompt = assemble(
            &scaffold,
            &identity,
            &api,
            &vocabulary,
            brief,
            Timestamp::from_millis(1_000),
        );

        // The durable scaffold framing.
        assert!(prompt.contains("run_lua"));
        // The persona, drawn verbatim from self's seed entry.
        assert!(prompt.contains("A discreet companion with a long memory."));
        // The build-derived API description, interpolated from the same typed source the implementation
        // uses: the call signature, a parameter's type, and the return type.
        assert!(prompt.contains("<memory>:append(text, opts?)"));
        // The notation legend that disambiguates a handle method from the placeholder.
        assert!(prompt.contains("stands for a memory handle you hold"));
        assert!(prompt.contains("text: string (required)"));
        assert!(prompt.contains("opts.visibility: \"public\" | \"private\""));
        assert!(prompt.contains("context.current()"));
        // The current tag vocabulary, drawn from the graph: the genesis-seeded `confidential` tag.
        assert!(prompt.contains("# Tags"));
        assert!(prompt.contains("confidential — Marks a context as confidential"));
        // The registered relation registry, drawn from the graph: a genesis-seeded relation and its
        // inverse label.
        assert!(prompt.contains("# Relations"));
        assert!(prompt.contains("same_as"));
        // The session's frozen contextual brief.
        assert!(prompt.contains("<participant name=\"phil\">a friend</participant>"));
        // The declared session time, in human units (1_000 ms after the epoch).
        assert!(prompt.contains("01 January 1970"));
        assert!(prompt.contains("UTC"));
    }
}
