//! Shared integration-test helpers. Included via `mod common;` from a test file; the directory
//! form keeps it from being compiled as its own test binary.

// Helpers (and the `Harness` re-export) are used by some test binaries and not others; that's
// expected for a shared module.
#![allow(dead_code, unused_imports)]

pub mod time;

pub use harness::Harness;

use zuihitsu::{MemoryName, Namespace};

/// Resolve the namespace-token placeholders in a test Lua script to real handles. A script names a
/// standard persona or thing by token — `PERSON_DAVE`, `EVENT_DENTIST` — and this swaps each for the
/// quoted handle (`"person/dave"`), sourcing the prefix from [`Namespace`] so no test script spells a
/// prefix and a renamed prefix is a single edit. A literal handle left in a script has no token to
/// match, so it passes through untouched and conversion can proceed file by file.
pub fn prepare_script(script: &str) -> String {
    // Longest token first, so `PERSON_DAVE_CHAT` is consumed before `PERSON_DAVE` can corrupt it.
    let mut entries: Vec<(&str, MemoryName)> = STANDARD_HANDLES
        .iter()
        .map(|(token, namespace, subject)| {
            (*token, MemoryName::from(namespace.with_name(*subject)))
        })
        .collect();
    entries.sort_by_key(|(token, _)| std::cmp::Reverse(token.len()));
    let mut out = script.to_owned();
    for (token, handle) in entries {
        out = out.replace(token, &format!("\"{}\"", handle.as_str()));
    }
    out
}

/// The standard handles the test corpus draws from — the single registry, as `(token, namespace,
/// subject)`. The token is spelled out (not derived) so a script's `PERSON_DAVE` is greppable straight
/// to its line here; the prefix is `Namespace`'s, never written out, so renaming a prefix is one edit.
/// Add a persona or thing with one line and reference it by token in any script.
const STANDARD_HANDLES: &[(&str, Namespace, &str)] = &[
    ("PERSON_A", Namespace::Person, "a"),
    ("PERSON_ALPHA", Namespace::Person, "alpha"),
    ("PERSON_B_AT_CHAT", Namespace::Person, "b@chat"),
    ("PERSON_BETA", Namespace::Person, "beta"),
    ("PERSON_DAVE", Namespace::Person, "dave"),
    ("PERSON_DAVE_AT_CHAT", Namespace::Person, "dave@chat"),
    ("PERSON_DAVE_AT_FORUM", Namespace::Person, "dave@forum"),
    ("PERSON_DAVE_CHAT", Namespace::Person, "dave-chat"),
    ("PERSON_DAVE_FORUM", Namespace::Person, "dave-forum"),
    ("PERSON_ERIN", Namespace::Person, "erin"),
    ("PERSON_FRANK", Namespace::Person, "frank"),
    ("PERSON_MARCUS", Namespace::Person, "marcus"),
    ("PERSON_NOBODY", Namespace::Person, "nobody"),
    ("PERSON_OPERATOR", Namespace::Person, "operator"),
    ("PERSON_SAM_CHAT", Namespace::Person, "sam-chat"),
    ("PERSON_SAM_FORUM", Namespace::Person, "sam-forum"),
    ("PERSON_SARAH", Namespace::Person, "sarah"),
    ("PLACE_SYDNEY", Namespace::Place, "sydney"),
    ("EVENT_ALL_HANDS", Namespace::Event, "all-hands"),
    ("EVENT_BOARD_UPDATE", Namespace::Event, "board-update"),
    ("EVENT_CLEANING", Namespace::Event, "cleaning"),
    ("EVENT_LAUNCH", Namespace::Event, "launch"),
    ("EVENT_PRODUCT_LAUNCH", Namespace::Event, "product_launch"),
    ("EVENT_STANDUP", Namespace::Event, "standup"),
    ("TOPIC_A", Namespace::Topic, "a"),
    ("TOPIC_ALPHA", Namespace::Topic, "alpha"),
    ("TOPIC_BETA", Namespace::Topic, "beta"),
    ("TOPIC_CLIMBING", Namespace::Topic, "climbing"),
    ("TOPIC_GHOST", Namespace::Topic, "ghost"),
    ("TOPIC_LOCKED", Namespace::Topic, "locked"),
    ("TOPIC_MIGRATION", Namespace::Topic, "migration"),
    ("TOPIC_OOPS", Namespace::Topic, "oops"),
    ("TOPIC_PAGE", Namespace::Topic, "page"),
    ("TOPIC_PLAN", Namespace::Topic, "plan"),
    ("TOPIC_Q3_PLAN", Namespace::Topic, "q3_plan"),
    ("TOPIC_ROADMAP", Namespace::Topic, "roadmap"),
    ("TOPIC_SENSITIVE", Namespace::Topic, "sensitive"),
    ("TOPIC_SHARED", Namespace::Topic, "shared"),
    ("TOPIC_SOURDOUGH", Namespace::Topic, "sourdough"),
    ("CONTEXT_CHAT_LEADS", Namespace::Context, "chat:leads"),
];

