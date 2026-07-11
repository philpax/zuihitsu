//! Runtime observability — the metrics a Grafana (or any Prometheus scraper) consumes, declared in
//! one place and observed through thin helpers (spec §Observability → metrics).
//!
//! Built on the `metrics` crate: each metric's name, help, and type are declared once in
//! [`describe`] (called at boot), and observation is a helper call (`observe_turn`, etc.) that
//! references the same `const` name. The [`metrics_exporter_prometheus`] recorder holds the state
//! and renders the Prometheus text for `/control/metrics`, so there is no hand-rolled snapshot or
//! renderer to keep in sync — adding a metric is one `const` + one `describe_*` + one helper.
//!
//! The set covers the four golden signals for the turn path (throughput, latency, errors,
//! saturation) plus agent-state gauges (the graph's size, the worker lag) and the agent's outward
//! I/O (MCP calls, Lua blocks, search) so an operator can tell "is the server up" from "is the agent
//! healthy" from "where did the time go."

use std::time::Duration;

use metrics::{counter, describe_counter, describe_gauge, describe_histogram, gauge, histogram};

use crate::model::{Usage, retry::CircuitState};

/// The histogram bucket bounds (seconds), shared by every latency histogram — turn duration, model
/// call duration, and MCP call duration are all latency-in-seconds, so one mesh keeps them
/// comparable. Cumulative, as Prometheus histograms are.
pub const LATENCY_BUCKETS: &[f64] = &[0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0, 30.0];

// Throughput.
pub const TURNS_TOTAL: &str = "zuihitsu_turns_total";
pub const MODEL_CALLS_TOTAL: &str = "zuihitsu_model_calls_total";
pub const MCP_CALLS_TOTAL: &str = "zuihitsu_mcp_calls_total";
pub const LUA_BLOCKS_TOTAL: &str = "zuihitsu_lua_blocks_total";
pub const MEMORY_SEARCH_TOTAL: &str = "zuihitsu_memory_search_total";
pub const WAKEUPS_FIRED_TOTAL: &str = "zuihitsu_wakeups_fired_total";
pub const WAKEUPS_SURFACED_TOTAL: &str = "zuihitsu_wakeups_surfaced_total";
pub const COMPACTIONS_TOTAL: &str = "zuihitsu_compactions_total";
pub const FLUSH_TURNS_TOTAL: &str = "zuihitsu_flush_turns_total";
pub const SESSIONS_OPENED_TOTAL: &str = "zuihitsu_sessions_opened_total";
pub const SESSIONS_CLOSED_TOTAL: &str = "zuihitsu_sessions_closed_total";

// Latency.
pub const TURNS_DURATION_SECONDS: &str = "zuihitsu_turns_duration_seconds";
pub const MODEL_DURATION_SECONDS: &str = "zuihitsu_model_duration_seconds";
pub const MCP_CALL_DURATION_SECONDS: &str = "zuihitsu_mcp_call_duration_seconds";
pub const MEMORY_SEARCH_DURATION_SECONDS: &str = "zuihitsu_memory_search_duration_seconds";

// Errors, labelled by category and cause (cause is `none` for non-turn errors).
pub const ERRORS_TOTAL: &str = "zuihitsu_errors_total";
pub const MCP_CALL_ERRORS_TOTAL: &str = "zuihitsu_mcp_call_errors_total";
pub const LUA_BLOCK_ERRORS_TOTAL: &str = "zuihitsu_lua_block_errors_total";

// Model-transport resilience: retries, the circuit breaker, and deferred turns.
pub const MODEL_RETRIES_TOTAL: &str = "zuihitsu_model_retries_total";
pub const MODEL_CIRCUIT_FAST_FAILS_TOTAL: &str = "zuihitsu_model_circuit_fast_fails_total";
pub const MODEL_CIRCUIT_STATE: &str = "zuihitsu_model_circuit_state";
pub const TURNS_DEFERRED_TOTAL: &str = "zuihitsu_turns_deferred_total";

