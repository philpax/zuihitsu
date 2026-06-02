//! System-prompt assembly: the scaffold framing, the agent's identity drawn from `self` (seeded as
//! its description at genesis), and the declared current time are composed into one prompt.

#![cfg(feature = "sqlite")]

use zuihitsu::{
    Graph, ManualClock, MemoryStore, PromptTemplateName, SeedSelf, Timestamp, genesis,
    latest_template, render_api_reference, system_prompt,
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
    let prompt = system_prompt::assemble(&scaffold, &identity, &api, Timestamp::from_millis(1_000));

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
    // The declared session time, in human units (1_000 ms after the epoch).
    assert!(prompt.contains("01 January 1970"));
    assert!(prompt.contains("UTC"));
}
