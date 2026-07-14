use super::*;

pub struct Knows;

#[async_trait]
impl Scenario for Knows {
    fn meta(&self) -> ScenarioMeta {
        ScenarioMeta {
            name: "link_people_who_know_each_other".to_owned(),
            category: Category::Relations,
            description: "Told two people are close friends, the agent should record a structured \
                          person-to-person link between them, not only prose. The relation label \
                          is the agent's to choose — the seeded `knows`, or a coinage like \
                          `close_friends` — and a judge assesses whether the chosen label \
                          expresses the told relationship."
                .to_owned(),
            bar: Bar::gating(),
        }
    }

    fn steps(&self) -> Vec<EvalStep> {
        vec![
            Turn::new(
                "discord",
                "team-room",
                "marcus",
                "Two people I'd like you to keep track of: Dave and Erin. They've been close friends \
                 since college and know each other really well.",
            )
            .into(),
        ]
    }

    async fn assess(&self, events: &[Event], judge: &Judge) -> Vec<Verdict> {
        // Any structured person-to-person link satisfies the gate: the label is the agent's to
        // choose (the seeded `knows`, or a coinage such as `close_friends` when the nuance
        // matters). A judge then assesses whether the chosen label expresses the told
        // relationship, guarding against a lenient gate admitting a semantically wrong link.
        let names: std::collections::BTreeMap<_, _> = events
            .iter()
            .filter_map(|event| match &event.payload {
                EventPayload::MemoryCreated { id, name, .. } => Some((*id, name.clone())),
                _ => None,
            })
            .collect();
        let person = |id: &MemoryId| {
            names
                .get(id)
                .is_some_and(|name| name.as_str().starts_with("person/"))
        };
        let person_links: Vec<String> = events
            .iter()
            .filter_map(|event| match &event.payload {
                EventPayload::LinkCreated {
                    from, to, relation, ..
                } if person(from) && person(to) => Some(relation.as_str().to_owned()),
                _ => None,
            })
            .collect();

        let mut verdicts = vec![Verdict::oracle_outcome(
            "linked the two people with a structured relation",
            !person_links.is_empty(),
            "created a person-to-person link between the two memories",
            "recorded the relationship only as prose, no person-to-person link",
        )];
        if !person_links.is_empty() {
            let evidence = format!(
                "The agent was told two people have been close friends since college and know \
                 each other really well. It linked their memories with the relation label(s): {}.",
                person_links.join(", ")
            );
            let judged = judge
                .assess(
                    "The relation label expresses that the two people know each other (a \
                     friendship or acquaintance relation, at any level of closeness — not an \
                     unrelated kind of relation such as employment or mentorship).",
                    &evidence,
                )
                .await;
            verdicts.push(verdict_from_judge_outcome(
                "the link's label expresses the told relationship",
                VerdictKind::Metric,
                judged,
            ));
        }
        verdicts
    }
}