// Turn-over-background priority at the shared model client.
pub const BACKGROUND_MODEL_DEFERRALS_TOTAL: &str = "zuihitsu_background_model_deferrals_total";

// Saturation: tokens.
pub const MODEL_PROMPT_TOKENS_TOTAL: &str = "zuihitsu_model_prompt_tokens_total";
pub const MODEL_COMPLETION_TOKENS_TOTAL: &str = "zuihitsu_model_completion_tokens_total";

// Saturation: gauges (process + agent state + worker lag, set at scrape time).
pub const UPTIME_SECONDS: &str = "zuihitsu_uptime_seconds";
pub const HEAD_SEQ: &str = "zuihitsu_head_seq";
pub const STORE_SIZE_BYTES: &str = "zuihitsu_store_size_bytes";
pub const SESSIONS_ACTIVE: &str = "zuihitsu_sessions_active";
pub const MEMORY_COUNT: &str = "zuihitsu_memory_count";
pub const ENTRY_COUNT: &str = "zuihitsu_entry_count";
pub const LINK_COUNT: &str = "zuihitsu_link_count";
pub const TAG_COUNT: &str = "zuihitsu_tag_count";
pub const RELATION_COUNT: &str = "zuihitsu_relation_count";
pub const INDEXER_LAG_SEQ: &str = "zuihitsu_indexer_lag_seq";
pub const DESCRIBER_STALE_MEMORIES: &str = "zuihitsu_describer_stale_memories";
pub const ADJUDICATOR_LAG_SEQ: &str = "zuihitsu_adjudicator_lag_seq";
pub const MCP_SERVERS_UP: &str = "zuihitsu_mcp_servers_up";
pub const MCP_TOOLS_TOTAL: &str = "zuihitsu_mcp_tools_total";

