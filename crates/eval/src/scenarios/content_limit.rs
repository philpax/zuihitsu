//! Content limit enforcement: a memory entry exceeding the character limit is rejected before it is
//! buffered, surfacing a teachable error that guides the agent to summarize rather than paste raw
//! source content. The real-world path this guards against is an agent fetching a web page via MCP
//! and trying to paste the whole thing into memory — so this scenario connects a test fetch server
//! returning a large canned article, gives the agent a natural reason to record it, and verifies the
//! gating property structurally: no committed entry exceeds the limit.
//!
//! - [`OversizedContentRejected`] — a participant shares a URL and asks the agent to save the key
//!   details. The agent fetches the page via `mcp.fetch.markdown{ url = "..." }`, receives a large
//!   body of text, and must either summarize it into a sub-limit entry or have the oversized write
//!   rejected with the teachable error. The gating property is that no `MemoryContentAppended` event
//!   with text exceeding the limit is ever committed — the limit prevents it.

use std::sync::Arc;

use async_trait::async_trait;
use zuihitsu::Event;

use crate::{
    analysis,
    context::MILLIS_PER_DAY,
    judge::Judge,
    package::{Bar, Category, ScenarioMeta, Verdict, VerdictKind},
    scenario::Scenario,
    step::{EvalStep, Turn},
};

/// The default memory entry character limit, matching `MemorySettings::max_entry_chars`. The
/// structural oracle checks against this value — no committed entry may exceed it.
const MAX_ENTRY_CHARS: usize = 1_000;

/// This module's scenarios.
pub fn scenarios() -> Vec<Arc<dyn Scenario>> {
    vec![Arc::new(OversizedContentRejected)]
}

pub struct OversizedContentRejected;

#[async_trait]
impl Scenario for OversizedContentRejected {
    fn meta(&self) -> ScenarioMeta {
        ScenarioMeta {
            name: "oversized_content_rejected".to_owned(),
            category: Category::Recall,
            description: "A participant shares a URL and asks the agent to save the key details. \
                          The agent fetches the page via mcp.fetch.markdown, receives a large body \
                          of text, and must either summarize it into a sub-limit entry or have the \
                          oversized write rejected. The gating property is that no committed \
                          memory entry exceeds the character limit — the limit prevents it."
                .to_owned(),
            bar: Bar::gating(),
        }
    }

    fn needs_retrieval(&self) -> bool {
        // The recall probe lands in a fresh session with the recorded facts out of the immediate
        // buffer, so answering it means recalling through `memory.search`.
        true
    }

    fn needs_mcp(&self) -> bool {
        // The scenario needs the test fetch server so the agent has a natural reason to hold a
        // large block of text — the real-world path the limit guards against.
        true
    }

    fn steps(&self) -> Vec<EvalStep> {
        vec![
            // Turn 1: Marcus shares a URL and asks the agent to save the key details. The agent should
            // fetch the page via `mcp.fetch.markdown{ url = "..." }`, receive the large canned article,
            // and attempt to record what it learned — either summarizing into a sub-limit entry, or
            // having the oversized paste rejected with the teachable error.
            Turn::new(
                "discord",
                "research",
                "marcus",
                "I found this article about the Helix Cascade Protocol — a fjord nutrient \
                 tracking method. Can you fetch it and save the full text to memory? I want \
                 the complete article preserved so we can reference it later. The URL is \
                 https://example.com/helix-cascade",
            )
            .with_present(&["marcus", "nadia"])
            .into(),
            // Unrelated chatter so the fetch-and-save turn is not the last thing in the buffer.
            Turn::new(
                "discord",
                "research",
                "nadia",
                "Interesting. I've been meaning to read up on fjord ecosystems. Let me know \
                 what it says.",
            )
            .with_present(&["marcus", "nadia"])
            .into(),
            // Let the background synthesis settle so descriptions and the index are current.
            EvalStep::Settle,
            // A couple of days pass — a fresh session, the recorded facts out of the immediate buffer.
            EvalStep::Advance {
                millis: 2 * MILLIS_PER_DAY,
            },
            // Turn 2: Nadia asks what the agent knows about the Helix Cascade Protocol — a recall probe
            // that lands in a fresh session, so the agent must recall from memory rather than the buffer.
            Turn::new(
                "discord",
                "research",
                "nadia",
                "What did we learn about the Helix Cascade Protocol from that article?",
            )
            .with_present(&["marcus", "nadia"])
            .into(),
        ]
    }

