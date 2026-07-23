//! The `upgrade-prompts` command: the offline write that re-registers stale build-default templates
//! against the running binary's defaults, so an operator adopts a changed default body without
//! re-running the agent conversationally.
//!
//! It opens the event log read-write — the agent must be stopped first (the open takes the
//! single-writer log lock and fails while a running agent holds it) — and, like the corrections,
//! appends forward rather than rewriting history. The reconciliation doctrine is honoured per name:
//!
//! - A **default-tracking** name (its latest registration is [`EventSource::Bootstrap`], an unchanged
//!   default) is upgraded to the build default when the build default is newer. Every such upgrade
//!   commits under [`EventSource::Bootstrap`] in one append, so the name stays default-tracking.
//! - An **operator-curated** name (its latest registration is operator-sourced) is held by default —
//!   reported as `held (operator-edited); pass --force to overwrite` — and overwritten only under
//!   `--force`. A forced overwrite registers under [`EventSource::Operator`], the subtlety being that
//!   the surface stays marked *curated*: the operator explicitly chose the new body, so a later boot
//!   still leaves it sovereign rather than auto-tracking it.

use zuihitsu::{
    Clock, EventPayload, EventSource, InstanceFeatures, SqliteStore, Store, SystemClock,
    config::EnvConfig, genesis,
};

use crate::cli::error::CliError;

/// Re-register the build's default templates against the log. Default-tracking names advance to a
/// newer build default in one Bootstrap append; operator-curated names are held unless `force` is set,
/// in which case each is overwritten with the build default under a fresh operator registration.
pub(crate) fn upgrade_prompts(config: &EnvConfig, force: bool) -> Result<(), CliError> {
    let mut store = open_store(config)?;
    // The live serving path builds the instance with the default feature set, so the offline upgrade
    // uses the same defaults — the scaffold body it writes matches what a booted agent would compose.
    let features = InstanceFeatures::default();
    let assessments = genesis::assess_templates(&store, &features).map_err(|source| {
        CliError::UpgradePrompts(format!("could not read the templates: {source}"))
    })?;

    // Default-tracking registrations (backfills and upgrades) ride one Bootstrap append; forced
    // curated overwrites ride a separate Operator append, so each carries its honest source.
    let mut tracking: Vec<EventPayload> = Vec::new();
    let mut curated: Vec<EventPayload> = Vec::new();

    for assessment in &assessments {
        let name = assessment.name;
        let default_version = assessment.default_version;
        if assessment.is_absent() {
            tracking.push(EventPayload::prompt_template_registered(
                name,
                default_version,
                assessment.default_body.clone(),
            ));
            tracing::info!(
                "backfilling {} at v{default_version} (never registered)",
                name.as_str()
            );
        } else if !assessment.is_curated() {
            let latest = assessment.latest_version().unwrap_or(0);
            if assessment.default_is_newer() {
                tracking.push(EventPayload::prompt_template_registered(
                    name,
                    default_version,
                    assessment.default_body.clone(),
                ));
                tracing::info!(
                    "upgrading {} v{latest} -> v{default_version} (default-tracking)",
                    name.as_str()
                );
            } else {
                tracing::info!(
                    "{} is up to date at v{latest} (default-tracking)",
                    name.as_str()
                );
            }
        } else {
            // Operator-curated: sovereign unless the operator explicitly forces the overwrite.
            let latest = assessment.latest_version().unwrap_or(0);
            if assessment.upgrade_available() {
                if force {
                    // Register at the next version under operator source, so it becomes the latest and
                    // the surface stays marked curated (the operator chose this body).
                    let version = latest + 1;
                    curated.push(EventPayload::prompt_template_registered(
                        name,
                        version,
                        assessment.default_body.clone(),
                    ));
                    tracing::info!(
                        "overwriting curated {} v{latest} with the build default v{default_version}, \
                         registered as v{version} (--force; stays operator-curated)",
                        name.as_str()
                    );
                } else {
                    tracing::info!(
                        "{} v{latest} held (operator-edited); the build default is v{default_version} \
                         — pass --force to overwrite",
                        name.as_str()
                    );
                }
            } else {
                tracing::info!(
                    "curated {} v{latest} is up to date with the build default",
                    name.as_str()
                );
            }
        }
    }

    let tracked = tracking.len();
    let overwritten = curated.len();
    if !tracking.is_empty() {
        store
            .append(SystemClock.now(), EventSource::Bootstrap, tracking)
            .map_err(|source| {
                CliError::UpgradePrompts(format!(
                    "could not append the default-tracking upgrades: {source}"
                ))
            })?;
    }
    if !curated.is_empty() {
        store
            .append(SystemClock.now(), EventSource::Operator, curated)
            .map_err(|source| {
                CliError::UpgradePrompts(format!(
                    "could not append the forced overwrites: {source}"
                ))
            })?;
    }

    tracing::info!(
        "upgrade complete: {tracked} default-tracking registration(s), {overwritten} forced \
         overwrite(s). The registrations take effect on the agent's next read."
    );
    Ok(())
}

/// Open the event log read-write, failing (with a running-agent hint) when the single-writer lock is
/// already held.
fn open_store(config: &EnvConfig) -> Result<SqliteStore, CliError> {
    let log_path = config.storage.event_log();
    SqliteStore::open(&log_path).map_err(|source| {
        CliError::UpgradePrompts(format!(
            "could not open the event log at {} for writing (is the agent running?): {source}",
            log_path.display()
        ))
    })
}
