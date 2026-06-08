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

use crate::{
    graph::EntryView,
    time::{self, Timestamp},
};

/// Compose the system prompt from the `scaffold` body, the agent's `identity` (the `self` memory's
/// content entries, verbatim), the `api_reference` block (the build's callable Lua API, rendered by
/// [`crate::agent::lua::render_api_reference`]), the session's frozen contextual `brief` (composed by
/// [`crate::memory::brief::compose`] and captured on `SessionStarted`), and the session's start time `now`.
pub fn assemble(
    scaffold: &str,
    identity: &[EntryView],
    api_reference: &str,
    brief: &str,
    now: Timestamp,
) -> String {
    let mut prompt =
        String::with_capacity(scaffold.len() + api_reference.len() + brief.len() + 256);
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
        prompt.push_str(api_reference);
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

// Gated on `lua` (not just `sqlite`) because the assertion exercises `render_api_reference`, which
// is part of the Lua API surface.
#[cfg(all(test, feature = "lua"))]
mod tests {
    //! The scaffold framing, the agent's identity drawn from `self` (seeded as its description at
    //! genesis), and the declared current time are composed into one prompt.
    use super::assemble;
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
        let self_memory = graph.memory_by_name("self").unwrap().unwrap();
        let identity = graph.entries_local(self_memory.id).unwrap();
        let api = render_api_reference();
        let brief = "<participant name=\"phil\">a friend</participant>";
        let prompt = assemble(
            &scaffold,
            &identity,
            &api,
            brief,
            Timestamp::from_millis(1_000),
        );

        // The durable scaffold framing.
        assert!(prompt.contains("run_lua"));
        // The persona, drawn verbatim from self's seed entry.
        assert!(prompt.contains("A discreet companion with a long memory."));
        // The build-derived API description, interpolated from the same typed source the implementation
        // uses: the call signature, a parameter's type, and the return type.
        assert!(prompt.contains("mem:append(text, opts?)"));
        assert!(prompt.contains("text: string (required)"));
        assert!(prompt.contains("opts.visibility: \"public\" | \"private\""));
        assert!(prompt.contains("context.current()"));
        // The session's frozen contextual brief.
        assert!(prompt.contains("<participant name=\"phil\">a friend</participant>"));
        // The declared session time, in human units (1_000 ms after the epoch).
        assert!(prompt.contains("01 January 1970"));
        assert!(prompt.contains("UTC"));
    }
}
