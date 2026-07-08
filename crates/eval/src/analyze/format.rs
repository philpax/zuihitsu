//! Shared formatting helpers for the analyze views.

use std::collections::BTreeMap;

/// Render coinage locations grouped by scenario with the run indices under each, e.g.
/// `infers_link_from_content #0, #2; other_scenario #1`.
pub(crate) fn render_locations(locations: &[(String, u32)]) -> String {
    let mut by_scenario: BTreeMap<&str, Vec<u32>> = BTreeMap::new();
    for (scenario, run) in locations {
        by_scenario.entry(scenario.as_str()).or_default().push(*run);
    }
    by_scenario
        .into_iter()
        .map(|(scenario, runs)| {
            let indices: Vec<String> = runs.iter().map(|run| format!("#{run}")).collect();
            format!("{scenario} {}", indices.join(", "))
        })
        .collect::<Vec<_>>()
        .join("; ")
}

/// The plural suffix for a count: empty for one, `s` otherwise.
pub(crate) fn plural(count: usize) -> &'static str {
    if count == 1 { "" } else { "s" }
}

/// Clip to `limit` characters (counting chars, not bytes, so it never splits one), noting how many
/// were dropped. `limit == 0` means no clipping — the full text.
pub(crate) fn trunc(text: &str, limit: usize) -> String {
    let text = text.trim();
    if limit == 0 || text.chars().count() <= limit {
        return text.to_owned();
    }
    let kept: String = text.chars().take(limit).collect();
    let dropped = text.chars().count() - limit;
    format!("{kept}… [+{dropped} chars]")
}

pub(crate) fn join_or_none(names: &[&str]) -> String {
    if names.is_empty() {
        "none".to_owned()
    } else {
        names.join(", ")
    }
}
