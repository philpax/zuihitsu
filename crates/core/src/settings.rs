//! Behavioral settings — the agent's tunables, stored in the log as a single typed struct.
//!
//! These are distinct from the main crate's `EnvConfig`, which is the serving/environment config
//! read from `config.toml` (endpoints, sampling). Settings instead live *in the log*: a
//! `ConfigSet` event carries a whole [`Settings`] snapshot, seeded at genesis and replaced when an
//! operator changes a tunable, so replay reproduces the behavior each value produced. The current
//! settings are the latest snapshot ([`Settings::from_store`]).
//!
//! The schema is **append-only**: fields are deprecated, never removed, so every snapshot ever
//! written still deserializes. A field absent from an older snapshot deserializes to its build
//! default — every struct is `#[serde(default)]` over a [`Default`] of the spec's starting values —
//! which is exactly the "a knob that didn't exist at this agent's genesis adopts the build default"
//! behavior the configuration design calls for (spec §Initialization → configuration). This is a
//! grouped, typed struct, deliberately not a per-context policy language: per-context variation, if
//! ever wanted, belongs in the agent's reasoning over the [`Namespace::Context`] memory, not here.

use serde::{Deserialize, Serialize};

use crate::{
    event::EventPayload,
    ids::Seq,
    store::{Store, StoreError},
};

/// One minute in seconds — so a duration default reads as `30 * MINUTE` rather than `1_800`. The
/// wire format stays `i64` seconds; this only names the unit at the definition site.
const MINUTE: i64 = crate::time::SECONDS_PER_MINUTE;

/// The agent's behavioral tunables, grouped by the subsystem each shapes. [`Default`] is the spec's
/// starting values (each substruct carries its own).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Settings {
    pub compaction: CompactionSettings,
    pub checkpoint: CheckpointSettings,
    pub brief: BriefSettings,
    pub turn: TurnSettings,
    pub search: SearchSettings,
    pub scheduler: SchedulerSettings,
    pub concurrency: ConcurrencySettings,
    pub observability: ObservabilitySettings,
    pub memory: MemorySettings,
    pub web: WebSettings,
    pub ambient: AmbientSettings,
    pub maintenance: MaintenanceSettings,
}

/// Session segmentation and the carryover across a compaction seam.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
#[cfg_attr(
    feature = "ts",
    derive(ts_rs::TS, settings_metadata_derive::SettingsMetadata),
    settings_metadata(parent = "compaction")
)]
pub struct CompactionSettings {
    /// Buffer token budget that triggers a re-segment.
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub token_budget: i64,
    /// Quiet period that ends a session.
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub idle_gap_seconds: i64,
    /// How much raw transcript crosses a compaction boundary.
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub carryover_char_budget: i64,
    /// Minimum number of turns in the ending session for the pre-compaction flush to run — the
    /// flush-gating threshold. A low-activity session (e.g. one that crossed the budget via a single
    /// large paste) falls below it and skips the flush, so the hot-path model call is paid only when
    /// there is working state worth flushing (spec §Compaction → pre-compaction flush).
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub flush_min_turns: i64,
    /// The model's stated context window in tokens, recorded so observers can relate the buffer
    /// budget to the true window. `None` when the instance was created without a configured model.
    #[cfg_attr(feature = "ts", ts(type = "number | null"))]
    pub context_length: Option<i64>,
}

