//! The prompt-template status the console reads (`GET /control/prompt-status`): each build-default
//! template name's registration state against the running build's defaults. Crosses the wire to the
//! console, so it lives here; the main crate computes it from the log and its `default_templates`.

use serde::Serialize;
use zuihitsu_core::event::PromptTemplateName;

/// One prompt template name's status against the build defaults. The console badges a curated surface
/// with a newer default available; the doctrine is that a default-tracking name (Bootstrap-latest)
/// auto-tracks the build at boot, while a curated name (operator-edited) is sovereign and adopts a new
/// default only on the operator's explicit choice.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct TemplateStatus {
    pub name: PromptTemplateName,
    /// The highest version registered on the log for this name (`0` if the name is somehow absent —
    /// which a booted agent's reconcile has already backfilled away).
    pub latest_version: u32,
    /// The latest registration is operator-sourced: a curated surface the boot reconcile never
    /// auto-touches. A default-tracking name is `false`.
    pub curated: bool,
    /// The build's newest default version for this name.
    pub default_version: u32,
    /// A newer or divergent build default is available for a curated surface to adopt. Always `false`
    /// for a default-tracking name, which the boot reconcile has already advanced to the build default.
    pub upgrade_available: bool,
}
