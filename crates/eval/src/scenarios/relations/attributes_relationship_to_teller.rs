use super::*;

/// A relationship is relayed by one participant; later, a *different* participant asks who is on
/// record and who said so. A correct answer attributes it to the original teller (Erin), not to the
/// one now asking (Marcus) — which is what a link's `told_by` provenance carries. Tests that the
/// provenance is legible when the agent reads the relationship back, rather than collapsing to the
/// current speaker.
pub struct AttributesRelationshipToTeller;

#[async_trait]
impl Scenario for AttributesRelationshipToTeller {
    fn meta(&self) -> ScenarioMeta {
        ScenarioMeta {
            name: "attributes_a_relationship_to_its_teller".to_owned(),
            category: Category::Relations,
            description: "One participant relays a relationship; later a different one asks who said \
                          so. The agent must attribute it to the original teller, not the asker — the \
                          link's told_by provenance."
                .to_owned(),
            bar: Bar::Metric { threshold: 0.5 },
        }
    }

    fn needs_retrieval(&self) -> bool {
        true
    }

    fn steps(&self) -> Vec<EvalStep> {
        vec![
            // Erin relays a relationship — the agent records it, the edge carrying Erin as its teller.
            Turn::new(
                "discord",
                "team-room",
                "erin",
                "Heads up for your notes: Dave's taken Grace under his wing — he's been mentoring her \
                 this quarter.",
            )
            .into(),
            EvalStep::Settle,
            // A *different* participant, in another room, asks who is on record and who said so. The teller
            // (Erin) is not the asker (Marcus), so attributing it correctly means reading the provenance, not
            // defaulting to whoever is speaking now.
            Turn::new(
                "discord",
                "hallway",
                "marcus",
                "I think someone mentioned Dave's mentoring a junior — who's he mentoring, and who told \
                 you about it?",
            )
            .into(),
        ]
    }

    async fn assess(&self, events: &[Event], judge: &Judge) -> Vec<Verdict> {
        let reply = analysis::last_agent_reply(events).unwrap_or_default();

        let evidence = format!(
            "Earlier, Erin (and only Erin) told the agent that Dave is mentoring Grace. Later, in a \
             different room, Marcus — who did not say it — asked who Dave is mentoring and who told the \
             agent about it. The agent replied:\n\"{reply}\""
        );
        let judged = judge
            .assess(
                "The reply both names the mentee (Grace) and attributes the information to Erin — the \
                 one who actually said it. Crediting Marcus (the one now asking), or giving no source \
                 when asked who told it, fails: the point is that the agent tracks who asserted the \
                 relationship, not who is currently speaking.",
                &evidence,
            )
            .await;

        vec![Verdict::from_judge_outcome(
            "attributes the relationship to its teller, not the asker",
            VerdictKind::Metric,
            judged,
        )]
    }
}