/// The mid-session checkpoint flush (spec §Compaction → checkpoint flush): a flush turn run while the
/// session stays open, so a parallel conversation can read this one's working state before it goes
/// idle. Gated three ways — substance, cooldown, and audience — so the model call is paid only when
/// there is an unflushed delta worth writing and another live conversation to read it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct CheckpointSettings {
    /// Whether the checkpoint sweeper runs at all.
    pub enabled: bool,
    /// Minimum unflushed delta, in characters of participant and agent turns past the session's flush
    /// watermark, for a checkpoint to fire — the substance gate.
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub min_delta_chars: i64,
    /// Minimum seconds since the session's last flush turn (or its start, if it has never flushed)
    /// before another checkpoint may run — the cooldown gate. Applies to the background timer sweep
    /// only; a fresh session opening waives it, since the opening session's brief needs the parallel
    /// conversations' state now.
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub cooldown_seconds: i64,
    /// Whether a fresh session opening for a conversation first checkpoint-flushes the *other* live
    /// conversations, so their working state reaches memory before the opening session's brief
    /// composes and its first turn dispatches. Substance-gated only (the cooldown and audience gates
    /// the timer sweep applies are waived); independent of the background timer, which `enabled`
    /// governs.
    pub flush_on_open: bool,
}

/// The fraction of the model's context window the compaction budget defaults to — the headroom left
/// for the system prefix and the reply when the agent re-segments. The window itself is operator-stated
/// config (the API does not report it), so the budget is derived from it at agent creation rather than
/// hardcoded; an explicit settings override still wins (spec §Compaction).
pub const COMPACTION_BUDGET_FRACTION: f64 = 0.8;

/// The compaction `token_budget` derived from a model's context window — [`COMPACTION_BUDGET_FRACTION`]
/// of it, in tokens.
pub fn compaction_budget_for(context_length: u32) -> i64 {
    (f64::from(context_length) * COMPACTION_BUDGET_FRACTION) as i64
}

/// Brief composition: what enters each brief, and how many participants get one.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
#[cfg_attr(
    feature = "ts",
    derive(ts_rs::TS, settings_metadata_derive::SettingsMetadata),
    settings_metadata(parent = "brief")
)]
pub struct BriefSettings {
    /// Character budget for a composed session brief. The composer packs blocks by priority until this
    /// is spent: the self and current-room blocks always render, then present-participant blocks in
    /// rank order, each dropping to a name-only line once the budget can no longer afford it, so a
    /// populous room cannot produce an unbounded brief. `0` disables everything but the mandatory
    /// blocks.
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub char_budget: i64,
    /// How many recent facts the brief includes.
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub recent_facts: i64,
    /// How many key relationships each brief block includes, after ranking them by type-weight and
    /// recency (a hub memory's edges are ranked and capped rather than all listed).
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub key_relationships: i64,
    /// The most entries in the brief's present set.
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub present_set_cap: i64,
    /// How far back a cold open looks for recently touched memories to re-surface as active threads,
    /// in days. A session that opens without a compaction carryover has no working set, so it derives
    /// one from the memories recent sessions across every conversation touched (their `LuaExecuted`
    /// touch sets), each still filtered through the visibility predicate against the new present set.
    /// `0` disables the cold-open derivation, leaving a fresh session's active-threads section empty.
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub cold_open_window_days: i64,
    /// The most active threads a cold open re-surfaces, after ranking the recently touched memories
    /// most-recent-first. Bounds the derived working set before the brief's own char budget packs it.
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub cold_open_threads: i64,
    /// How far ahead the `<upcoming/>` block looks, in days.
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub upcoming_window_days: i64,
    /// The most upcoming items the `<upcoming/>` block lists.
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub max_upcoming_items: i64,
}

/// Scheduled-work delivery: the drained wake-up surface (spec §Agent-initiated speech).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
#[cfg_attr(
    feature = "ts",
    derive(ts_rs::TS, settings_metadata_derive::SettingsMetadata),
    settings_metadata(parent = "scheduler")
)]
pub struct SchedulerSettings {
    /// The most fired wake-ups a single session-open drain raises.
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub max_wakeups_per_session: i64,
    /// How often the background scheduler driver fires due wake-ups, in seconds (spec §Scheduled work).
    /// Read by the serving host at startup, so a change takes effect on restart.
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub tick_seconds: i64,
}