/// Declare every metric's help and type. Called once after the recorder is installed (at boot, or
/// in a test after `set_default_local_recorder`), so the Prometheus text carries `# HELP`/`# TYPE`
/// and a scraper sees the metric exists before the first observation.
pub fn describe() {
    // Throughput.
    describe_counter!(TURNS_TOTAL, "Conversational turns handled.");
    describe_counter!(
        MODEL_CALLS_TOTAL,
        "Model generate calls (turn steps, flushes, and background catch-ups)."
    );
    describe_counter!(MCP_CALLS_TOTAL, "MCP tool calls the agent made.");
    describe_counter!(LUA_BLOCKS_TOTAL, "run_lua blocks the agent executed.");
    describe_counter!(MEMORY_SEARCH_TOTAL, "memory.search calls.");
    describe_counter!(WAKEUPS_FIRED_TOTAL, "Due wake-ups the scheduler fired.");
    describe_counter!(
        WAKEUPS_SURFACED_TOTAL,
        "Fired wake-ups surfaced into an opening session."
    );
    describe_counter!(
        COMPACTIONS_TOTAL,
        "Sessions ended by a token-budget compaction."
    );
    describe_counter!(FLUSH_TURNS_TOTAL, "Pre-compaction / idle flush turns run.");
    describe_counter!(SESSIONS_OPENED_TOTAL, "Sessions opened.");
    describe_counter!(SESSIONS_CLOSED_TOTAL, "Sessions closed.");
    // Latency.
    describe_histogram!(TURNS_DURATION_SECONDS, "Conversational turn duration.");
    describe_histogram!(MODEL_DURATION_SECONDS, "Per-call model generate duration.");
    describe_histogram!(MCP_CALL_DURATION_SECONDS, "Per-call MCP tool duration.");
    describe_histogram!(MEMORY_SEARCH_DURATION_SECONDS, "memory.search duration.");
    // Errors.
    describe_counter!(ERRORS_TOTAL, "Failures, labelled by category and cause.");
    describe_counter!(
        MCP_CALL_ERRORS_TOTAL,
        "MCP tool calls that failed (spawn, protocol, tool, dead, timeout)."
    );
    describe_counter!(
        LUA_BLOCK_ERRORS_TOTAL,
        "run_lua blocks that ended in an error or abort."
    );
    // Model-transport resilience.
    describe_counter!(
        MODEL_RETRIES_TOTAL,
        "Transient model-call failures that were retried."
    );
    describe_counter!(
        MODEL_CIRCUIT_FAST_FAILS_TOTAL,
        "Model calls failed fast because the circuit was open (no backend call)."
    );
    describe_gauge!(
        MODEL_CIRCUIT_STATE,
        "The model circuit breaker's state: 0 closed, 1 half-open, 2 open."
    );
    describe_counter!(
        TURNS_DEFERRED_TOTAL,
        "Routed turns deferred because the model backend was unreachable (the inbound stays \
         durable; the next successful turn covers it)."
    );
    describe_counter!(
        BACKGROUND_MODEL_DEFERRALS_TOTAL,
        "Background model calls that waited for a pending conversation turn to dispatch first \
         (turn-over-background priority at the shared model client)."
    );
    // Saturation: tokens.
    describe_counter!(
        MODEL_PROMPT_TOKENS_TOTAL,
        "Prompt tokens reported by the model."
    );
    describe_counter!(
        MODEL_COMPLETION_TOKENS_TOTAL,
        "Completion tokens reported by the model."
    );
    // Gauges.
    describe_gauge!(UPTIME_SECONDS, "Process uptime.");
    describe_gauge!(
        HEAD_SEQ,
        "The event-log head seq (the highest committed sequence number)."
    );
    describe_gauge!(STORE_SIZE_BYTES, "The event-log file size in bytes.");
    describe_gauge!(
        SESSIONS_ACTIVE,
        "Live sessions (conversations with an open in-process session)."
    );
    describe_gauge!(MEMORY_COUNT, "Live memories in the graph.");
    describe_gauge!(ENTRY_COUNT, "Live content entries in the graph.");
    describe_gauge!(LINK_COUNT, "Links in the graph.");
    describe_gauge!(TAG_COUNT, "Tags in the graph.");
    describe_gauge!(RELATION_COUNT, "Registered link relations.");
    describe_gauge!(
        INDEXER_LAG_SEQ,
        "How far the vector indexer trails the log head, in seqs."
    );
    describe_gauge!(
        DESCRIBER_STALE_MEMORIES,
        "The background describer's backlog: memories whose content has changed since they were \
         last described."
    );
    describe_gauge!(
        ADJUDICATOR_LAG_SEQ,
        "How far the background adjudicator trails the log head, in seqs."
    );
    describe_gauge!(MCP_SERVERS_UP, "MCP servers brought up at boot.");
    describe_gauge!(
        MCP_TOOLS_TOTAL,
        "Total projected MCP tools across all servers."
    );
}

// === Observation helpers (the turn path and background workers). ===

/// Observe one completed turn: throughput + latency.
pub fn observe_turn(duration: Duration) {
    counter!(TURNS_TOTAL).increment(1);
    histogram!(TURNS_DURATION_SECONDS).record(duration.as_secs_f64());
}

/// Observe a turn-path failure: as a turn (throughput + latency) plus a labelled error.
/// `cause` distinguishes `model`/`lua`/`store`/`graph` for turn errors; pass `none` when the
/// failure has no finer cause.
pub fn observe_turn_error(category: &str, cause: &str, duration: Duration) {
    observe_turn(duration);
    // Label values must be `'static`; dynamic `&str` args are owned into `SharedString`.
    counter!(ERRORS_TOTAL, "category" => category.to_string(), "cause" => cause.to_string())
        .increment(1);
}

/// Observe a failure in a background worker pass (no turn-duration entry). Categories are `describe`,
/// `adjudicate`, `indexer`, `scheduler`, `sweep`.
pub fn observe_worker_error(category: &str) {
    counter!(ERRORS_TOTAL, "category" => category.to_string(), "cause" => "none").increment(1);
}

/// Observe one background model call deferred behind a pending conversation turn — the arbiter held
/// the background pass back until the turn dispatched (spec §Write path → model sharing). Counted
/// once per background call that had to wait at all, not per wait iteration.
pub fn observe_background_model_deferral() {
    counter!(BACKGROUND_MODEL_DEFERRALS_TOTAL).increment(1);
}

