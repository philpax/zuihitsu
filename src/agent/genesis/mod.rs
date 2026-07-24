//! Genesis and boot at the log level. An agent is created by rolling out the first events of its
//! log; boot branches on whether that rollout completed (spec §Initialization).
//!
//! Genesis is **idempotent**: re-driving it emits only the events that are missing, keyed on
//! content-stable identities (templates on `(name, version)`, relations and config on their key,
//! `self` on its unique name) rather than on freshly-minted ULIDs. So an interrupted creation
//! resumes cleanly, and `GenesisCompleted`'s `manifest_hash` — computed over content, not minted
//! ids — is stable across resumes. Boot keys off the presence of `GenesisCompleted`, never log
//! emptiness, so a crash mid-genesis is never mistaken for a born agent.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::{
    InstanceFeatures, TemplateStatus,
    agent::templates::{LatestRegistration, default_supersedes, latest_registrations},
    clock::Clock,
    event::{Cardinality, EventPayload, EventSource, PromptTemplateName, Teller, Visibility},
    ids::{EntryId, MemoryId, MemoryName, Seq},
    settings::{Settings, compaction_budget_for},
    store::{Store, StoreError},
    vocabulary::{RelationName, TagName},
};

mod seed;

use seed::{default_templates, manifest_hash, seed_relations, seed_tags};

/// The seed identity an operator provides at creation: a name for the agent, a one-line persona,
/// and optional seed disposition entries. A freshly-born agent knows nothing else — genesis seeds
/// no `created_by` link and no facts about anyone (spec §Initialization).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SeedSelf {
    pub agent_name: String,
    pub persona: String,
    pub seed_entries: Vec<String>,
}

/// What boot finds in the log. Boot branches on this, not on emptiness.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum GenesisStatus {
    /// No events — direct the operator to create the agent.
    Empty,
    /// Events present but no `GenesisCompleted` — an interrupted genesis to re-drive.
    Incomplete,
    /// `GenesisCompleted` present — a born agent, ready to serve.
    Complete,
}

/// The outcome of a rollout.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Rollout {
    /// Genesis was already complete; nothing was emitted.
    AlreadyComplete,
    /// Genesis ran, emitting this many events (the full sequence on a fresh log, or just the
    /// missing tail when resuming an interrupted one).
    Created { events_emitted: usize },
}

/// The counts of a template reconciliation, reported distinctly so a boot log distinguishes a
/// backfilled name from an auto-tracked upgrade from a curated surface held back.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TemplateReconciliation {
    /// Names never registered on this log, now registered at the build default (a template the
    /// agent's genesis predates).
    pub backfilled: usize,
    /// Default-tracking names (their latest registration is [`EventSource::Bootstrap`]) whose build
    /// default is newer, now advanced to it.
    pub upgraded: usize,
    /// Operator-curated names (their latest registration is operator-sourced) with a newer build
    /// default available, left untouched — the operator adopts them explicitly (the console badge and
    /// `debug upgrade-prompts --force`).
    pub held_as_curated: usize,
}

/// Reconcile the build's default templates against the log at every boot of a born agent, treating
/// the latest registration's envelope source as the changed-by-operator signal:
///
/// - A name never registered is backfilled at the build default — a newer build can introduce a
///   template name an existing agent's genesis predates (a maintenance pass's synthesis prompt, say),
///   and without this the subsystem reading that template skips silently forever.
/// - A default-tracking name (its latest registration is [`EventSource::Bootstrap`], an unchanged
///   default) auto-tracks the build: when the build default is strictly newer it is registered, so a
///   changed default body reaches the agent without operator action. A version equal to or below the
///   log's latest is left alone — a re-register is a no-op, and a log born under a newer build is
///   never downgraded.
/// - An operator-curated name (its latest registration is operator-sourced) is sovereign and never
///   auto-touched, regardless of content. A newer build default is surfaced as upgradeable (counted
///   in `held_as_curated`) rather than applied, so adoption stays the operator's explicit choice.
///
/// Every registration this emits carries [`EventSource::Bootstrap`], keeping default-tracking names
/// default-tracking. The whole set commits as one atomic append.
pub fn reconcile_templates(
    store: &mut dyn Store,
    clock: &dyn Clock,
    features: &InstanceFeatures,
) -> Result<TemplateReconciliation, StoreError> {
    let assessments = assess_templates(store, features)?;
    let mut to_emit: Vec<EventPayload> = Vec::new();
    let mut counts = TemplateReconciliation::default();
    for assessment in &assessments {
        if assessment.is_absent() {
            to_emit.push(EventPayload::prompt_template_registered(
                assessment.name,
                assessment.default_version,
                assessment.default_body.clone(),
            ));
            counts.backfilled += 1;
        } else if !assessment.is_curated() {
            if assessment.default_is_newer() {
                to_emit.push(EventPayload::prompt_template_registered(
                    assessment.name,
                    assessment.default_version,
                    assessment.default_body.clone(),
                ));
                counts.upgraded += 1;
            }
        } else if assessment.upgrade_available() {
            counts.held_as_curated += 1;
        }
    }
    if !to_emit.is_empty() {
        store.append(clock.now(), EventSource::Bootstrap, to_emit)?;
    }
    if counts != TemplateReconciliation::default() {
        tracing::info!(
            backfilled = counts.backfilled,
            upgraded = counts.upgraded,
            held_as_curated = counts.held_as_curated,
            "reconciled the build's default templates against the log"
        );
    }
    Ok(counts)
}