/// The agent step loop.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
#[cfg_attr(
    feature = "ts",
    derive(ts_rs::TS, settings_metadata_derive::SettingsMetadata),
    settings_metadata(parent = "turn")
)]
pub struct TurnSettings {
    /// Per-turn step bound; hitting it ends the turn with a surfaced error.
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub max_steps: i64,
    /// Per-block duration budget (spec §Concurrency → lock acquisition): a block held longer than this —
    /// stuck on slow external I/O or a lock-wait — aborts, emitting nothing. Set generously, above a
    /// single MCP call's own timeout, so an ordinary multi-call block is never cut.
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub block_timeout_seconds: i64,
    /// How many times a block that times out on a lock-wait (with no MCP call) is re-run before giving
    /// up with a terminal error (spec §Concurrency → timeout-and-retry). A block that has made an MCP
    /// call is never retried, regardless of this bound.
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub max_block_attempts: i64,
    /// The window, in seconds, during which a new inbound message batch supersedes the conversation's
    /// in-flight turn (spec §Concurrency → per-conversation supersession). Measured from the arrival of
    /// the burst's first unanswered message: within it, a fresh batch cooperatively cancels the
    /// generating turn so the newer batch answers once with everything in context; once it has elapsed,
    /// the in-flight turn runs to completion and a later batch waits its turn instead. `0` disables
    /// supersession entirely — every batch waits for the previous turn to finish — while the
    /// per-conversation serialization stays on regardless.
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub supersede_window_seconds: i64,
}

/// Concurrency limits (spec §Concurrency): how many conversation streams may run at once. The shared
/// local model is the binding constraint, so this caps concurrent turns rather than letting unbounded
/// streams crowd the model. Read when the server is constructed, so a change takes effect on restart.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
#[cfg_attr(
    feature = "ts",
    derive(ts_rs::TS, settings_metadata_derive::SettingsMetadata),
    settings_metadata(parent = "concurrency")
)]
pub struct ConcurrencySettings {
    /// The most conversation turns that may be in flight at once; further streams queue for a slot.
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub max_concurrent_streams: i64,
    /// How stale the oldest pending description may get, in seconds, before the background describer
    /// escalates a sweep to conversation priority. Turn-over-background priority lets a busy instance
    /// hold the describer back indefinitely, so this is the release valve: once the oldest undescribed
    /// content change is older than this horizon, the next describe sweep runs at turn priority
    /// instead of yielding, keeping a persistent backlog from starving. Zero disables the escape.
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub describe_staleness_escape_seconds: i64,
}

/// Observability (spec §Observability): how much of each model call the model-interaction record
/// captures. The full request repeats the agent loop's growing buffer (delta-encoded, but still
/// material at the `Base` of each turn), so the verbosity is operator-tunable at runtime.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
#[cfg_attr(
    feature = "ts",
    derive(ts_rs::TS, settings_metadata_derive::SettingsMetadata),
    settings_metadata(parent = "observability")
)]
pub struct ObservabilitySettings {
    /// How much of each model call to record (the deliberation is always captured; this governs the
    /// request side).
    pub capture_model_calls: CaptureLevel,
}

/// How much of a model call's request the model-interaction record stores (spec §Observability). The
/// deliberation — reasoning, finish reason, usage, latency — is captured at every level above `Off`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub enum CaptureLevel {
    /// The delta-encoded request plus a digest — full reconstruction of every prompt.
    #[default]
    Full,
    /// Only the request digest (no message content), plus the full response.
    Digest,
    /// No model-interaction record at all.
    Off,
}

/// Memory write limits — guards against oversized entries that bloat the event log and pollute briefs
/// and search snippets.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct MemorySettings {
    /// The maximum character length of a single memory content entry. An entry exceeding this is
    /// rejected with a teachable error before it is buffered. The agent should summarize what it
    /// learned rather than pasting source content verbatim.
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub max_entry_chars: i64,
}