/// Observe one model `generate` call: saturation (calls, latency, tokens).
pub fn observe_model_call(duration: Duration, usage: &Usage) {
    counter!(MODEL_CALLS_TOTAL).increment(1);
    histogram!(MODEL_DURATION_SECONDS).record(duration.as_secs_f64());
    if let Some(prompt) = usage.prompt_tokens {
        counter!(MODEL_PROMPT_TOKENS_TOTAL).increment(prompt as u64);
    }
    if let Some(completion) = usage.completion_tokens {
        counter!(MODEL_COMPLETION_TOKENS_TOTAL).increment(completion as u64);
    }
}

/// Observe one MCP tool call: throughput + latency. Pair with [`observe_mcp_call_error`] on failure.
pub fn observe_mcp_call(duration: Duration) {
    counter!(MCP_CALLS_TOTAL).increment(1);
    histogram!(MCP_CALL_DURATION_SECONDS).record(duration.as_secs_f64());
}

/// Observe a failed MCP tool call.
pub fn observe_mcp_call_error() {
    counter!(MCP_CALL_ERRORS_TOTAL).increment(1);
}

/// Observe one `run_lua` block executed; pair with [`observe_lua_block_error`] when it terminated.
pub fn observe_lua_block() {
    counter!(LUA_BLOCKS_TOTAL).increment(1);
}

/// Observe a `run_lua` block that ended in an error or abort (catchable — the turn continues).
pub fn observe_lua_block_error() {
    counter!(LUA_BLOCK_ERRORS_TOTAL).increment(1);
}

/// Observe one transport retry of a model call: the attempt failed transiently and the wrapper is
/// about to try again. Infra-transparent to the log (spec §Event sourcing) — this counter and the
/// paired `tracing::warn!` are the only trace a retry leaves.
pub fn observe_model_retry() {
    counter!(MODEL_RETRIES_TOTAL).increment(1);
}

/// Observe a model call that failed fast because the circuit was open — no backend call was made.
pub fn observe_model_circuit_fast_fail() {
    counter!(MODEL_CIRCUIT_FAST_FAILS_TOTAL).increment(1);
}

/// Record that a routed turn was deferred: the inbound is durable, but the model backend was
/// unreachable, so no response cycle ran (the next successful turn's buffer replay covers it).
pub fn observe_turn_deferred() {
    counter!(TURNS_DEFERRED_TOTAL).increment(1);
}

/// Set the model circuit-breaker state gauge: `0` closed, `1` half-open, `2` open.
pub fn set_model_circuit_state(state: CircuitState) {
    let value = match state {
        CircuitState::Closed => 0.0,
        CircuitState::HalfOpen => 1.0,
        CircuitState::Open => 2.0,
    };
    gauge!(MODEL_CIRCUIT_STATE).set(value);
}

/// Observe one `memory.search`: throughput + latency.
pub fn observe_search(duration: Duration) {
    counter!(MEMORY_SEARCH_TOTAL).increment(1);
    histogram!(MEMORY_SEARCH_DURATION_SECONDS).record(duration.as_secs_f64());
}

/// Record that the scheduler fired `count` due wake-ups.
pub fn observe_wakeups_fired(count: usize) {
    counter!(WAKEUPS_FIRED_TOTAL).increment(count as u64);
}

/// Record that `count` fired wake-ups were surfaced into an opening session.
pub fn observe_wakeups_surfaced(count: usize) {
    counter!(WAKEUPS_SURFACED_TOTAL).increment(count as u64);
}

/// Record that a session opened.
pub fn observe_session_opened() {
    counter!(SESSIONS_OPENED_TOTAL).increment(1);
}

/// Record that a session closed.
pub fn observe_session_closed() {
    counter!(SESSIONS_CLOSED_TOTAL).increment(1);
}

/// Record that a session was ended for a token-budget compaction.
pub fn observe_compaction() {
    counter!(COMPACTIONS_TOTAL).increment(1);
}

