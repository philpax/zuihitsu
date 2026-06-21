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
//! ever wanted, belongs in the agent's reasoning over the `context/*` memory, not here.

use serde::{Deserialize, Serialize};

use crate::{
    event::EventPayload,
    ids::Seq,
    store::{Store, StoreError},
};

/// The agent's behavioral tunables, grouped by the subsystem each shapes. [`Default`] is the spec's
/// starting values (each substruct carries its own).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Settings {
    pub compaction: CompactionSettings,
    pub brief: BriefSettings,
    pub turn: TurnSettings,
    pub search: SearchSettings,
    pub scheduler: SchedulerSettings,
    pub concurrency: ConcurrencySettings,
    pub observability: ObservabilitySettings,
}

/// Session segmentation and the carryover across a compaction seam.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
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
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct BriefSettings {
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub token_budget: i64,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub recent_facts: i64,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub present_set_cap: i64,
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
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
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
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
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
}

/// Concurrency limits (spec §Concurrency): how many conversation streams may run at once. The shared
/// local model is the binding constraint, so this caps concurrent turns rather than letting unbounded
/// streams crowd the model. Read when the server is constructed, so a change takes effect on restart.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ConcurrencySettings {
    /// The most conversation turns that may be in flight at once; further streams queue for a slot.
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub max_concurrent_streams: i64,
}

/// Observability (spec §Observability): how much of each model call the model-interaction record
/// captures. The full request repeats the agent loop's growing buffer (delta-encoded, but still
/// material at the `Base` of each turn), so the verbosity is operator-tunable at runtime.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
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

/// Multi-signal search scoring (spec §Time → search scoring): the blend weights and the recency
/// decay.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct SearchSettings {
    pub cosine: f32,
    pub bm25: f32,
    pub tag: f32,
    pub recency: RecencySettings,
}

/// The recency bonus and its volatility-dependent decay constant.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct RecencySettings {
    /// Maximum recency contribution (at zero age).
    pub bonus: f32,
    /// Decay time constant in days, by the memory's volatility.
    pub tau_days: TauDays,
}

/// The recency decay constant (in days) for each [`Volatility`](crate::event::Volatility) level.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct TauDays {
    pub high: f32,
    pub medium: f32,
    pub low: f32,
}

impl Default for CompactionSettings {
    fn default() -> Self {
        CompactionSettings {
            token_budget: 24_000,
            idle_gap_seconds: 1_800,
            carryover_char_budget: 4_000,
            flush_min_turns: 4,
        }
    }
}

impl Default for BriefSettings {
    fn default() -> Self {
        BriefSettings {
            token_budget: 2_000,
            recent_facts: 8,
            present_set_cap: 10,
            upcoming_window_days: 7,
            max_upcoming_items: 5,
        }
    }
}

impl Default for SchedulerSettings {
    fn default() -> Self {
        SchedulerSettings {
            max_wakeups_per_session: 5,
            tick_seconds: 60,
        }
    }
}

impl Default for TurnSettings {
    fn default() -> Self {
        TurnSettings {
            max_steps: 12,
            block_timeout_seconds: 180,
            max_block_attempts: 3,
        }
    }
}

impl Default for ConcurrencySettings {
    fn default() -> Self {
        ConcurrencySettings {
            max_concurrent_streams: 4,
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
        CaptureLevel, ConcurrencySettings, ObservabilitySettings, SchedulerSettings, Settings,
        TurnSettings,
    };

    #[test]
    fn turn_defaults_match_the_spec_starting_values() {
        let turn = TurnSettings::default();
        assert_eq!(turn.max_steps, 12);
        assert_eq!(turn.block_timeout_seconds, 180);
        assert_eq!(turn.max_block_attempts, 3);
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
    }
}