/// Web fetching (`web.markdown`): the transport limits and identity the in-house fetcher uses to pull
/// a page and extract its main content as Markdown. The transport-shaping values — the timeout, the
/// byte cap, the user agent, and the private-address gate — are read when the serving host constructs
/// the fetcher, so a change to them takes effect on restart; `max_markdown_chars` is applied to the
/// extracted Markdown per fetch.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct WebSettings {
    /// How long a single fetch may take before it is abandoned with a teachable timeout error. Set
    /// comfortably under the block timeout (`TurnSettings::block_timeout_seconds`), so a slow page
    /// fails the fetch cleanly rather than tripping the whole block's budget.
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub fetch_timeout_seconds: i64,
    /// The most bytes a response body may carry before the fetch is aborted. The cap is enforced as
    /// the body streams, so an oversized page is dropped mid-download rather than buffered whole.
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub max_response_bytes: i64,
    /// The most characters the extracted Markdown may carry. Content beyond this is truncated with an
    /// explicit marker, so a long page cannot flood the agent's context; the agent summarizes what it
    /// read into memory rather than holding the whole page.
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub max_markdown_chars: i64,
    /// The `User-Agent` header the fetcher sends, so a fetched host can identify the agent.
    pub user_agent: String,
    /// Whether to allow fetches that resolve to loopback, private (RFC 1918), link-local, or
    /// unique-local addresses. Off by default: the instance's own control API listens on localhost, so
    /// an agent-driven fetch to a private address is a server-side request forgery hazard. Enable it
    /// only for a deployment that deliberately fetches from a trusted private network.
    pub allow_private_addresses: bool,
}

/// Ambient recall: the fast pre-turn lexical pass that surfaces memories the frozen brief did not, so
/// the agent recalls what it would not have thought to search for. A hit weaker than `min_score`, or a
/// memory the brief already carries, is dropped; at most `max_hits` survive.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct AmbientSettings {
    /// Whether the ambient recall pass runs at all. Off leaves the turn assembled exactly as before —
    /// no hint, no `AmbientRecallSurfaced` record.
    pub enabled: bool,
    /// The most memories the ambient hint surfaces, after salience filtering and ranking best-first.
    /// Kept small: the hint is a spark for recall, not a second brief.
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub max_hits: i64,
    /// The weakest bm25 score a hit may carry and still surface. FTS5 bm25 is more negative for a
    /// stronger match, so a hit survives only when its best score is at or below this ceiling; a value
    /// nearer zero admits weaker matches, a more negative value demands stronger ones.
    pub min_score: f64,
}

/// Multi-signal search scoring (spec §Time → search scoring): the blend weights and the recency
/// decay.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
#[cfg_attr(
    feature = "ts",
    derive(ts_rs::TS, settings_metadata_derive::SettingsMetadata),
    settings_metadata(parent = "search")
)]
pub struct SearchSettings {
    /// Weight of cosine (semantic vector) similarity in the search blend.
    pub cosine: f32,
    /// Weight of BM25 (lexical) similarity in the search blend.
    pub bm25: f32,
    /// Weight of tag overlap in the search blend.
    pub tag: f32,
    pub recency: RecencySettings,
}

/// The recency bonus and its volatility-dependent decay constant.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
#[cfg_attr(
    feature = "ts",
    derive(ts_rs::TS, settings_metadata_derive::SettingsMetadata),
    settings_metadata(parent = "search.recency")
)]
pub struct RecencySettings {
    /// Maximum recency contribution (at zero age).
    pub bonus: f32,
    /// Decay time constant in days, by the memory's volatility.
    pub tau_days: TauDays,
}

/// The recency decay constant (in days) for each [`Volatility`](crate::event::Volatility) level.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
#[cfg_attr(
    feature = "ts",
    derive(ts_rs::TS, settings_metadata_derive::SettingsMetadata),
    settings_metadata(parent = "search.recency.tau_days")
)]
pub struct TauDays {
    /// Recency decay constant for high-volatility memories, in days.
    pub high: f32,
    /// Recency decay constant for medium-volatility memories, in days.
    pub medium: f32,
    /// Recency decay constant for low-volatility memories, in days.
    pub low: f32,
}