/// Record that a pre-compaction / idle flush turn ran.
pub fn observe_flush_turn() {
    counter!(FLUSH_TURNS_TOTAL).increment(1);
}

// === Gauge setters (derived values, set at scrape time). ===

/// Set the process-scrape gauges the library does not derive: uptime and the event-log file size.
pub fn set_process_gauges(uptime_seconds: f64, store_size_bytes: Option<u64>) {
    gauge!(UPTIME_SECONDS).set(uptime_seconds);
    if let Some(bytes) = store_size_bytes {
        gauge!(STORE_SIZE_BYTES).set(bytes as f64);
    }
}

/// Set the live-session gauge.
pub fn set_sessions_active(count: u64) {
    gauge!(SESSIONS_ACTIVE).set(count as f64);
}

/// Set the head-seq gauge.
pub fn set_head_seq(seq: u64) {
    gauge!(HEAD_SEQ).set(seq as f64);
}

/// Set the agent-state gauges: the graph's size (memories, entries, links, tags, relations).
pub fn set_graph_counts(memories: u64, entries: u64, links: u64, tags: usize, relations: usize) {
    gauge!(MEMORY_COUNT).set(memories as f64);
    gauge!(ENTRY_COUNT).set(entries as f64);
    gauge!(LINK_COUNT).set(links as f64);
    gauge!(TAG_COUNT).set(tags as f64);
    gauge!(RELATION_COUNT).set(relations as f64);
}

/// Set the worker-lag gauges. `indexer_lag` is `None` on a graph-only instance (no embedder).
pub fn set_lag(indexer_lag: Option<u64>, describer_backlog: u64, adjudicator_lag: u64) {
    if let Some(lag) = indexer_lag {
        gauge!(INDEXER_LAG_SEQ).set(lag as f64);
    }
    gauge!(DESCRIBER_STALE_MEMORIES).set(describer_backlog as f64);
    gauge!(ADJUDICATOR_LAG_SEQ).set(adjudicator_lag as f64);
}

/// Set the MCP-health gauges.
pub fn set_mcp(servers_up: usize, tools_total: usize) {
    gauge!(MCP_SERVERS_UP).set(servers_up as f64);
    gauge!(MCP_TOOLS_TOTAL).set(tools_total as f64);
}

#[cfg(test)]
mod tests {
    //! Observe into a local recorder and render, proving the helpers and the declaration stay in
    //! sync — a metric observed but not declared still renders a sample, and a declared one renders
    //! its HELP/TYPE. Uses `set_default_local_recorder` so the test is isolated (no global recorder).
    use super::*;
    use metrics_exporter_prometheus::PrometheusBuilder;

    fn render(observed: impl FnOnce()) -> String {
        let recorder = PrometheusBuilder::new()
            .set_buckets(LATENCY_BUCKETS)
            .unwrap()
            .build_recorder();
        let handle = recorder.handle();
        let _guard = metrics::set_default_local_recorder(&recorder);
        describe();
        observed();
        handle.render()
    }

    #[test]
    fn a_turn_observes_throughput_latency_and_tokens() {
        let text = render(|| {
            observe_turn(Duration::from_millis(850));
            observe_model_call(
                Duration::from_millis(600),
                &Usage {
                    prompt_tokens: Some(120),
                    completion_tokens: Some(30),
                    total_tokens: Some(150),
                    ..Usage::default()
                },
            );
        });
        assert!(text.contains("# TYPE zuihitsu_turns_total counter\n"));
        assert!(text.contains("zuihitsu_turns_total 1\n"));
        assert!(text.contains("zuihitsu_model_calls_total 1\n"));
        assert!(text.contains("zuihitsu_model_prompt_tokens_total 120\n"));
        assert!(text.contains("zuihitsu_model_completion_tokens_total 30\n"));
        assert!(text.contains("# TYPE zuihitsu_turns_duration_seconds histogram\n"));
        assert!(text.contains("zuihitsu_turns_duration_seconds_count 1\n"));
    }

