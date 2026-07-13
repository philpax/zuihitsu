//! Web browsing: an agent asked to look at a page fetches it with `web.markdown(url)`, reports what
//! it found, and records the substance — the page's prose, not its chrome. A later turn probes a
//! concrete detail from the page, so recall of the stored content is exercised, not just relay.
//!
//! The page (served by the fixture web fetcher) is a code-forge repo view for an invented open-source
//! project, wrapped in realistic chrome — a top navigation bar, sign-in prompts, star and fork
//! counts, a file listing, and a sidebar of repository metadata — around a substantive README. The
//! properties tested: the reply reflects the page's actual content, what the agent stores carries the
//! README's prose rather than the chrome artifacts, and a later probe recalls a concrete detail.
//!
//! - [`ReadsAndRecallsAPage`] — a participant shares the project URL and asks what the project is;
//!   the agent fetches and reports. Days later, a different participant asks a specific detail (the
//!   hashing algorithm), which lands in a fresh session and must be recalled from memory.

use std::sync::Arc;

use async_trait::async_trait;
use zuihitsu::Event;

use crate::{
    analysis,
    context::MILLIS_PER_DAY,
    fetch_fixture::PROJECT_URL,
    judge::Judge,
    package::{Bar, Category, ScenarioMeta, Verdict, VerdictKind},
    scenario::Scenario,
    step::{EvalStep, Turn},
};

/// The chrome strings from the fixture project page. None of these belong in memory: they are the
/// navigation, action counts, and sidebar metadata the extraction strips, so a stored entry that
/// carries one is chrome that leaked past the summarize-the-prose practice.
const CHROME_ARTIFACTS: &[&str] = &[
    "Pull requests",
    "Marketplace",
    "Sign in",
    "Star 1,318",
    "Fork 87",
    "Watch 42",
    "Rust 99.1%",
    "Forge, Inc",
];

/// This module's scenarios.
pub fn scenarios() -> Vec<Arc<dyn Scenario>> {
    vec![Arc::new(ReadsAndRecallsAPage)]
}

pub struct ReadsAndRecallsAPage;

#[async_trait]
impl Scenario for ReadsAndRecallsAPage {
    fn meta(&self) -> ScenarioMeta {
        ScenarioMeta {
            name: "reads_and_recalls_a_page".to_owned(),
            category: Category::Writes,
            description: "A participant shares a project page URL and asks what the project is. The \
                          agent fetches it with web.markdown, reports what it found, and records the \
                          substance. Days later, in a fresh session, a different participant asks a \
                          concrete detail from the page, which must be recalled from memory. The \
                          tested properties: the reply reflects the page's actual content, the stored \
                          memory carries the README's prose rather than the page chrome, and the \
                          later probe recalls a concrete detail."
                .to_owned(),
            bar: Bar::Metric { threshold: 0.6 },
        }
    }

    fn needs_retrieval(&self) -> bool {
        // The detail probe lands in a fresh session with the fetched content out of the immediate
        // buffer, so answering it means recalling through `memory.search`.
        true
    }

    fn steps(&self) -> Vec<EvalStep> {
        vec![
            // Turn 1: Priya shares the project URL and asks what it is. The agent should fetch the
            // page via `web.markdown("...")`, read the extracted README, report what Tessera is, and
            // record the substance.
            Turn::new(
                "discord",
                "dev",
                "priya",
                format!(
                    "Someone linked me this the other day — can you take a look and tell me what \
                     this project actually is? {PROJECT_URL}"
                ),
            )
            .with_present(&["priya", "devon"])
            .into(),
            // Unrelated chatter so the fetch-and-report turn is not the last thing in the buffer.
            Turn::new(
                "discord",
                "dev",
                "devon",
                "Nice, I've been looking for something like that. I'll read it properly later.",
            )
            .with_present(&["priya", "devon"])
            .into(),
            // Let the background synthesis settle so descriptions and the index are current.
            EvalStep::Settle,
            // A couple of days pass — a fresh session, the fetched content out of the immediate buffer.
            EvalStep::Advance {
                millis: 2 * MILLIS_PER_DAY,
            },
            // Turn 2: Devon asks a concrete detail from the README — a recall probe that lands in a
            // fresh session, so the agent must recall from memory rather than the buffer.
            Turn::new(
                "discord",
                "dev",
                "devon",
                "Quick question about that storage library you looked at — what does it use to hash \
                 the chunks?",
            )
            .with_present(&["priya", "devon"])
            .into(),
        ]
    }