/// The status of each build-default template name against the log, for the console's prompt surface
/// (`GET /control/prompt-status`): whether the name is a curated (operator-edited) surface, the
/// build's newest default version, and whether a newer default is available for a curated surface to
/// adopt. A default-tracking name never reports `upgrade_available` — the boot reconcile has already
/// advanced it to the build default.
pub fn template_statuses(
    store: &dyn Store,
    features: &InstanceFeatures,
) -> Result<Vec<TemplateStatus>, StoreError> {
    Ok(assess_templates(store, features)?
        .into_iter()
        .map(|assessment| TemplateStatus {
            name: assessment.name,
            latest_version: assessment.latest_version().unwrap_or(0),
            curated: assessment.is_curated(),
            default_version: assessment.default_version,
            upgrade_available: assessment.upgrade_available(),
        })
        .collect())
}

/// One build-default template name assessed against the log: its build default (version and body)
/// beside the log's latest registration for the name, if any. The shared classification the boot
/// reconcile, the console status read, and the offline `debug upgrade-prompts` command all derive
/// from. There is exactly one entry per build-default name.
pub struct TemplateAssessment {
    pub name: PromptTemplateName,
    pub default_version: u32,
    pub default_body: String,
    latest: Option<LatestRegistration>,
}

impl TemplateAssessment {
    /// The name has never been registered on the log — the reconcile backfills it.
    pub fn is_absent(&self) -> bool {
        self.latest.is_none()
    }

    /// The highest version registered for the name, or `None` when it is absent.
    pub fn latest_version(&self) -> Option<u32> {
        self.latest
            .as_ref()
            .map(|registration| registration.version)
    }

    /// The latest registration is operator-sourced — a curated surface the reconcile never
    /// auto-touches. A default-tracking name (Bootstrap-latest) and an absent name are both `false`.
    pub fn is_curated(&self) -> bool {
        self.latest
            .as_ref()
            .is_some_and(|registration| registration.source != EventSource::Bootstrap)
    }

    /// The build default is strictly newer than the latest registration — the condition for
    /// advancing a default-tracking name.
    pub fn default_is_newer(&self) -> bool {
        self.latest
            .as_ref()
            .is_some_and(|registration| self.default_version > registration.version)
    }

    /// A newer or divergent build default a curated surface could adopt. Always `false` for a
    /// default-tracking or absent name.
    pub fn upgrade_available(&self) -> bool {
        self.is_curated()
            && self.latest.as_ref().is_some_and(|registration| {
                default_supersedes(self.default_version, &self.default_body, registration)
            })
    }
}

/// Fold the log's latest registration per name against the build defaults, yielding one assessment
/// per build-default name.
pub fn assess_templates(
    store: &dyn Store,
    features: &InstanceFeatures,
) -> Result<Vec<TemplateAssessment>, StoreError> {
    let latest = latest_registrations(store)?;
    Ok(default_templates(features)
        .into_iter()
        .map(|template| TemplateAssessment {
            name: template.name,
            default_version: template.version,
            default_body: template.body,
            latest: latest.get(&template.name).cloned(),
        })
        .collect())
}

/// Classify the log for boot.
pub fn status(store: &dyn Store) -> Result<GenesisStatus, StoreError> {
    let events = store.read_from(Seq::ZERO)?;
    if events.is_empty() {
        Ok(GenesisStatus::Empty)
    } else if events.iter().any(is_genesis_completed) {
        Ok(GenesisStatus::Complete)
    } else {
        Ok(GenesisStatus::Incomplete)
    }
}

