//! Per-instance API feature flags — which Lua API features an instance enables.
//!
//! The flags gate three things in lockstep: the functions installed on the Lua globals, the entries
//! in the system prompt's API description, and the scaffold dotpoints that teach the agent to use
//! them. A function the prompt teaches but the runtime rejects is a confusing failure; a function
//! the runtime accepts but the prompt hides is an undiscoverable capability — so the three gates
//! must move together (see `CONTRIBUTING.md` → Instance features).
//!
//! The flags are coarse-grained: a feature maps to a *practice* (linking, tagging, …) and its whole
//! group of API functions, not per-function, because the scaffold's dotpoints teach practices that
//! span several calls each.

/// Which Lua API features an instance enables — controls the functions installed on the Lua
/// globals, the entries in the system prompt's API description, and the scaffold dotpoints that
/// teach the agent to use them. Coarse-grained: a feature maps to a practice (linking, tagging,
/// …) and its whole group of API functions, not per-function.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InstanceFeatures {
    /// Memory write/read — `memory.create`, `:append`, `:entries`, `:supersede`, etc. Always on;
    /// an agent without memory is not an agent. Included for completeness, not togglable.
    pub memory: bool,
    /// Link registration and traversal — `:link`, `:unlink`, `:outgoing`, `links.register`, etc.
    pub linking: bool,
    /// Tag application and vocabulary — `:tag`, `:untag`, `tags.create`, etc.
    pub tagging: bool,
    /// Cross-platform merge proposals — `:propose_merge`.
    pub merging: bool,
    /// Calendar queries and date arithmetic — `calendar.*`, `:add_days`, etc.
    pub calendar: bool,
    /// Context memory access — `context.current`. Always on; the brief and session machinery
    /// depend on it. Included for completeness, not togglable.
    pub context: bool,
}

impl Default for InstanceFeatures {
    fn default() -> Self {
        InstanceFeatures {
            memory: true,
            linking: true,
            tagging: true,
            merging: true,
            calendar: true,
            context: true,
        }
    }
}