/// Maintenance pass scheduling (spec §Write path → maintenance passes). Each pass runs on
/// a timer, but only when enough activity has accrued since its last run — a pass that finds
/// nothing to do is cheap, and a pass that runs too soon after the last one wastes a model
/// call. The activity gate is per-pass: each tracks events since its last cursor advance.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
#[cfg_attr(
    feature = "ts",
    derive(ts_rs::TS, settings_metadata_derive::SettingsMetadata),
    settings_metadata(parent = "maintenance")
)]
pub struct MaintenanceSettings {
    /// Whether maintenance passes run at all. When false, the passes are registered but never fire
    /// on the timer; they can still be invoked on demand via the CLI/console.
    pub enabled: bool,
    /// How often the maintenance driver ticks, in seconds. The maintenance passes are heavier than
    /// describe/link-inference, so this defaults to a longer interval.
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub tick_seconds: i64,
    /// Minimum number of events since the last consolidation run before the next consolidation sweep
    /// fires. Zero disables the activity gate (runs every tick).
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub consolidation_min_activity: i64,
    /// Minimum number of events since the last canonicalize run before the next canonicalize sweep
    /// fires.
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub canonicalize_min_activity: i64,
    /// Minimum number of events since the last link-cleanup run before the next sweep fires.
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub link_cleanup_min_activity: i64,
    /// The loose cosine bar for gathering tier-1 consolidation candidates: entries whose contextual
    /// cosine clears it are clustered together as a *candidate* group, and the synthesis model call
    /// then decides actual membership. Geometry cannot reliably separate a genuine thematic fusion
    /// (four phrasings of one fact can sit at cosine 0.60–0.69 under the live embedder) from a
    /// related-but-distinct pair (which can sit at ~0.70, inside that same band), so the bar is set
    /// low to gather the fusion candidates and the model disposes. A false positive here is cheap —
    /// the model leaves the unrelated entry out of its selection and it stays live.
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub consolidation_candidate_threshold: f64,
    /// The cosine similarity threshold for the append-time cross-subject advisory band (the
    /// non-blocking dedup hint surfaced when a new entry resembles one about another subject). It is
    /// no longer the tier-1 clustering bar — `consolidation_candidate_threshold` gathers candidates
    /// and the synthesis model decides membership — so this now shapes only that advisory surface.
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub consolidation_similarity_threshold: f64,
    /// The cosine similarity threshold for the append-time dedup check. Higher than the
    /// consolidation threshold (e.g. 0.95) because a write-time rejection is more disruptive than a
    /// loose consolidation cluster.
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub dedup_similarity_threshold: f64,
}

impl Default for CompactionSettings {
    fn default() -> Self {
        CompactionSettings {
            token_budget: 24_000,
            idle_gap_seconds: 30 * MINUTE,
            carryover_char_budget: 4_000,
            flush_min_turns: 4,
            context_length: None,
        }
    }
}

impl Default for CheckpointSettings {
    fn default() -> Self {
        CheckpointSettings {
            enabled: true,
            min_delta_chars: 2_000,
            cooldown_seconds: 5 * MINUTE,
            flush_on_open: true,
        }
    }
}

impl Default for BriefSettings {
    fn default() -> Self {
        BriefSettings {
            char_budget: 8_000,
            recent_facts: 8,
            key_relationships: 8,
            present_set_cap: 10,
            cold_open_window_days: 7,
            cold_open_threads: 4,
            upcoming_window_days: 7,
            max_upcoming_items: 5,
        }
    }
}

impl Default for SchedulerSettings {
    fn default() -> Self {
        SchedulerSettings {
            max_wakeups_per_session: 5,
            tick_seconds: MINUTE,
        }
    }
}

