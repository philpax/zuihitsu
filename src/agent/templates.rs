//! Prompt templates, read from the log.
//!
//! Templates are orchestration config (spec §Initialization → prompt templates): registered at
//! genesis as `PromptTemplateRegistered` events keyed by `(name, version)`, and read back from the
//! log rather than materialized into the graph. The current template for a name is its highest
//! registered version, so updating one is a new registration at a bumped version and old
//! `produced_by` references keep resolving to the version they named.

use crate::{
    event::{EventPayload, PromptTemplateName},
    ids::Seq,
    store::{Store, StoreError},
};

/// A registered prompt template at a particular version.
pub struct PromptTemplate {
    pub version: u32,
    pub body: String,
}

/// The highest-versioned template registered under `name`, or `None` if there is none.
pub fn latest_template(
    store: &dyn Store,
    name: PromptTemplateName,
) -> Result<Option<PromptTemplate>, StoreError> {
    let mut latest: Option<PromptTemplate> = None;
    for event in store.read_from(Seq::ZERO)? {
        if let EventPayload::PromptTemplateRegistered {
            name: registered,
            version,
            body,
            ..
        } = event.payload
            && registered == name
            && latest
                .as_ref()
                .is_none_or(|current| version >= current.version)
        {
            latest = Some(PromptTemplate { version, body });
        }
    }
    Ok(latest)
}