mod harness {
    use std::{cell::Cell, sync::Arc, time::Duration};

    use zuihitsu::{
        AmbientSettings, Authority, BlockContext, BlockOutcome, CaptureLevel, ConversationId,
        Embedder, Engine, Event, EventPayload, EventSource, FakeEmbedder, Graph,
        InMemoryVectorIndex, InboundMessage, Initiation, InstanceFeatures, ManualClock, MemoryId,
        MemoryStore, ModelClient, PromptTemplateName, Seq, Session, Teller, Turn, TurnId,
        TurnRecord, TurnRole, TurnView, VectorIndex, append_turn,
        model::index::{apply_batch, embed_batch},
        run_adjudicate_catch_up, run_describe_catch_up, run_link_inference_catch_up,
    };

    use super::time::TEST_NOW;

    /// A block-duration budget generous enough that no in-memory test block ever trips it; the
    /// timeout's firing path is exercised directly in the MCP tests with a deliberately slow server.
    const TEST_BLOCK_TIMEOUT: Duration = Duration::from_secs(30);
    /// The per-block lock-wait retry bound for tests.
    const TEST_MAX_BLOCK_ATTEMPTS: u32 = 3;
    /// The memory entry character limit for tests — generous enough that existing test content
    /// passes, while still exercising the limit in the dedicated oversized-content tests.
    const TEST_MAX_ENTRY_CHARS: usize = 10_000;

    /// A complete agent backed entirely in memory: an in-memory event log, an in-memory graph, a
    /// manual clock, and one Lua session. The `engine` is the same shared handle the turn writes
    /// through, so a `run` and a subsequent `h.engine.graph.lock()` read observe each other. The
    /// `clock` field is a separate handle sharing the engine clock's atomic, for tests to read. Each
    /// `run` executes a block as its own turn.
    pub struct Harness {
        pub engine: Arc<Engine>,
        pub clock: ManualClock,
        pub session: Session,
        /// The stand-in inbound participant a turn is attributed to.
        pub participant: MemoryId,
        /// The inbound batch and its turn ids, stored on the harness so the `Turn` returned by
        /// `as_turn` can borrow them. Each `as_turn` call replaces these.
        inbound_batch: Vec<InboundMessage>,
        participant_turn_ids: Vec<TurnId>,
        /// The describer's per-memory serialization guard, mirroring the server's. The describer keeps
        /// no cursor — its backlog is the graph's log-derived stale set — so [`Harness::describe`]
        /// catches every stale memory up, and [`Harness::baseline_descriptions`] marks the current
        /// stale set described so a later catch-up never reconsiders the seeded `self`.
        describe_guard: tokio::sync::Mutex<()>,
        adjudicator_cursor: Cell<Seq>,
        link_inference_cursor: Cell<Seq>,
    }