impl Default for TurnSettings {
    fn default() -> Self {
        TurnSettings {
            max_steps: 12,
            block_timeout_seconds: 3 * MINUTE,
            max_block_attempts: 3,
            supersede_window_seconds: MINUTE,
        }
    }
}

impl Default for ConcurrencySettings {
    fn default() -> Self {
        ConcurrencySettings {
            max_concurrent_streams: 4,
            describe_staleness_escape_seconds: 5 * MINUTE,
        }
    }
}

impl Default for ObservabilitySettings {
    fn default() -> Self {
        ObservabilitySettings {
            capture_model_calls: CaptureLevel::Full,
        }
    }
}

impl Default for MemorySettings {
    fn default() -> Self {
        MemorySettings {
            max_entry_chars: 1_000,
        }
    }
}

impl Default for WebSettings {
    fn default() -> Self {
        WebSettings {
            fetch_timeout_seconds: 30,
            max_response_bytes: 5 * 1_024 * 1_024,
            max_markdown_chars: 20_000,
            user_agent: "zuihitsu/0.1 (+https://github.com/philpax/zuihitsu)".to_owned(),
            allow_private_addresses: false,
        }
    }
}

impl Default for AmbientSettings {
    fn default() -> Self {
        AmbientSettings {
            enabled: true,
            max_hits: 3,
            min_score: -0.3,
        }
    }
}

impl Default for SearchSettings {
    fn default() -> Self {
        SearchSettings {
            cosine: 0.5,
            bm25: 0.3,
            tag: 0.2,
            recency: RecencySettings::default(),
        }
    }
}

impl Default for RecencySettings {
    fn default() -> Self {
        RecencySettings {
            bonus: 0.3,
            tau_days: TauDays::default(),
        }
    }
}

impl Default for TauDays {
    fn default() -> Self {
        TauDays {
            high: 90.0,
            medium: 365.0,
            low: 3650.0,
        }
    }
}

impl Default for MaintenanceSettings {
    fn default() -> Self {
        MaintenanceSettings {
            enabled: true,
            tick_seconds: 60,
            consolidation_min_activity: 20,
            canonicalize_min_activity: 5,
            link_cleanup_min_activity: 20,
            consolidation_candidate_threshold: 0.60,
            consolidation_similarity_threshold: 0.85,
            dedup_similarity_threshold: 0.95,
        }
    }
}

impl Settings {
    /// The current settings: the latest `ConfigSet` snapshot in the log, or [`Default`] if none has
    /// been written yet.
    pub fn from_store(store: &dyn Store) -> Result<Settings, StoreError> {
        let mut settings = Settings::default();
        for event in store.read_from(Seq::ZERO)? {
            if let EventPayload::ConfigSet {
                settings: logged, ..
            } = event.payload
            {
                settings = logged;
            }
        }
        Ok(settings)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AmbientSettings, BriefSettings, CaptureLevel, CheckpointSettings, ConcurrencySettings,
        MemorySettings, ObservabilitySettings, SchedulerSettings, Settings, TurnSettings,
        WebSettings,
    };

    #[test]
    fn turn_defaults_match_the_spec_starting_values() {
        let turn = TurnSettings::default();
        assert_eq!(turn.max_steps, 12);
        assert_eq!(turn.block_timeout_seconds, 180);
        assert_eq!(turn.max_block_attempts, 3);
        assert_eq!(turn.supersede_window_seconds, 60);
    }

    #[test]
    fn concurrency_defaults_match_the_spec_starting_values() {
        assert_eq!(ConcurrencySettings::default().max_concurrent_streams, 4);
    }

    #[test]
    fn scheduler_defaults_match_the_spec_starting_values() {
        let scheduler = SchedulerSettings::default();
        assert_eq!(scheduler.max_wakeups_per_session, 5);
        assert_eq!(scheduler.tick_seconds, 60);
    }