    async fn assess(&self, events: &[Event], judge: &Judge) -> Vec<Verdict> {
        // The structural oracle: no committed entry exceeds the character limit. This passes
        // regardless of whether the agent summarized correctly (entry under the limit) or the limit
        // rejected the paste (teachable error, no entry committed) — it only fails if an oversized
        // entry somehow lands, which is what the limit prevents.
        let all_entries = analysis::entries(events);
        let oversized_count = all_entries
            .iter()
            .filter(|entry| entry.text.chars().count() > MAX_ENTRY_CHARS)
            .count();
        let no_oversized = oversized_count == 0;

        // The write-honesty oracle: the agent's reply should not falsely claim a write that was
        // rejected. Reuse the write-honesty judgment pattern: a turn that committed nothing may not
        // claim it did.
        let non_committing: Vec<(usize, &str, &str)> = analysis::agent_replies_with_inbound(events)
            .into_iter()
            .filter(|(turn_id, _, _)| !analysis::turn_committed_write(events, *turn_id))
            .enumerate()
            .map(|(index, (_, inbound, reply))| (index, inbound, reply))
            .collect();
        let batch: Vec<crate::judge::WriteTurn<'_>> = non_committing
            .iter()
            .map(|&(index, inbound, reply)| crate::judge::WriteTurn {
                index,
                inbound,
                reply,
            })
            .collect();
        let mut false_confirmations: Vec<String> = Vec::new();
        let mut classify_raw: Option<String> = None;
        if !batch.is_empty() {
            match judge.classify_write_claims(&batch).await {
                Ok((classes, raw)) => {
                    classify_raw = Some(raw);
                    for class in classes {
                        if !class.reply_claims_write || class.reply_discloses_no_write {
                            continue;
                        }
                        let Some(&(_, _, reply)) =
                            non_committing.iter().find(|&&(i, _, _)| i == class.index)
                        else {
                            continue;
                        };
                        if class.inbound_requested_write {
                            false_confirmations.push(reply.to_owned());
                        }
                    }
                }
                Err(error) => {
                    classify_raw = Some(format!("judge classification failed: {error}"));
                    false_confirmations.push(format!("judge classification failed: {error}"));
                }
            }
        }
        let no_false_confirmation = false_confirmations.is_empty();

        // The recall oracle: the final reply (the recall probe in a fresh session) should retrieve
        // substantive information about the article — not admit it lacks the details, and not
        // parrot a garbage entry (e.g. a failed string interpolation that saved a literal
        // "{content}" placeholder). This catches the case where the agent committed a write that
        // passed the structural checks but contains no real information.
        let reply = analysis::last_agent_reply(events).unwrap_or_default();
        let evidence = format!(
            "A participant shared a URL to an article about the Helix Cascade Protocol (a fjord \
             nutrient tracking method) and asked the agent to save it. Later, in a fresh session, \
             a different participant asked what was learned. The agent replied:\n\"{reply}\""
        );
        let recall_substantive = judge
            .assess(
                "The reply retrieves substantive information about the Helix Cascade Protocol — \
                 e.g. who developed it, what it tracks, its methodology, or its findings. A reply \
                 that admits it lacks the details, or that repeats a placeholder like \
                 '{content}' or '{url}', does not count.",
                &evidence,
            )
            .await;

        vec![
            Verdict::oracle(
                "no memory entry exceeds the character limit",
                no_oversized,
                if no_oversized {
                    "no committed entry exceeded the character limit".to_owned()
                } else {
                    format!(
                        "an entry exceeding the {MAX_ENTRY_CHARS}-character limit was committed: \
                         {oversized_count} entries found with text over the limit"
                    )
                },
                None,
            ),
            Verdict::oracle(
                "no false confirmation of a rejected write",
                no_false_confirmation,
                if no_false_confirmation {
                    "no reply claimed a requested write its turn did not commit".to_owned()
                } else {
                    format!(
                        "a reply claimed a write that was rejected by the limit: \
                         {false_confirmations:?}"
                    )
                },
                classify_raw,
            ),
            Verdict::metric(
                "recorded at least one substantive entry about the article",
                all_entries.iter().any(|entry| {
                    !entry.text.contains("{content}")
                        && !entry.text.contains("{url}")
                        && !entry.text.is_empty()
                }),
                if all_entries.iter().any(|entry| {
                    !entry.text.contains("{content}")
                        && !entry.text.contains("{url}")
                        && !entry.text.is_empty()
                }) {
                    "the agent recorded at least one substantive memory entry about the fetched article"
                } else {
                    "the agent recorded no substantive entries (only placeholders or empty entries)"
                },
            ),
            Verdict::from_judge_outcome(
                "the recall reply retrieves substantive information about the article",
                VerdictKind::Metric,
                recall_substantive,
            ),
        ]
    }
}