    impl Default for Harness {
        fn default() -> Self {
            let clock = ManualClock::new(TEST_NOW);
            Harness {
                engine: Engine::new(
                    Box::new(MemoryStore::new()),
                    Graph::open_in_memory().unwrap(),
                    Box::new(clock.clone()),
                ),
                clock,
                session: Session::new(ConversationId::generate(), InstanceFeatures::default()),
                participant: MemoryId::generate(),
                inbound_batch: Vec::new(),
                participant_turn_ids: Vec::new(),
                describe_guard: tokio::sync::Mutex::new(()),
                adjudicator_cursor: Cell::new(Seq::ZERO),
                link_inference_cursor: Cell::new(Seq::ZERO),
            }
        }
    }

    /// The embedding dimensionality the retrieval-backed harness uses (the fake embedder's size).
    const TEST_EMBED_DIMS: usize = 16;

    impl Harness {
        pub fn new() -> Harness {
            Harness::default()
        }

        /// A harness whose engine has semantic retrieval attached (a fake embedder and in-memory
        /// vector index), for exercising `memory.search`. Drive [`Harness::index`] after a write to
        /// embed it before searching.
        pub fn with_retrieval() -> Harness {
            let clock = ManualClock::new(TEST_NOW);
            let embedder: Arc<dyn Embedder> = Arc::new(FakeEmbedder::new(TEST_EMBED_DIMS));
            let vectors: Box<dyn VectorIndex> = Box::new(InMemoryVectorIndex::new());
            Harness {
                engine: Engine::with_retrieval(
                    Box::new(MemoryStore::new()),
                    Graph::open_in_memory().unwrap(),
                    Box::new(clock.clone()),
                    embedder,
                    vectors,
                ),
                clock,
                session: Session::new(ConversationId::generate(), InstanceFeatures::default()),
                participant: MemoryId::generate(),
                inbound_batch: Vec::new(),
                participant_turn_ids: Vec::new(),
                describe_guard: tokio::sync::Mutex::new(()),
                adjudicator_cursor: Cell::new(Seq::ZERO),
                link_inference_cursor: Cell::new(Seq::ZERO),
            }
        }

        /// As [`Harness::default`], but with a narrowed API feature set — for tests that exercise a
        /// behaviour in isolation (e.g. disabling `linking` to verify the agent cannot call
        /// `links.create` while the link-inference pass still creates the link).
        pub fn with_features(features: InstanceFeatures) -> Harness {
            let clock = ManualClock::new(TEST_NOW);
            Harness {
                engine: Engine::new(
                    Box::new(MemoryStore::new()),
                    Graph::open_in_memory().unwrap(),
                    Box::new(clock.clone()),
                ),
                clock,
                session: Session::new(ConversationId::generate(), features),
                participant: MemoryId::generate(),
                inbound_batch: Vec::new(),
                participant_turn_ids: Vec::new(),
                describe_guard: tokio::sync::Mutex::new(()),
                adjudicator_cursor: Cell::new(Seq::ZERO),
                link_inference_cursor: Cell::new(Seq::ZERO),
            }
        }

        /// Catch the harness's vector index up to its log — embed everything committed since the last
        /// call, so a subsequent `memory.search` can find it. Panics if the harness has no retrieval.
        pub async fn index(&self) {
            let retrieval = self.engine.retrieval.as_ref().expect("retrieval attached");
            let from = retrieval.vectors.lock().cursor().unwrap().next();
            let events = self.engine.store.lock().read_from(from).unwrap();
            let batch = embed_batch(retrieval.embedder.as_ref(), &events)
                .await
                .unwrap();
            apply_batch(&mut **retrieval.vectors.lock(), batch).unwrap();
        }

        /// Mark every currently-stale memory described — call after `genesis::rollout` so a later
        /// [`Harness::describe`] only regenerates what the test itself wrote, not the seeded `self`
        /// (whose description genesis already provided). Records a `DescribePassCompleted` over the
        /// current stale set, so the baseline is derived from the log rather than a cursor position.
        pub fn baseline_descriptions(&self) {
            let stale = self.engine.graph.lock().stale_memories().unwrap();
            if stale.is_empty() {
                return;
            }
            let now = self.engine.clock.now();
            self.engine
                .store
                .lock()
                .append(
                    now,
                    EventSource::Agent,
                    vec![EventPayload::describe_pass_completed(stale)],
                )
                .unwrap();
            let mut graph = self.engine.graph.lock();
            graph
                .materialize_from(self.engine.store.lock().as_ref())
                .unwrap();
        }

