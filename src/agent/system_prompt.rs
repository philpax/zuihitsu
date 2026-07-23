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
    prompt::{AssembledPrompt, PromptSectionKind},
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
) -> AssembledPrompt {
    let mut prompt = AssembledPrompt::default();
    prompt.push(PromptSectionKind::Scaffold, scaffold);

    if !identity.is_empty() {
        prompt.push(PromptSectionKind::Identity, "\n\n# Who you are\n\n");
        for (index, entry) in identity.iter().enumerate() {
            if index > 0 {
                prompt.push(PromptSectionKind::Identity, "\n\n");
            }
            prompt.push(PromptSectionKind::Identity, &entry.text);
        }
    }

    if !api_reference.is_empty() {
        prompt.push(PromptSectionKind::ApiReference, "\n\n# What you can do\n\n");
        prompt.push(PromptSectionKind::ApiReference, &method_notation_legend());
        prompt.push(PromptSectionKind::ApiReference, "\n\n");
        prompt.push(PromptSectionKind::ApiReference, api_reference);
    }

    if !vocabulary.is_empty() {
        prompt.push(PromptSectionKind::Vocabulary, "\n\n");
        prompt.push(PromptSectionKind::Vocabulary, vocabulary);
    }

    if !brief.is_empty() {
        prompt.push(
            PromptSectionKind::Brief,
            "\n\n# What you know right now\n\n",
        );
        prompt.push(PromptSectionKind::Brief, brief);
    }

    prompt.push(
        PromptSectionKind::CurrentTime,
        "\n\n# Current time\n\nThe session begins on ",
    );
    prompt.push(PromptSectionKind::CurrentTime, &time::format_datetime(now));
    prompt.push(PromptSectionKind::CurrentTime, ".");
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
/// Each line is `name / inverse — from-to[, symmetric][, reflexive]: description`, so the agent
/// knows which relations `links.create` accepts and what each is for (register new ones with
/// `links.register`).
pub fn render_relation_registry(relations: &[RelationView]) -> String {
    if relations.is_empty() {
        return String::new();
    }
    let mut out = String::from(
        "# Relations\n\nRelations you can link with links.create(subject, relation, object) (register new ones with links.register):",
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
            "\n- {} / {} — {}-to-{}{traits}: {}",
            relation.name.as_str(),
            relation.inverse.as_str(),
            relation.from_card.as_str().to_lowercase(),
            relation.to_card.as_str().to_lowercase(),
            relation.description,
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
    use super::{assemble, method_notation_legend, render_vocabulary};
    use crate::{
        InstanceFeatures,
        agent::{
            genesis::{self, SeedSelf},
            lua::render_api_reference,
            templates::latest_template,
        },
        clock::ManualClock,
        event::{PromptTemplateName, Teller, Visibility},
        graph::{EntryOrigin, EntryView, Graph},
        ids::EntryId,
        prompt::PromptSectionKind,
        store::MemoryStore,
        time::{self, Timestamp},
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
            None,
            &InstanceFeatures::default(),
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
        let api = render_api_reference(&InstanceFeatures::default());
        let vocabulary =
            render_vocabulary(&graph.all_tags().unwrap(), &graph.all_relations().unwrap());
        let brief = "<participant name=\"marcus\">a friend</participant>";
        let assembled = assemble(
            &scaffold,
            &identity,
            &api,
            &vocabulary,
            brief,
            Timestamp::from_millis(1_000),
        );
        let prompt = assembled.text();

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
        assert!(prompt.contains("<participant name=\"marcus\">a friend</participant>"));
        // The declared session time, in human units (1_000 ms after the epoch).
        assert!(prompt.contains("01 January 1970"));
        assert!(prompt.contains("UTC"));
    }

    /// A synthetic content entry carrying only the `text` that `assemble` reads; the remaining fields
    /// are inert defaults, so the fixture stays readable and free of personal names.
    fn entry(text: &str) -> EntryView {
        EntryView {
            entry_id: EntryId::generate(),
            asserted_at: Timestamp::from_millis(0),
            occurred_sort: None,
            occurred_at: None,
            occurred_authored: false,
            text: text.to_owned(),
            told_by: Teller::Agent,
            told_in: None,
            visibility: Visibility::default(),
            superseded_by: None,
            retracted_reason: None,
            origin: EntryOrigin::Recorded,
        }
    }

    const SCAFFOLD: &str = "SCAFFOLD BODY";
    const API_REFERENCE: &str = "API REFERENCE BODY";
    const VOCABULARY: &str = "# Tags\n\ntag stuff";
    const BRIEF: &str = "BRIEF BODY";

    fn fixed_now() -> Timestamp {
        Timestamp::from_millis(1_000)
    }

    /// context-debugger.AC7.2: with a full six-section fixture, `assemble`'s text is byte-for-byte the
    /// concatenation the composer has always produced — the leading separators, the section headers,
    /// the notation legend inside the API block, and the trailing time sentence, in order. The legend
    /// and the formatted datetime are interpolated from the same shared helpers the composer uses, so
    /// this pins the assembly structure the refactor could disturb rather than those helpers' bodies.
    #[test]
    fn assemble_output_is_stable() {
        let identity = [
            entry("I am the first entry"),
            entry("I am the second entry"),
        ];
        let assembled = assemble(
            SCAFFOLD,
            &identity,
            API_REFERENCE,
            VOCABULARY,
            BRIEF,
            fixed_now(),
        );

        let expected = format!(
            "SCAFFOLD BODY\
             \n\n# Who you are\n\nI am the first entry\n\nI am the second entry\
             \n\n# What you can do\n\n{legend}\n\nAPI REFERENCE BODY\
             \n\n# Tags\n\ntag stuff\
             \n\n# What you know right now\n\nBRIEF BODY\
             \n\n# Current time\n\nThe session begins on {datetime}.",
            legend = method_notation_legend(),
            datetime = time::format_datetime(fixed_now()),
        );
        assert_eq!(assembled.text(), expected);
    }

    /// context-debugger.AC1.2 (assembly half): the recorded spans name every present section in
    /// emission order, and slicing the text by each span yields that section's whole contribution —
    /// its leading separator, its header, and its body.
    #[test]
    fn sections_slice_to_their_contributions() {
        let identity = [
            entry("I am the first entry"),
            entry("I am the second entry"),
        ];
        let assembled = assemble(
            SCAFFOLD,
            &identity,
            API_REFERENCE,
            VOCABULARY,
            BRIEF,
            fixed_now(),
        );
        let text = assembled.text();
        let sections = assembled.sections();

        let kinds: Vec<_> = sections.iter().map(|span| span.kind).collect();
        assert_eq!(
            kinds,
            vec![
                PromptSectionKind::Scaffold,
                PromptSectionKind::Identity,
                PromptSectionKind::ApiReference,
                PromptSectionKind::Vocabulary,
                PromptSectionKind::Brief,
                PromptSectionKind::CurrentTime,
            ],
        );

        let slice = |index: usize| {
            let span = &sections[index];
            &text[span.start as usize..span.end as usize]
        };
        assert_eq!(slice(0), "SCAFFOLD BODY");
        assert_eq!(
            slice(1),
            "\n\n# Who you are\n\nI am the first entry\n\nI am the second entry",
        );
        assert_eq!(
            slice(2),
            format!(
                "\n\n# What you can do\n\n{}\n\nAPI REFERENCE BODY",
                method_notation_legend()
            ),
        );
        assert_eq!(slice(3), format!("\n\n{VOCABULARY}"));
        assert_eq!(slice(4), "\n\n# What you know right now\n\nBRIEF BODY");
        assert_eq!(
            slice(5),
            format!(
                "\n\n# Current time\n\nThe session begins on {}.",
                time::format_datetime(fixed_now())
            ),
        );

        // The spans tile the whole text: contiguous from zero to its length, with no gaps.
        let mut cursor = 0usize;
        for span in sections {
            assert_eq!(span.start as usize, cursor);
            cursor = span.end as usize;
        }
        assert_eq!(cursor, text.len());
    }

    /// The conditional sections drop out cleanly: with no identity, vocabulary, or brief, only the
    /// three unconditional kinds are recorded, and the remaining spans still tile the text.
    #[test]
    fn absent_sections_have_no_span_and_tiling_still_holds() {
        let assembled = assemble(SCAFFOLD, &[], API_REFERENCE, "", "", fixed_now());
        let text = assembled.text();
        let sections = assembled.sections();

        let kinds: Vec<_> = sections.iter().map(|span| span.kind).collect();
        assert_eq!(
            kinds,
            vec![
                PromptSectionKind::Scaffold,
                PromptSectionKind::ApiReference,
                PromptSectionKind::CurrentTime,
            ],
        );

        let mut cursor = 0usize;
        for span in sections {
            assert_eq!(span.start as usize, cursor);
            cursor = span.end as usize;
        }
        assert_eq!(cursor, text.len());
    }
}
