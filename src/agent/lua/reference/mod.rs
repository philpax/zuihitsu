//! The agent-facing Lua API as a typed catalogue, rendered into the system prompt's API description.
//! Kept beside the functions installed in [`crate::agent::lua::Session::execute`] so the prompt and the
//! implementation cannot drift.

mod calendar;
mod context;
mod links;
mod memory;
mod tags;
mod web;

#[cfg(test)]
mod tests;

use crate::{
    InstanceFeatures,
    agent::api_doc::{ApiEntry, render},
};

/// The agent-facing Lua API, as a typed catalogue. Defined here, beside the functions installed in
/// [`crate::agent::lua::Session::execute`], so the prompt and the implementation cannot drift: changing a function
/// means changing its entry right next to it. Rendered into the system prompt's API description
/// through [`render`] — the same renderer MCP tools project through (spec §System
/// prompt → API description).
///
/// The catalogue is filtered by `features`: a disabled feature's entries are omitted, so the prompt's
/// "What you can do" section never describes a function the runtime rejects. This is the second of the
/// three gates (Lua registration, API reference, scaffold) that must stay in lockstep.
pub fn api_reference(features: &InstanceFeatures) -> Vec<ApiEntry> {
    let mut entries = memory::entries();
    if features.linking {
        entries.extend(links::handle_methods());
    }
    if features.merging {
        entries.extend(memory::merge_entries());
    }
    if features.tagging {
        entries.extend(tags::handle_methods());
        entries.extend(tags::module_entries());
    }
    if features.linking {
        entries.extend(links::module_entries());
    }
    entries.extend(context::entries());
    if features.transcripts {
        entries.extend(context::convo_entries());
    }
    if features.calendar {
        entries.extend(calendar::entries());
    }
    if features.browsing {
        entries.extend(web::entries());
    }
    entries.extend(memory::block_entries());
    entries.extend(memory::turn_entries());
    entries
}

/// Render [`api_reference`] as the system prompt's API-description block, filtered by `features`.
pub fn render_api_reference(features: &InstanceFeatures) -> String {
    render(&api_reference(features))
}