        /// Baseline the link-inference cursor at the current log head — call after `genesis::rollout`
        /// so a later [`Harness::link_inference`] only infers from what the test itself wrote, not the
        /// seeded `self` (whose content genesis already provided).
        pub fn baseline_link_inference(&self) {
            self.link_inference_cursor
                .set(self.engine.store.lock().head().unwrap());
        }

        /// Run the description catch-up over everything written since the cursor — the off-hot-path
        /// regeneration the server's background describer does, driven explicitly (spec §Write path).
        /// Regenerates descriptions, belief arbitration, and temporal extraction for the memories the
        /// turn(s) since the last call wrote, advancing the cursor.
        pub async fn describe(&self, model: &dyn ModelClient) {
            run_describe_catch_up(&self.engine, model, &self.describe_guard)
                .await
                .unwrap();
        }

        /// Run the merge-adjudication catch-up over the proposals written since its cursor — the
        /// off-hot-path pass the server's background adjudicator does, driven explicitly. Weighs each
        /// proposed merge and, on acceptance, authors the `same_as`, advancing the cursor.
        pub async fn adjudicate(&self, model: &dyn ModelClient) {
            let (advanced, _) =
                run_adjudicate_catch_up(&self.engine, model, self.adjudicator_cursor.get())
                    .await
                    .unwrap();
            self.adjudicator_cursor.set(advanced);
        }

        /// Run the link-inference catch-up over everything written since its cursor — the off-hot-path
        /// pass the server's background link-inference worker does, driven explicitly (spec §Write
        /// path → link inference). Infers relationships from the memories written since the last call,
        /// advancing the cursor.
        pub async fn link_inference(&self, model: &dyn ModelClient) {
            let (advanced, _) =
                run_link_inference_catch_up(&self.engine, model, self.link_inference_cursor.get())
                    .await
                    .unwrap();
            self.link_inference_cursor.set(advanced);
        }