    #[test]
    fn observability_defaults_to_full_capture() {
        assert_eq!(
            ObservabilitySettings::default().capture_model_calls,
            CaptureLevel::Full
        );
    }

    #[test]
    fn a_snapshot_predating_a_field_adopts_its_build_default() {
        // The schema is append-only: an older `ConfigSet` snapshot, written before `block_timeout_seconds`
        // (or the whole `concurrency` group) existed, omits the key — and must deserialize to the build
        // default rather than failing.
        let legacy = serde_json::json!({ "turn": { "max_steps": 7 } });
        let settings: Settings = serde_json::from_value(legacy).unwrap();
        assert_eq!(settings.turn.max_steps, 7);
        assert_eq!(
            settings.turn.block_timeout_seconds,
            TurnSettings::default().block_timeout_seconds
        );
        // `supersede_window_seconds` postdates the turn group; its absence adopts the build default.
        assert_eq!(
            settings.turn.supersede_window_seconds,
            TurnSettings::default().supersede_window_seconds
        );
        assert_eq!(
            settings.concurrency.max_concurrent_streams,
            ConcurrencySettings::default().max_concurrent_streams
        );
        // A field added to an existing group (scheduler's `tick_seconds`) likewise adopts its default.
        assert_eq!(
            settings.scheduler.tick_seconds,
            SchedulerSettings::default().tick_seconds
        );
        // A whole group added later (observability) adopts its default too.
        assert_eq!(
            settings.observability.capture_model_calls,
            CaptureLevel::Full
        );
        // The checkpoint group postdates many logged snapshots; its absence must default with the
        // sweeper enabled.
        assert_eq!(settings.checkpoint, CheckpointSettings::default());
        assert!(settings.checkpoint.enabled);
        // `flush_on_open` postdates the checkpoint group; its absence must default the open-triggered
        // sweep on.
        assert!(settings.checkpoint.flush_on_open);
        // The memory group postdates many logged snapshots; its absence must default to the limit.
        assert_eq!(settings.memory, MemorySettings::default());
        assert_eq!(settings.memory.max_entry_chars, 1_000);
        // The web group postdates many logged snapshots; its absence must default the fetcher's limits
        // and leave private-address fetches refused.
        assert_eq!(settings.web, WebSettings::default());
        assert!(!settings.web.allow_private_addresses);
        // The ambient group postdates many logged snapshots; its absence must default the pass on.
        assert_eq!(settings.ambient, AmbientSettings::default());
        assert!(settings.ambient.enabled);
    }

    #[test]
    fn ambient_defaults_match_the_spec_starting_values() {
        let ambient = AmbientSettings::default();
        assert!(ambient.enabled);
        assert_eq!(ambient.max_hits, 3);
        assert_eq!(ambient.min_score, -0.3);
    }

    #[test]
    fn web_defaults_match_the_spec_starting_values() {
        let web = WebSettings::default();
        assert_eq!(web.fetch_timeout_seconds, 30);
        assert_eq!(web.max_markdown_chars, 20_000);
        assert!(!web.allow_private_addresses);
    }

    #[test]
    fn memory_defaults_match_the_spec_starting_values() {
        let memory = MemorySettings::default();
        assert_eq!(memory.max_entry_chars, 1_000);
    }

    #[test]
    fn a_brief_snapshot_with_the_retired_token_budget_adopts_the_char_budget_default() {
        // The dead `token_budget` knob was replaced by an enforced `char_budget`. An older `ConfigSet`
        // snapshot still carries `token_budget`; the unknown key is ignored, and the absent
        // `char_budget` deserializes to its build default rather than failing.
        let legacy = serde_json::json!({ "brief": { "token_budget": 2_000, "recent_facts": 5 } });
        let settings: Settings = serde_json::from_value(legacy).unwrap();
        assert_eq!(settings.brief.recent_facts, 5);
        assert_eq!(
            settings.brief.char_budget,
            BriefSettings::default().char_budget
        );
    }
}
