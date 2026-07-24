//! The operator Lua console control action ([`crate::instance::Control::run_lua`]): an ad-hoc,
//! no-commit sandbox. Exercised over the in-memory backends. The load-bearing property is that a run
//! leaves nothing in the log: it neither mints a conversation (unlike a live turn) nor records a
//! `LuaExecuted`, so an operator's experiment never shifts the agent's history, and `context.current`
//! is nil since the sandbox has no room.
use crate::{Instance, SeedSelf, clock::ManualClock, time::Timestamp};

/// A born instance whose `self` carries only its seeded persona entry — the state a console run reads.
fn born_server() -> Instance {
    let server =
        Instance::in_memory(Box::new(ManualClock::new(Timestamp::from_millis(0)))).unwrap();
    server
        .control()
        .create_agent(&SeedSelf {
            agent_name: "Kestrel".to_owned(),
            persona: "A discreet companion with a long memory.".to_owned(),
            seed_entries: vec![],
        })
        .unwrap();
    server
}

#[tokio::test]
async fn a_console_run_reads_state_without_touching_the_log() {
    let server = born_server();
    let before = server.control().events().unwrap().len();

    // A read against `self` runs the block VM with no conversation (the sandbox has none).
    let outcome = server
        .control()
        .run_lua("return memory.get(\"self\"):entries()", false, false)
        .await
        .unwrap();
    assert!(
        outcome.error.is_none(),
        "the run should succeed with no conversation: {:?}",
        outcome.error
    );
    assert!(outcome.result.is_some(), "the read should render a result");

    // No `console/lua` conversation is minted and no `LuaExecuted` is recorded, so the log is untouched
    // — the run is invisible to it, as the sandbox promises.
    assert_eq!(
        server.control().events().unwrap().len(),
        before,
        "run_lua must append no events"
    );
}

#[tokio::test]
async fn the_console_sandbox_has_no_current_context() {
    let server = born_server();
    let before = server.control().events().unwrap().len();
    // With no conversation, reading `context.current` resolves to nil rather than blowing up or
    // lazily minting a room.
    let outcome = server
        .control()
        .run_lua("return context.current", false, false)
        .await
        .unwrap();
    assert!(
        outcome.error.is_none(),
        "context.current should be nil, not an error: {:?}",
        outcome.error
    );
    assert_eq!(
        server.control().events().unwrap().len(),
        before,
        "reading context.current must not mint anything"
    );
}