        /// Borrow the harness as a [`Turn`] over `model` for `inbound`, ready to hand to `run_turn`.
        /// Captures the full model-interaction record, the production default. Records the
        /// participant turn in the event log before returning, mirroring `route_messages`.
        pub fn as_turn<'a>(
            &'a mut self,
            model: &'a dyn ModelClient,
            inbound: &'a str,
            max_steps: usize,
        ) -> Turn<'a> {
            self.as_turn_capturing(model, inbound, max_steps, CaptureLevel::Full)
        }

        /// As [`Harness::as_turn`], but with an explicit model-interaction capture level — for tests
        /// that exercise the `Digest`/`Off` paths.
        pub fn as_turn_capturing<'a>(
            &'a mut self,
            model: &'a dyn ModelClient,
            inbound: &'a str,
            max_steps: usize,
            capture: CaptureLevel,
        ) -> Turn<'a> {
            self.prepare_inbound(inbound);
            Turn {
                session: &self.session,
                model,
                engine: self.engine.clone(),
                inbound: &self.inbound_batch,
                participant_turn_ids: &self.participant_turn_ids,
                brief: "",
                session_started_at: self.engine.clock.now(),
                buffer: &[],
                template: PromptTemplateName::Scaffold,
                authority: Authority::Platform,
                present_set: &[],
                brief_memories: &[],
                // The low-level agent-loop harness keeps ambient recall off so a seeded memory never
                // perturbs a test's recorded messages; the pass is exercised through `route_message`.
                ambient: AmbientSettings {
                    enabled: false,
                    ..AmbientSettings::default()
                },
                max_steps,
                block_timeout: TEST_BLOCK_TIMEOUT,
                max_block_attempts: TEST_MAX_BLOCK_ATTEMPTS,
                max_entry_chars: TEST_MAX_ENTRY_CHARS,
                capture,
            }
        }

        /// As [`Harness::as_turn`], but replaying `buffer` as the prior conversation — for multi-turn
        /// scenarios where a later turn must see what the agent said and did earlier (build it with
        /// `buffer_turns` over the recorded turns).
        pub fn as_turn_buffered<'a>(
            &'a mut self,
            model: &'a dyn ModelClient,
            inbound: &'a str,
            max_steps: usize,
            buffer: &'a [TurnView],
        ) -> Turn<'a> {
            self.prepare_inbound(inbound);
            Turn {
                session: &self.session,
                model,
                engine: self.engine.clone(),
                inbound: &self.inbound_batch,
                participant_turn_ids: &self.participant_turn_ids,
                brief: "",
                session_started_at: self.engine.clock.now(),
                buffer,
                template: PromptTemplateName::Scaffold,
                authority: Authority::Platform,
                present_set: &[],
                brief_memories: &[],
                ambient: AmbientSettings {
                    enabled: false,
                    ..AmbientSettings::default()
                },
                max_steps,
                block_timeout: TEST_BLOCK_TIMEOUT,
                max_block_attempts: TEST_MAX_BLOCK_ATTEMPTS,
                max_entry_chars: TEST_MAX_ENTRY_CHARS,
                capture: CaptureLevel::Full,
            }
        }

        /// Record the participant turn in the event log and populate the batch fields, mirroring
        /// what `route_messages` does before calling `run_session_turn`.
        fn prepare_inbound(&mut self, inbound: &str) {
            let turn_id = TurnId::generate();
            append_turn(
                self.engine.store.lock().as_mut(),
                self.engine.clock.as_ref(),
                TurnRecord {
                    conversation: self.session.conversation(),
                    turn_id,
                    role: TurnRole::Participant,
                    text: inbound.to_owned(),
                    participant: Some(self.participant),
                    initiation: Initiation::Responding,
                    produced_by: None,
                },
            )
            .unwrap();
            self.inbound_batch = vec![InboundMessage {
                participant: self.participant,
                text: inbound.to_owned(),
            }];
            self.participant_turn_ids = vec![turn_id];
        }

        /// Execute one Lua block against the harness's engine, as a fresh agent-authored turn (the
        /// teller is the agent; see the conversation tests for participant-attributed writes).
        pub async fn run(&self, script: &str) -> BlockOutcome {
            self.session
                .execute(
                    &self.engine,
                    &BlockContext {
                        teller: Teller::Agent,
                        authority: Authority::Platform,
                        turn_id: TurnId::generate(),
                        present_set: Vec::new(),
                        dry_run: false,
                        block_timeout: TEST_BLOCK_TIMEOUT,
                        max_block_attempts: TEST_MAX_BLOCK_ATTEMPTS,
                        max_entry_chars: TEST_MAX_ENTRY_CHARS,
                    },
                    &super::prepare_script(script),
                )
                .await
                .unwrap()
        }

        /// Execute one block with an explicit teller and present set — for the visibility-sensitive
        /// reads where *who is present* changes what a direct read may surface.
        pub async fn run_as(
            &self,
            teller: Teller,
            present_set: Vec<MemoryId>,
            script: &str,
        ) -> BlockOutcome {
            self.session
                .execute(
                    &self.engine,
                    &BlockContext {
                        teller,
                        authority: Authority::Platform,
                        turn_id: TurnId::generate(),
                        present_set,
                        dry_run: false,
                        block_timeout: TEST_BLOCK_TIMEOUT,
                        max_block_attempts: TEST_MAX_BLOCK_ATTEMPTS,
                        max_entry_chars: TEST_MAX_ENTRY_CHARS,
                    },
                    &super::prepare_script(script),
                )
                .await
                .unwrap()
        }

        /// The whole event log from seq zero, in order — the common test read after a turn or
        /// catch-up pass. Saves the `store.lock().read_from(Seq::ZERO).unwrap()` boilerplate.
        pub fn events(&self) -> Vec<Event> {
            self.engine.store.lock().read_from(Seq::ZERO).unwrap()
        }
    }
}
