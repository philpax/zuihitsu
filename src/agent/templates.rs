//! Prompt templates, read from the log.
//!
//! Templates are orchestration config (spec §Initialization → prompt templates): registered at
//! genesis as `PromptTemplateRegistered` events keyed by `(name, version)`, and read back from the
//! log rather than materialized into the graph. The current template for a name is its highest
//! registered version, so updating one is a new registration at a bumped version and old
//! `produced_by` references keep resolving to the version they named.

use std::collections::BTreeMap;

use crate::{
    event::{EventPayload, EventSource, PromptTemplateName},
    ids::Seq,
    store::{Store, StoreError},
};

/// A registered prompt template at a particular version.
pub struct PromptTemplate {
    pub version: u32,
    pub body: String,
}

/// The latest registration of a template name: its highest version, that version's body, and the
/// envelope source that wrote it. The source is the changed-by-operator signal — a name whose latest
/// registration is [`EventSource::Bootstrap`] is an unchanged default that auto-tracks the build,
/// while an [`EventSource::Operator`] latest is a curated surface the boot reconcile never touches.
#[derive(Clone, Debug)]
pub(crate) struct LatestRegistration {
    pub version: u32,
    pub body: String,
    pub source: EventSource,
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

/// The latest registration per template name, keyed by highest version (ties resolving to the later
/// registration in the log, matching [`latest_template`]). The single log scan the reconcile, the
/// console status read, and the offline upgrade command all fold over.
pub(crate) fn latest_registrations(
    store: &dyn Store,
) -> Result<BTreeMap<PromptTemplateName, LatestRegistration>, StoreError> {
    let mut latest: BTreeMap<PromptTemplateName, LatestRegistration> = BTreeMap::new();
    for event in store.read_from(Seq::ZERO)? {
        if let EventPayload::PromptTemplateRegistered {
            name,
            version,
            body,
            ..
        } = event.payload
        {
            let supersedes = latest
                .get(&name)
                .is_none_or(|current| version >= current.version);
            if supersedes {
                latest.insert(
                    name,
                    LatestRegistration {
                        version,
                        body,
                        source: event.source,
                    },
                );
            }
        }
    }
    Ok(latest)
}

/// Whether a build default is a newer or divergent template than what is registered: a strictly
/// higher version, or the same version carrying a different body. Drives the console's upgrade badge
/// for a curated surface. (A default-tracking name's auto-upgrade uses the strict version comparison
/// alone, since the build never changes a body without bumping the version.)
pub(crate) fn default_supersedes(
    default_version: u32,
    default_body: &str,
    registered: &LatestRegistration,
) -> bool {
    default_version > registered.version
        || (default_version == registered.version && default_body != registered.body)
}
