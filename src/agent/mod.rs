//! The agent subsystem: the turn loop and the surfaces it speaks through.
//!
//! [`turn`] is the loop itself — it drives the model, runs Lua blocks, and commits their effects;
//! [`lua`] is the scripting seam the agent acts through; [`system_prompt`] assembles the prompt the
//! loop sends; and [`api_doc`], [`genesis`], and [`templates`] are the ungated scaffolding (the API
//! reference renderer, the seed rollout, and the prompt templates) the rest of the crate also uses.
//!
//! The gates differ per submodule, so this hub stays ungated rather than being the loop itself;
//! `pub use turn::*` re-exports the loop at this module's root so `crate::agent::run_turn` and the
//! crate-root re-exports are unchanged.

pub mod api_doc;
pub mod genesis;
pub mod lua;
pub mod maintenance;
mod mcp_api;
pub use mcp_api::McpCatalogue;
pub mod system_prompt;
pub mod templates;
pub mod turn;

pub use turn::*;