/// Roll out genesis idempotently: emit the build's default templates, seed relations, default
/// config, and the seed `self`, skipping anything already present, then `GenesisCompleted`. The
/// whole tail commits as one atomic append.
///
/// The scaffold's feature-gated dotpoints are baked into the log here (the features passed in
/// decide which guidance persists), so feature-gating the scaffold is a genesis-time decision: a
/// turn reads the baked template verbatim, never re-running `default_templates`. The Lua
/// registration and API reference, by contrast, read the running binary's features fresh each turn.
pub fn rollout(
    store: &mut dyn Store,
    clock: &dyn Clock,
    seed: &SeedSelf,
    context_length: Option<u32>,
    features: &InstanceFeatures,
) -> Result<Rollout, StoreError> {
    let existing = store.read_from(Seq::ZERO)?;
    if existing.iter().any(is_genesis_completed) {
        tracing::debug!("genesis already complete; nothing to roll out");
        return Ok(Rollout::AlreadyComplete);
    }

    let mut templates_present: BTreeSet<(PromptTemplateName, u32)> = BTreeSet::new();
    let mut relations_present: BTreeSet<String> = BTreeSet::new();
    let mut tags_present: BTreeSet<String> = BTreeSet::new();
    let mut config_present = false;
    let mut self_present = false;
    for event in &existing {
        match &event.payload {
            EventPayload::PromptTemplateRegistered { name, version, .. } => {
                templates_present.insert((*name, *version));
            }
            EventPayload::LinkTypeRegistered { name, .. } => {
                relations_present.insert(name.as_str().to_owned());
            }
            EventPayload::TagCreated { name, .. } => {
                tags_present.insert(name.as_str().to_owned());
            }
            EventPayload::ConfigSet { .. } => {
                config_present = true;
            }
            EventPayload::MemoryCreated { name, .. } if name.is_self() => {
                self_present = true;
            }
            _ => {}
        }
    }

    let templates = default_templates(features);
    let mut to_emit: Vec<EventPayload> = Vec::new();

    for template in &templates {
        if !templates_present.contains(&(template.name, template.version)) {
            to_emit.push(EventPayload::prompt_template_registered(
                template.name,
                template.version,
                template.body.clone(),
            ));
        }
    }

    for relation in seed_relations() {
        if !relations_present.contains(relation.name.as_str()) {
            to_emit.push(EventPayload::LinkTypeRegistered {
                name: relation.name,
                inverse: relation.inverse,
                from_card: relation.from_card,
                to_card: relation.to_card,
                symmetric: relation.symmetric,
                reflexive: relation.reflexive,
                description: relation.description.to_owned(),
            });
        }
    }

    for tag in seed_tags() {
        if !tags_present.contains(tag.name) {
            to_emit.push(EventPayload::tag_created(
                TagName::new(tag.name),
                tag.description,
            ));
        }
    }

    if !config_present {
        // The compaction budget is derived from the model's context window when one is configured;
        // without it (an in-memory or model-less instance), the built-in default stands.
        let mut settings = Settings::default();
        if let Some(context_length) = context_length {
            settings.compaction.token_budget = compaction_budget_for(context_length);
            settings.compaction.context_length = Some(i64::from(context_length));
        }
        to_emit.push(EventPayload::config_set(settings));
    }

    if !self_present {
        let self_id = MemoryId::generate();
        to_emit.push(EventPayload::memory_created(
            self_id,
            MemoryName::new(MemoryName::SELF),
        ));
        // The persona is the agent's charter: a seed content entry, not a description. Entries are
        // immutable and append-only, so the authored voice never drifts, while the self can still
        // evolve as the agent appends further self-observations. The system prompt draws the
        // agent's identity from these entries verbatim, never from the regenerable description.
        for text in std::iter::once(&seed.persona).chain(&seed.seed_entries) {
            to_emit.push(EventPayload::MemoryContentAppended {
                id: self_id,
                entry_id: EntryId::generate(),
                asserted_at: clock.now(),
                occurred_at: None,
                text: text.clone(),
                told_by: Teller::Bootstrap,
                told_in: None,
                visibility: Visibility::Public,
            });
        }
    }

    let template_versions: BTreeMap<String, u32> = templates
        .iter()
        .map(|t| (t.name.as_str().to_owned(), t.version))
        .collect();
    to_emit.push(EventPayload::genesis_completed(
        manifest_hash(seed, &templates),
        template_versions,
    ));

    let events_emitted = to_emit.len();
    store.append(clock.now(), EventSource::Bootstrap, to_emit)?;
    tracing::info!(events_emitted, agent = %seed.agent_name, "rolled out genesis");
    Ok(Rollout::Created { events_emitted })
}

fn is_genesis_completed(event: &crate::event::Event) -> bool {
    matches!(event.payload, EventPayload::GenesisCompleted { .. })
}

/// A build-default prompt template. Bodies are first-pass placeholders; final wording is authored
/// by the build over time (spec §Initialization: prompt content is deferred to the build). A body
/// change bumps `version` — distinct bodies never share a `(name, version)` pair — so an older
/// event's `produced_by` keeps naming the body it was generated under; a body's history lives in
/// version control, not in comments.
struct TemplateDef {
    name: PromptTemplateName,
    version: u32,
    body: String,
}

/// A build-seeded system tag. Like the seed relations, these are build defaults rather than part of
/// the genesis manifest hash, so adding one does not perturb an existing agent's hash.
struct TagDef {
    name: &'static str,
    description: &'static str,
}

struct RelationDef {
    name: RelationName,
    inverse: RelationName,
    from_card: Cardinality,
    to_card: Cardinality,
    symmetric: bool,
    reflexive: bool,
    description: &'static str,
}

#[cfg(test)]
mod tests;
