//! The serving mode the shared console bundle boots into. The console's `index.html` carries a
//! `__ZUIHITSU_APP_MODE__` template token that the serving binary replaces at serve time; this
//! enum is the typed, shared spelling of that token, used by both the backend (which injects it)
//! and the frontend (which reads `window.__APP_MODE__`).

use serde::{Deserialize, Serialize};

/// Which host mode the shared console bundle boots into. The single Vite bundle serves three
/// modes: the agent's live view (`agent`), the eval viewer (`eval`), and the standalone console
/// (`console`). `Console` is never injected by a serving binary: the standalone mode is the token
/// left unreplaced, and the frontend falls back to it when `window.__APP_MODE__` carries neither
/// `agent` nor `eval`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "lowercase")]
#[cfg_attr(feature = "ts", ts(rename_all = "lowercase"))]
pub enum AppMode {
    /// The agent binary's focused live view.
    Agent,
    /// The eval binary's live eval viewer.
    Eval,
    /// The standalone console: landing page, package picker, and trends over `eval/history.jsonl`.
    Console,
}

impl AppMode {
    /// The template-token value this mode replaces `__ZUIHITSU_APP_MODE__` with in the HTML shell.
    pub fn as_str(self) -> &'static str {
        match self {
            AppMode::Agent => "agent",
            AppMode::Eval => "eval",
            AppMode::Console => "console",
        }
    }
}
