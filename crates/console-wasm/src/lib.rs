//! The console's WASM bridge.
//!
//! A [`Replica`] holds an event log and the graph it folds into, using `zuihitsu-core`'s real
//! materializer — the same projection the live agent runs (see `console/PLAN.md`). The frontend
//! constructs one from a run's `Event[]` (an eval package now, a live `/control` stream later) and
//! queries it for the State and Time-travel views. The event-stream views (Events, Conversation)
//! and the eval-package chrome render off the JSON directly, so they need nothing here.
//!
//! The boundary discipline: events come in as raw JSON bytes parsed by `serde` *inside* the module
//! (one copy across the boundary), and results go out through `serde-wasm-bindgen`'s JSON-compatible
//! serializer, so numbers land as JS numbers rather than `BigInt` — matching the ts-rs bindings,
//! which type `Seq` and the timestamps as `number`.

use serde::Serialize;
use wasm_bindgen::prelude::*;
use zuihitsu_core::{
    event::Event,
    graph::{EntryView, Graph, LinkView, MemoryView},
    ids::Seq,
};

/// Everything the State view shows when a memory is opened: the memory itself, its live content
/// entries, its full history (including superseded entries), its links, and its `same_as` class.
/// Composed from several core reads so the frontend opens a memory in one call.
#[derive(Serialize)]
struct MemoryDetail {
    memory: MemoryView,
    entries: Vec<EntryView>,
    history: Vec<EntryView>,
    links: Vec<LinkView>,
    class: Vec<MemoryView>,
}

/// A materializing read replica: an event log plus the graph state it folds into. The log is
/// retained so the graph can be re-folded to any earlier `Seq` for time-travel.
#[wasm_bindgen]
pub struct Replica {
    events: Vec<Event>,
    graph: Graph,
    /// The highest `Seq` currently folded into `graph` (the fold horizon).
    head: Seq,
}

#[wasm_bindgen]
impl Replica {
    /// Build a replica from a JSON-encoded `Event[]` — a run's embedded log, or a live catch-up
    /// batch. The events are folded through the real materializer up to their head.
    #[wasm_bindgen(constructor)]
    pub fn new(events_json: &[u8]) -> Result<Replica, JsError> {
        let mut events: Vec<Event> = serde_json::from_slice(events_json)
            .map_err(|error| JsError::new(&format!("console: parsing the event log: {error}")))?;
        events.sort_by_key(|event| event.seq);

        let mut replica = Replica {
            events,
            graph: Graph::open_in_memory().map_err(graph_error)?,
            head: Seq::ZERO,
        };
        let head = replica
            .events
            .last()
            .map(|event| event.seq)
            .unwrap_or(Seq::ZERO);
        replica.fold_through(head)?;
        Ok(replica)
    }

    /// The number of events in the log (independent of the fold horizon).
    #[wasm_bindgen(getter, js_name = eventCount)]
    pub fn event_count(&self) -> usize {
        self.events.len()
    }

    /// The highest `Seq` in the log — the upper bound of the time-travel range.
    #[wasm_bindgen(getter, js_name = headSeq)]
    pub fn head_seq(&self) -> f64 {
        self.events
            .last()
            .map(|event| event.seq.0 as f64)
            .unwrap_or(0.0)
    }

    /// The `Seq` currently folded into the graph (what the queries below reflect).
    #[wasm_bindgen(getter, js_name = foldedSeq)]
    pub fn folded_seq(&self) -> f64 {
        self.head.0 as f64
    }

    /// Re-fold the graph to reflect only events with `seq <= up_to` — the time-travel scrub. Folding
    /// from zero each time is fine at a run's scale; caching checkpoints is a later optimization.
    #[wasm_bindgen(js_name = foldTo)]
    pub fn fold_to(&mut self, up_to: f64) -> Result<(), JsError> {
        self.fold_through(Seq(up_to.max(0.0) as u64))
    }

    /// Every memory at the current fold horizon, as `MemoryView[]`, ordered by name. Pass a `prefix`
    /// (e.g. `"person/"`) to scope by namespace, or an empty string for all.
    pub fn memories(&self, prefix: &str) -> Result<JsValue, JsError> {
        let mut memories = self
            .graph
            .memories_in_namespace(prefix)
            .map_err(graph_error)?;
        memories.sort_by(|a, b| a.name.as_str().cmp(b.name.as_str()));
        to_js(&memories)
    }

    /// The full State-view detail for one memory by name, or `null` if there is no such memory at
    /// the current fold horizon. Bundles its live entries, its history, its links, and its `same_as`
    /// class so the frontend opens a memory in a single call.
    pub fn memory(&self, name: &str) -> Result<JsValue, JsError> {
        let Some(memory) = self.graph.memory_by_name(name).map_err(graph_error)? else {
            return Ok(JsValue::NULL);
        };
        let entries = self.graph.entries_local(memory.id).map_err(graph_error)?;
        let history = self
            .graph
            .entries_local_history(memory.id)
            .map_err(graph_error)?;
        let links = self.graph.links(memory.id).map_err(graph_error)?;
        let mut class = Vec::new();
        for id in self.graph.class_members(memory.id).map_err(graph_error)? {
            if let Some(view) = self.graph.memory_by_id(id).map_err(graph_error)? {
                class.push(view);
            }
        }
        to_js(&MemoryDetail {
            memory,
            entries,
            history,
            links,
            class,
        })
    }

    /// The tag vocabulary at the current fold horizon, as `TagVocabularyEntry[]` (name, purpose, and
    /// live-use count).
    pub fn tags(&self) -> Result<JsValue, JsError> {
        to_js(&self.graph.all_tags().map_err(graph_error)?)
    }

    /// The registered link relations at the current fold horizon, as `RelationView[]`.
    pub fn relations(&self) -> Result<JsValue, JsError> {
        to_js(&self.graph.all_relations().map_err(graph_error)?)
    }
}

impl Replica {
    /// Rebuild `graph` from a fresh in-memory projection, applying every event with `seq <= up_to`.
    fn fold_through(&mut self, up_to: Seq) -> Result<(), JsError> {
        let mut graph = Graph::open_in_memory().map_err(graph_error)?;
        for event in self.events.iter().filter(|event| event.seq <= up_to) {
            graph.apply(event).map_err(graph_error)?;
        }
        self.graph = graph;
        self.head = up_to;
        Ok(())
    }
}

/// Serialize a value to a JS value with the JSON-compatible number policy (plain numbers, not
/// `BigInt`), so the result matches the ts-rs bindings the frontend is typed against.
fn to_js<T: Serialize>(value: &T) -> Result<JsValue, JsError> {
    value
        .serialize(&serde_wasm_bindgen::Serializer::json_compatible())
        .map_err(|error| JsError::new(&format!("console: serializing a result: {error}")))
}

/// Render a core graph error as a JS error, leading with the console context.
fn graph_error(error: zuihitsu_core::graph::GraphError) -> JsError {
    JsError::new(&format!("console: {error}"))
}
