use super::*;

/// A person's connections, recorded as links in one room, are retrieved when asked about them in
/// another — the read side of the relationship graph. The two `knows` edges are established together,
/// then a later room with an empty buffer asks who the person knows, so answering means reading the
/// connections back rather than echoing the live conversation.
pub struct RecallsConnections;

#[async_trait]
impl Scenario for RecallsConnections {
    fn meta(&self) -> ScenarioMeta {
        ScenarioMeta {
            name: "recalls_who_someone_knows".to_owned(),
            category: Category::Relations,
            description:
                "Two of a person's relationships, linked in one room, are recalled when a \
                          different room asks who they know — the agent reads its connections back."
                    .to_owned(),
            bar: Bar::Metric { threshold: 0.5 },
        }
    }

    fn needs_retrieval(&self) -> bool {
        true
    }

    fn steps(&self) -> Vec<EvalStep> {
        vec![
            // Turn 1: two of Dave's relationships come up in passing, for the agent to record as links.
            Turn::new(
                TEST_PLATFORM,
                "team-room",
                "marcus",
                "Dave's bringing a couple of friends along on Friday — Erin, who he's known since \
                 college, and Frank, his buddy from the climbing gym.",
            )
            .into(),
            // Regenerate descriptions and embed, as the background workers would, before the recall room.
            EvalStep::Settle,
            // Turn 2: a different room, an empty buffer — answering means reading Dave's connections back,
            // not echoing the live conversation. The asker is Erin, herself one of Dave's connections, so a
            // reasonable reply may only name Frank (the other connection) and omit Erin, assuming she
            // already knows she is one of Dave's connections.
            Turn::new(
                TEST_PLATFORM,
                "hallway",
                "erin",
                "Hey, who does Dave actually know around here? Trying to get a sense of his crowd.",
            )
            .into(),
        ]
    }

    async fn assess(&self, events: &[Event], judge: &Judge) -> Vec<Verdict> {
        let reply = analysis::last_agent_reply(events).unwrap_or_default();
        let read_links = analysis::link_reader_called(events);

        let evidence = format!(
            "Earlier, in another room, the agent was told that Dave knows two people: Erin (from \
             college) and Frank (from the climbing gym). Later, in a different room with no prior \
             conversation, **Erin herself** asked who Dave knows. The agent replied:\n\"{reply}\""
        );
        let judged = judge
            .assess(
                "The reply identifies Dave's connections: it names Frank. Erin is also one of Dave's \
                 connections, but since Erin is the one asking, the agent may reasonably omit her — \
                 she already knows. Addressing Erin as \"you\" (e.g. \"Dave knows you and Frank\") is \
                 fine but not required. The reply passes if it conveys that Frank is one of Dave's \
                 connections; omitting Erin alone does not fail it.",
                &evidence,
            )
            .await;

        vec![
            verdict_from_judge_outcome("recalls Dave's connections", VerdictKind::Metric, judged),
            Verdict::metric_outcome(
                "reached for a link reader",
                read_links,
                "read the connections back (outgoing/incoming/links, or details' links line)",
                "answered without reading the links back",
            ),
        ]
    }
}