    #[test]
    fn errors_carry_category_and_cause_labels() {
        let text = render(|| {
            observe_turn_error("turn", "lua", Duration::from_millis(500));
            observe_worker_error("describe");
        });
        assert!(text.contains("zuihitsu_errors_total{category=\"turn\",cause=\"lua\"} 1"));
        assert!(text.contains("zuihitsu_errors_total{category=\"describe\",cause=\"none\"} 1"));
    }

    #[test]
    fn every_metric_observes_and_renders_its_type() {
        // A metric only renders once observed (standard PrometheusExporter behavior), so observing
        // each helper and asserting its TYPE line catches both a name typo in the observe call (the
        // sample would render with no TYPE) and a missing describe (no HELP/TYPE). This is the
        // single wiring check that the declaration and the observation stay in sync.
        let text = render(|| {
            observe_turn(Duration::from_millis(10));
            observe_turn_error("turn", "lua", Duration::from_millis(10));
            observe_worker_error("describe");
            observe_model_call(
                Duration::from_millis(10),
                &Usage {
                    prompt_tokens: Some(1),
                    completion_tokens: Some(1),
                    total_tokens: Some(2),
                    ..Usage::default()
                },
            );
            observe_mcp_call(Duration::from_millis(10));
            observe_mcp_call_error();
            observe_lua_block();
            observe_lua_block_error();
            observe_model_retry();
            observe_model_circuit_fast_fail();
            observe_turn_deferred();
            set_model_circuit_state(CircuitState::Open);
            observe_search(Duration::from_millis(10));
            observe_wakeups_fired(1);
            observe_wakeups_surfaced(1);
            observe_session_opened();
            observe_session_closed();
            observe_compaction();
            observe_flush_turn();
            set_process_gauges(1.0, Some(1));
            set_sessions_active(1);
            set_head_seq(1);
            set_graph_counts(1, 1, 1, 1, 1);
            set_lag(Some(1), 1, 1);
            set_mcp(1, 1);
        });
        for name in [
            TURNS_TOTAL,
            MODEL_CALLS_TOTAL,
            MCP_CALLS_TOTAL,
            LUA_BLOCKS_TOTAL,
            MEMORY_SEARCH_TOTAL,
            WAKEUPS_FIRED_TOTAL,
            WAKEUPS_SURFACED_TOTAL,
            COMPACTIONS_TOTAL,
            FLUSH_TURNS_TOTAL,
            SESSIONS_OPENED_TOTAL,
            SESSIONS_CLOSED_TOTAL,
            ERRORS_TOTAL,
            MCP_CALL_ERRORS_TOTAL,
            LUA_BLOCK_ERRORS_TOTAL,
            MODEL_RETRIES_TOTAL,
            MODEL_CIRCUIT_FAST_FAILS_TOTAL,
            MODEL_CIRCUIT_STATE,
            TURNS_DEFERRED_TOTAL,
            MODEL_PROMPT_TOKENS_TOTAL,
            MODEL_COMPLETION_TOKENS_TOTAL,
            TURNS_DURATION_SECONDS,
            MODEL_DURATION_SECONDS,
            MCP_CALL_DURATION_SECONDS,
            MEMORY_SEARCH_DURATION_SECONDS,
            UPTIME_SECONDS,
            HEAD_SEQ,
            STORE_SIZE_BYTES,
            SESSIONS_ACTIVE,
            MEMORY_COUNT,
            ENTRY_COUNT,
            LINK_COUNT,
            TAG_COUNT,
            RELATION_COUNT,
            INDEXER_LAG_SEQ,
            DESCRIBER_STALE_MEMORIES,
            ADJUDICATOR_LAG_SEQ,
            MCP_SERVERS_UP,
            MCP_TOOLS_TOTAL,
        ] {
            assert!(
                text.contains(&format!("# TYPE {name}")),
                "missing TYPE for {name}"
            );
        }
        // The labelled error counter carries both labels.
        assert!(text.contains("zuihitsu_errors_total{category=\"turn\",cause=\"lua\"} 1"));
    }
}