    async fn assess(&self, events: &[Event], judge: &Judge) -> Vec<Verdict> {
        let entries = analysis::entries(events);

        // The chrome oracle (structural): no committed entry carries a chrome artifact. This pins the
        // fixture's exact chrome strings — nav labels, action counts, sidebar metadata — which are
        // never prose worth recording, so their presence is unambiguous leakage.
        let leaked: Vec<String> = entries
            .iter()
            .flat_map(|entry| {
                CHROME_ARTIFACTS
                    .iter()
                    .filter(|artifact| entry.text.contains(**artifact))
                    .map(|artifact| format!("{artifact:?} in {:?}", entry.memory))
            })
            .collect();
        let no_chrome = leaked.is_empty();

        // The report oracle (metric, judged): the first reply — the agent's report after fetching —
        // reflects what Tessera actually is, rather than a hedge or a description of the page's UI.
        let first_reply = analysis::agent_replies(events)
            .first()
            .copied()
            .unwrap_or_default()
            .to_owned();
        let report_evidence = format!(
            "A participant shared a link to a project page and asked what the project is. After \
             fetching the page, the agent replied:\n\"{first_reply}\""
        );
        let report_reflects = judge
            .assess(
                "The reply describes what the Tessera project is from the page's content — a \
                 content-addressed file storage library for Rust that deduplicates data by chunking \
                 files and addressing chunks by their content hash. A reply that only describes the \
                 web page's interface, hedges without saying what the project is, or claims it could \
                 not read the page does not count.",
                &report_evidence,
            )
            .await;

        // The recall oracle (metric, judged): the final reply recalls a concrete detail from the
        // README — the hashing algorithm (BLAKE3) — retrieved from memory in a fresh session.
        let last_reply = analysis::last_agent_reply(events).unwrap_or_default();
        let recall_evidence = format!(
            "Days later, in a fresh session, a different participant asked what the storage library \
             uses to hash its chunks. The agent replied:\n\"{last_reply}\""
        );
        let recall_detail = judge
            .assess(
                "The reply correctly identifies the hashing algorithm as BLAKE3 (the detail stated \
                 in the project's README). A reply that names a different algorithm, or that admits \
                 it does not recall the detail, does not count.",
                &recall_evidence,
            )
            .await;

        vec![
            Verdict::metric(
                "the stored memory carries the README's prose, not page chrome",
                no_chrome,
                if no_chrome {
                    "no committed entry carried a chrome artifact".to_owned()
                } else {
                    format!("chrome leaked into memory: {leaked:?}")
                },
            ),
            Verdict::metric(
                "recorded at least one substantive entry about the project",
                entries.iter().any(|entry| !entry.text.trim().is_empty()),
                if entries.iter().any(|entry| !entry.text.trim().is_empty()) {
                    "the agent recorded at least one entry about the fetched project"
                } else {
                    "the agent recorded no entries about the fetched project"
                },
            ),
            Verdict::from_judge_outcome(
                "the report reply reflects the page's actual content",
                VerdictKind::Metric,
                report_reflects,
            ),
            Verdict::from_judge_outcome(
                "the recall reply retrieves the concrete detail (BLAKE3) from memory",
                VerdictKind::Metric,
                recall_detail,
            ),
        ]
    }
}
