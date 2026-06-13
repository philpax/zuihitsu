//! Deterministic contextual-brief composition (spec §Contextual briefs).
//!
//! At session start the composer assembles the agent's hot context — the **self** brief, the
//! **current room** (with its tags), and a **per-participant** brief for each present participant —
//! into a block that is frozen into the system prompt for the session. Every fact is filtered
//! through the visibility predicate against the *full, uncapped* present set; the present-set cap
//! governs only which participants get a full block, never what the predicate sees (spec §Present-set
//! cap → invariant). The two sets are kept distinct by construction, so a high-population room can't
//! leak through the cap.
//!
//! Composition reads the graph but runs no model and makes no relevance judgment beyond a
//! deterministic recency ranking, so a leak into a brief is a mechanism bug catchable without
//! inference. The active-context and tag-vocabulary sections (spec §Composition 4–5) and rich
//! active-threads arrive with the compaction/working-set machinery; this cut composes the self,
//! current-context, and per-participant sections, which is what the brief-surface fixtures gate.

use std::fmt::Write as _;

use serde::{Deserialize, Serialize};

use crate::{
    event::Visibility,
    graph::{Graph, GraphError, MemoryView},
    ids::{MemoryId, MemoryName},
    settings::BriefSettings,
    time::{self, Timestamp},
    vocabulary::TagName,
};

use crate::visibility::{self, ClassOf, VisibilityDecision};

/// A failure composing the brief, delegating to the graph beneath it.
#[derive(Debug)]
pub enum BriefError {
    Graph(GraphError),
}

impl std::fmt::Display for BriefError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BriefError::Graph(error) => write!(f, "brief (graph): {error}"),
        }
    }
}

impl std::error::Error for BriefError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            BriefError::Graph(error) => Some(error),
        }
    }
}

impl From<GraphError> for BriefError {
    fn from(error: GraphError) -> BriefError {
        BriefError::Graph(error)
    }
}

/// The session-specific inputs to [`compose`]: who is present, the current room, the working set
/// carried across a compaction seam, and the session's start time. Bundled into a request so the call
/// reads clearly (`compose(graph, settings, &request)`) rather than as a row of bare arguments.
pub struct BriefRequest<'a> {
    /// The full present set — the visibility predicate always resolves against all of it.
    pub present_set: &'a [MemoryId],
    /// The room's `context/*` memory, if any.
    pub current_context: Option<MemoryId>,
    /// Memories carried across a compaction seam (empty otherwise), rendered as active threads.
    pub working_set: &'a [MemoryId],
    /// The session's start time — the reference for the `<upcoming/>` window.
    pub now: Timestamp,
}

/// Compose the contextual brief for the session described by `request`. The predicate always resolves
/// against the full present set; `settings.present_set_cap` bounds only how many participants get a
/// full block, with the remainder collapsed to name-only. The working set is re-filtered through
/// `visible` against the present set like any other block (spec §Compaction → working-set carryover).
pub fn compose(
    graph: &Graph,
    settings: &BriefSettings,
    request: &BriefRequest,
) -> Result<String, BriefError> {
    let &BriefRequest {
        present_set,
        current_context,
        working_set,
        now,
    } = request;
    // The visibility predicate resolves identity over the `same_as` class.
    let class_of = |id| graph.class_id(id).map(|class| class.unwrap_or(id));
    let recent = settings.recent_facts.max(0) as usize;
    let mut out = String::new();

    // 1. Self brief — the agent's own memory in the per-participant shape.
    if let Some(self_memory) = graph.memory_by_name(MemoryName::SELF)? {
        out.push_str("# You\n");
        render_memory_body(
            &mut out,
            graph,
            &self_memory,
            present_set,
            &class_of,
            recent,
        )?;
        out.push('\n');
    }

    // 2. Current context — the room — with its #confidential tag, visible regardless of who is
    //    present (it is a memory-level tag, not a teller-gated entry).
    if let Some(context_id) = current_context
        && let Some(context) = graph.memory_by_id(context_id)?
    {
        let room = visibility::room_display(context.name.as_str());
        if context.tags.contains(&TagName::Confidential) {
            let _ = writeln!(out, "# Current room: {room} (confidential)");
        } else {
            let _ = writeln!(out, "# Current room: {room}");
        }
        if !context.description.is_empty() {
            let _ = writeln!(out, "{}", context.description);
        }
        out.push('\n');
    }

    // 3. Per-participant briefs, ranked by recency and capped. The cap bounds full blocks only; the
    //    predicate above and below always sees the full present set.
    if !present_set.is_empty() {
        out.push_str("# Present\n");
        let cap = settings.present_set_cap.max(0) as usize;
        for (index, participant) in ranked_present(graph, present_set)?.into_iter().enumerate() {
            let Some(memory) = graph.memory_by_id(participant)? else {
                continue;
            };
            if index < cap {
                let _ = writeln!(out, "## {}", memory.name.as_str());
                render_memory_body(&mut out, graph, &memory, present_set, &class_of, recent)?;
            } else {
                // The tail collapses to name-only — still present for the predicate, just not
                // given a full block (spec §Present-set cap).
                let _ = writeln!(out, "- {} (present)", memory.name.as_str());
            }
        }
    }

    // 6. Active threads — the working set carried across a compaction seam: the memories the ending
    //    session touched, re-surfaced so the new session does not lose the thread. Each is rendered
    //    in the per-participant shape, so its facts are re-filtered through `visible` against the new
    //    present set (an aside about a now-present subject is suppressed). Self, the current room, and
    //    present participants are already shown above, so they are skipped to avoid duplication.
    if !working_set.is_empty() {
        let self_id = graph
            .memory_by_name(MemoryName::SELF)?
            .map(|memory| memory.id);
        let mut threads = String::new();
        for &id in working_set {
            if Some(id) == self_id || Some(id) == current_context || present_set.contains(&id) {
                continue;
            }
            let Some(memory) = graph.memory_by_id(id)? else {
                continue;
            };
            let mut body = String::new();
            render_memory_body(&mut body, graph, &memory, present_set, &class_of, recent)?;
            if body.trim().is_empty() {
                continue;
            }
            let _ = writeln!(threads, "## {}", memory.name.as_str());
            threads.push_str(&body);
        }
        if !threads.is_empty() {
            out.push_str("# Active threads\n");
            out.push_str(&threads);
        }
    }

    // 7. Upcoming — near-future calendared items, soonest first, so the agent organically raises
    //    them (spec §Calendar → <upcoming/>). Each occurrence is filtered through `visible` like any
    //    entry — a private aside about an absent person carries its marker, one about a now-present
    //    subject is suppressed — and the list is capped.
    let window_days = settings.upcoming_window_days.max(0);
    let max_items = settings.max_upcoming_items.max(0) as usize;
    if window_days > 0 && max_items > 0 {
        let to = Timestamp::from_millis(
            now.as_millis()
                .saturating_add(window_days * time::MILLIS_PER_DAY),
        );
        let mut lines = Vec::new();
        for (memory, entry) in graph.occurrences_in_window(now, to)? {
            if lines.len() >= max_items {
                break;
            }
            if !visibility::visible(&entry, &memory, present_set, &class_of)? {
                continue;
            }
            let when = entry
                .occurred_sort
                .map_or_else(String::new, time::format_day);
            let mut line = format!("- {when}: {} — {}", memory.name.as_str(), entry.text);
            if entry.visibility != Visibility::Public {
                let teller = graph.teller_display(&entry.told_by)?;
                let room = graph.marker_room(entry.told_in)?;
                line.push(' ');
                line.push_str(&visibility::teller_private_marker(&teller, room.as_ref()));
            }
            lines.push(line);
        }
        if !lines.is_empty() {
            out.push_str("# Upcoming\n");
            for line in lines {
                let _ = writeln!(out, "{line}");
            }
        }
    }

    Ok(out)
}

/// The composed brief plus its derivation: every memory the composer considered and, for each of
/// their entries, the visibility verdict and whether it reached the brief. Re-derived (not stored),
/// since composition is deterministic — this is the console's "how the brief was composed" surface.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BriefTrace {
    /// The composed brief, identical to [`compose`]'s output.
    pub text: String,
    /// One per memory the composer drew from, in composition order.
    pub sections: Vec<BriefSectionTrace>,
}

/// One memory's contribution to the brief: which it is, the role it played, and the fate of each of
/// its entries.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BriefSectionTrace {
    pub kind: SectionKind,
    pub memory: MemoryName,
    /// Only meaningful for a `CurrentRoom` section: whether the room carries `#confidential`.
    pub confidential: bool,
    pub entries: Vec<EntryTrace>,
}

/// The role a memory played in the brief (spec §Composition).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SectionKind {
    /// The agent's own `self` memory.
    SelfBrief,
    /// The current room's `context/*` memory.
    CurrentRoom,
    /// A present participant.
    Participant,
    /// A working-set memory carried across a compaction seam.
    ActiveThread,
}

/// One entry's fate during composition: its text, its declared visibility, the predicate's verdict,
/// and whether — given the verdict and the recency window — it actually reached the brief.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EntryTrace {
    pub text: String,
    pub visibility: Visibility,
    pub decision: VisibilityDecision,
    /// True when the entry both passed the predicate and fell within the recent-facts window.
    pub in_brief: bool,
}

/// Compose the brief and, alongside it, the trace of how it was built: every considered memory and
/// the per-entry verdicts. The text matches [`compose`] exactly; the sections walk the same memories
/// in the same order, recording for each entry why it did or did not surface.
pub fn compose_traced(
    graph: &Graph,
    settings: &BriefSettings,
    request: &BriefRequest,
) -> Result<BriefTrace, BriefError> {
    let text = compose(graph, settings, request)?;
    let class_of = |id| graph.class_id(id).map(|class| class.unwrap_or(id));
    let recent = settings.recent_facts.max(0) as usize;
    let mut sections = Vec::new();

    if let Some(self_memory) = graph.memory_by_name(MemoryName::SELF)? {
        sections.push(section_trace(
            graph,
            &self_memory,
            request.present_set,
            &class_of,
            recent,
            SectionKind::SelfBrief,
        )?);
    }

    if let Some(context_id) = request.current_context
        && let Some(context) = graph.memory_by_id(context_id)?
    {
        let mut section = section_trace(
            graph,
            &context,
            request.present_set,
            &class_of,
            recent,
            SectionKind::CurrentRoom,
        )?;
        section.confidential = context.tags.contains(&TagName::Confidential);
        sections.push(section);
    }

    for participant in ranked_present(graph, request.present_set)? {
        if let Some(memory) = graph.memory_by_id(participant)? {
            sections.push(section_trace(
                graph,
                &memory,
                request.present_set,
                &class_of,
                recent,
                SectionKind::Participant,
            )?);
        }
    }

    let self_id = graph
        .memory_by_name(MemoryName::SELF)?
        .map(|memory| memory.id);
    for &id in request.working_set {
        if Some(id) == self_id
            || Some(id) == request.current_context
            || request.present_set.contains(&id)
        {
            continue;
        }
        if let Some(memory) = graph.memory_by_id(id)? {
            sections.push(section_trace(
                graph,
                &memory,
                request.present_set,
                &class_of,
                recent,
                SectionKind::ActiveThread,
            )?);
        }
    }

    Ok(BriefTrace { text, sections })
}

/// Compose a single participant's brief block, against `present_set`. Used for a mid-session join:
/// the joiner's brief is built against the now-present set and injected as a system message, rather
/// than rebuilding the whole frozen prompt (spec §Mid-conversation joins). Because `present_set`
/// includes the joiner, the subject-guard suppresses asides about them — a subject joining closes
/// the dangerous direction. Empty if the participant is unknown.
pub fn compose_participant(
    graph: &Graph,
    participant: MemoryId,
    present_set: &[MemoryId],
    settings: &BriefSettings,
) -> Result<String, BriefError> {
    let class_of = |id| graph.class_id(id).map(|class| class.unwrap_or(id));
    let recent = settings.recent_facts.max(0) as usize;
    let mut out = String::new();
    if let Some(memory) = graph.memory_by_id(participant)? {
        let _ = writeln!(out, "## {}", memory.name.as_str());
        render_memory_body(&mut out, graph, &memory, present_set, &class_of, recent)?;
    }
    Ok(out)
}

/// Render a memory's body in the per-participant shape: summary, visible recent facts (with the
/// teller-private marker baked in), and key relationships.
fn render_memory_body(
    out: &mut String,
    graph: &Graph,
    memory: &MemoryView,
    present_set: &[MemoryId],
    class_of: &ClassOf,
    recent: usize,
) -> Result<(), BriefError> {
    if !memory.description.is_empty() {
        let _ = writeln!(out, "<summary>{}</summary>", memory.description);
    }

    let facts = visible_recent_facts(graph, memory, present_set, class_of, recent)?;
    if !facts.is_empty() {
        out.push_str("<recent_facts>\n");
        for fact in &facts {
            let _ = writeln!(out, "- {fact}");
        }
        out.push_str("</recent_facts>\n");
    }

    let relationships = relationships(graph, memory.id)?;
    if !relationships.is_empty() {
        out.push_str("<relationships>\n");
        for relationship in &relationships {
            let _ = writeln!(out, "- {relationship}");
        }
        out.push_str("</relationships>\n");
    }

    Ok(())
}

/// A memory's last `recent` content entries that are visible to `present_set`, in commit order, each
/// rendered as text with the inline teller-private marker appended when it is a surviving private
/// entry (resolving its `told_in` room and `#confidential` flag at build time).
fn visible_recent_facts(
    graph: &Graph,
    memory: &MemoryView,
    present_set: &[MemoryId],
    class_of: &ClassOf,
    recent: usize,
) -> Result<Vec<String>, BriefError> {
    let mut facts = Vec::new();
    for entry in graph.class_entries(memory.id)? {
        if !visibility::visible(&entry, memory, present_set, class_of)? {
            continue;
        }
        let mut line = entry.text.clone();
        if entry.visibility != Visibility::Public {
            let teller = graph.teller_display(&entry.told_by)?;
            let room = graph.marker_room(entry.told_in)?;
            line.push(' ');
            line.push_str(&visibility::teller_private_marker(&teller, room.as_ref()));
        }
        facts.push(line);
    }
    let start = facts.len().saturating_sub(recent);
    Ok(facts.split_off(start))
}

/// A memory's key relationships, rendered as `relation: other-handle`, skipping soft-deleted
/// neighbours. The full ranking by recency × type-weight (spec §Per-participant brief) is a later
/// refinement; this lists the live edges touching the memory.
fn relationships(graph: &Graph, id: MemoryId) -> Result<Vec<String>, BriefError> {
    let mut relationships = Vec::new();
    for link in graph.links(id)? {
        let other = if link.from == id { link.to } else { link.from };
        if let Some(memory) = graph.memory_by_id(other)? {
            relationships.push(format!(
                "{}: {}",
                link.relation.as_str(),
                memory.name.as_str()
            ));
        }
    }
    Ok(relationships)
}

/// The present participants ordered for the cap: most-recently-active first (by their latest
/// asserted entry across the merged identity), with the memory id as a deterministic tie-break.
/// Trace one memory's entries: walk its full history (superseded entries included, so a fact
/// filtered for being superseded still appears with its reason), record each entry's visibility
/// verdict, and mark the last `recent` surviving entries as the ones that reached the brief — the
/// same recency window the composer applies.
fn section_trace(
    graph: &Graph,
    memory: &MemoryView,
    present_set: &[MemoryId],
    class_of: &ClassOf,
    recent: usize,
    kind: SectionKind,
) -> Result<BriefSectionTrace, BriefError> {
    let mut entries = Vec::new();
    let mut visible_positions = Vec::new();
    for entry in graph.class_history(memory.id)? {
        let decision = visibility::explain(&entry, memory, present_set, class_of)?;
        if decision.is_visible() {
            visible_positions.push(entries.len());
        }
        entries.push(EntryTrace {
            text: entry.text,
            visibility: entry.visibility,
            decision,
            in_brief: false,
        });
    }
    let keep_from = visible_positions.len().saturating_sub(recent);
    for &position in &visible_positions[keep_from..] {
        entries[position].in_brief = true;
    }
    Ok(BriefSectionTrace {
        kind,
        memory: memory.name.clone(),
        confidential: false,
        entries,
    })
}

fn ranked_present(graph: &Graph, present_set: &[MemoryId]) -> Result<Vec<MemoryId>, BriefError> {
    let mut keyed: Vec<(i64, MemoryId)> = Vec::new();
    for &id in present_set {
        let latest = graph
            .class_entries(id)?
            .iter()
            .map(|entry| entry.asserted_at.as_millis())
            .max()
            .unwrap_or(i64::MIN);
        keyed.push((latest, id));
    }
    keyed.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.0.cmp(&b.1.0)));
    Ok(keyed.into_iter().map(|(_, id)| id).collect())
}

#[cfg(test)]
mod tests {
    //! Contextual-brief composition tests (spec appendix scenarios 2, 14, 21 — the deterministic
    //! `[brief]`/`[predicate]` surface). Each builds a materialized graph, composes a brief for a
    //! present set, and asserts a fact is present or absent — model-free, because composition is
    //! deterministic.
    use crate::{
        brief::{self, BriefRequest},
        event::{EventPayload, Teller, Visibility},
        graph::Graph,
        ids::{EntryId, MemoryId, MemoryName},
        settings::{BriefSettings, Settings},
        store::{MemoryStore, Store},
        time::{CivilDate, TemporalRef, Timestamp},
        vocabulary::TagName,
    };

    /// Compose a brief at the epoch (these deterministic tests don't exercise the time-relative
    /// `<upcoming/>` window unless they plant a future occurrence, so a fixed `now` keeps them stable).
    fn compose_at_epoch(
        graph: &Graph,
        settings: &BriefSettings,
        present_set: &[MemoryId],
        current_context: Option<MemoryId>,
        working_set: &[MemoryId],
    ) -> String {
        brief::compose(
            graph,
            settings,
            &BriefRequest {
                present_set,
                current_context,
                working_set,
                now: Timestamp::from_millis(0),
            },
        )
        .unwrap()
    }

    /// A content append carrying an `occurred_at` (the `appended` helper below leaves it `None`).
    fn appended_at(
        id: MemoryId,
        occurred_at: TemporalRef,
        text: &str,
        told_by: Teller,
        visibility: Visibility,
    ) -> EventPayload {
        EventPayload::MemoryContentAppended {
            id,
            entry_id: EntryId::generate(),
            asserted_at: Timestamp::from_millis(0),
            occurred_at: Some(occurred_at),
            text: text.to_owned(),
            told_by,
            told_in: None,
            visibility,
        }
    }

    /// Build a store, append `payloads`, and materialize a fresh in-memory graph from them.
    fn materialized(payloads: Vec<EventPayload>) -> (MemoryStore, Graph) {
        let mut store = MemoryStore::new();
        store
            .append(Timestamp::from_millis(1_000), payloads)
            .unwrap();
        let mut graph = Graph::open_in_memory().unwrap();
        graph.materialize_from(&store).unwrap();
        (store, graph)
    }

    fn created(id: MemoryId, name: &str) -> EventPayload {
        EventPayload::MemoryCreated {
            id,
            name: MemoryName::new(name),
        }
    }

    fn appended(
        id: MemoryId,
        at_ms: i64,
        text: &str,
        told_by: Teller,
        visibility: Visibility,
    ) -> EventPayload {
        EventPayload::MemoryContentAppended {
            id,
            entry_id: EntryId::generate(),
            asserted_at: Timestamp::from_millis(at_ms),
            occurred_at: None,
            text: text.to_owned(),
            told_by,
            told_in: None,
            visibility,
        }
    }

    #[test]
    fn current_room_brief_shows_confidential_regardless_of_present_set() {
        // Scenario 14: #leads is #confidential. A later session has Phil and Dave but not the teller;
        // the current-context brief still shows confidential — it's a memory-level tag, not teller-gated.
        let leads = MemoryId::generate();
        let phil = MemoryId::generate();
        let dave = MemoryId::generate();
        let (_store, graph) = materialized(vec![
            created(leads, "context/leads"),
            EventPayload::TagCreated {
                name: TagName::new("confidential"),
                description: "confidential room".to_owned(),
            },
            EventPayload::TagAppliedToMemory {
                memory: leads,
                tag: TagName::new("confidential"),
            },
            created(phil, "person/phil"),
            created(dave, "person/dave"),
        ]);

        let out = compose_at_epoch(
            &graph,
            &Settings::default().brief,
            &[phil, dave],
            Some(leads),
            &[],
        );
        assert!(out.contains("Current room: #leads (confidential)"));
    }

    #[test]
    fn an_aside_about_a_present_subject_is_suppressed_in_the_brief() {
        // Scenario 2 (composition half): Erin's private aside about Phil. With Phil present, his brief
        // block renders his public fact but the subject-guard suppresses the aside. (The surfaces-while-
        // absent half and the join injection complete at the join increment.)
        let phil = MemoryId::generate();
        let erin = MemoryId::generate();
        let (_store, graph) = materialized(vec![
            created(phil, "person/phil"),
            created(erin, "person/erin"),
            appended(
                phil,
                1_000,
                "on the platform team",
                Teller::Agent,
                Visibility::Public,
            ),
            appended(
                phil,
                1_100,
                "is being managed out",
                Teller::Participant(erin),
                Visibility::PrivateToTeller,
            ),
        ]);

        let out = compose_at_epoch(&graph, &Settings::default().brief, &[erin, phil], None, &[]);
        assert!(out.contains("on the platform team")); // Phil's block renders
        assert!(!out.contains("is being managed out")); // ...but the aside is suppressed
    }

    #[test]
    fn a_subject_joining_suppresses_asides_about_them() {
        // Scenario 2 (join half): Erin's private aside about Phil. While only Erin is present it is
        // visible (it would surface to her). Phil's join-brief is built against the now-present set
        // {Erin, Phil}, where the subject-guard suppresses it — the dangerous direction is closed.
        let phil = MemoryId::generate();
        let erin = MemoryId::generate();
        let (_store, graph) = materialized(vec![
            created(phil, "person/phil"),
            created(erin, "person/erin"),
            appended(
                phil,
                1_000,
                "is being managed out",
                Teller::Participant(erin),
                Visibility::PrivateToTeller,
            ),
        ]);
        let settings = Settings::default().brief;

        // Before Phil joins (only Erin present): the aside is visible.
        let before = brief::compose_participant(&graph, phil, &[erin], &settings).unwrap();
        assert!(before.contains("is being managed out"));

        // Phil's join-brief, built against {Erin, Phil}: the subject-guard suppresses it.
        let join_brief =
            brief::compose_participant(&graph, phil, &[erin, phil], &settings).unwrap();
        assert!(!join_brief.contains("is being managed out"));
    }

    #[test]
    fn the_working_set_is_re_filtered_against_the_new_present_set() {
        // The working set carried across a compaction is re-filtered through `visible` against the *new*
        // present set, never trusted from the old session: Erin's private aside about Phil surfaces in
        // active threads while only Erin is present, but is suppressed once Phil is present at the new
        // segment boundary (the safety property fixture 22 guards, at the deterministic level).
        let phil = MemoryId::generate();
        let erin = MemoryId::generate();
        let (_store, graph) = materialized(vec![
            created(phil, "person/phil"),
            created(erin, "person/erin"),
            appended(
                phil,
                1_000,
                "is being managed out",
                Teller::Participant(erin),
                Visibility::PrivateToTeller,
            ),
        ]);
        let settings = Settings::default().brief;

        // Phil is in the working set. With only Erin present, the aside is visible in active threads.
        let only_erin = compose_at_epoch(&graph, &settings, &[erin], None, &[phil]);
        assert!(only_erin.contains("# Active threads"));
        assert!(only_erin.contains("is being managed out"));

        // With Phil present at the new boundary, the aside is suppressed — the working-set copy is
        // re-filtered against {Erin, Phil} just like any other block.
        let with_phil = compose_at_epoch(&graph, &settings, &[erin, phil], None, &[phil]);
        assert!(!with_phil.contains("is being managed out"));
    }

    #[test]
    fn the_present_set_cap_does_not_narrow_the_predicate() {
        // Scenario 21: with the present-set cap set to 1, Dave is present but ranks below the cap (only a
        // name-only entry, no full block). A fact on Phil (in the cap, rendered) excludes Dave; the
        // exclude must still fire, because the predicate resolves against the full present set — not the
        // capped one. Told by Phil himself, so the subject-guard does not also suppress it: the exclude
        // is the only thing gating it, isolating the cap-vs-predicate separation.
        let phil = MemoryId::generate();
        let dave = MemoryId::generate();
        let (_store, graph) = materialized(vec![
            created(phil, "person/phil"),
            created(dave, "person/dave"),
            // Phil has the more recent activity, so he ranks into the cap of 1; Dave falls below it.
            appended(
                phil,
                2_000,
                "joined the climbing gym",
                Teller::Participant(phil),
                Visibility::Public,
            ),
            EventPayload::MemoryContentAppended {
                id: phil,
                entry_id: EntryId::generate(),
                asserted_at: Timestamp::from_millis(2_100),
                occurred_at: None,
                text: "thinking of leaving, keep it from Dave".to_owned(),
                told_by: Teller::Participant(phil),
                told_in: None,
                visibility: Visibility::Exclude(vec![dave]),
            },
        ]);

        let mut settings = Settings::default().brief;
        settings.present_set_cap = 1;
        let out = compose_at_epoch(&graph, &settings, &[phil, dave], None, &[]);

        assert!(out.contains("joined the climbing gym")); // Phil's block renders (in the cap)
        assert!(out.contains("person/dave (present)")); // Dave is present but below the cap (name-only)
        // The exclude fires because Dave is in the full present set, despite ranking below the cap.
        assert!(!out.contains("keep it from Dave"));
    }

    #[test]
    fn upcoming_block_lists_near_future_items_within_the_window() {
        // now = epoch (day 0). The dentist on day 3 falls in the default 7-day window; the far review on
        // day 30 does not.
        let dentist = MemoryId::generate();
        let far = MemoryId::generate();
        let (_store, graph) = materialized(vec![
            created(dentist, "event/dentist"),
            appended_at(
                dentist,
                TemporalRef::Day(CivilDate("1970-01-04".into())),
                "cleaning",
                Teller::Agent,
                Visibility::Public,
            ),
            created(far, "event/far"),
            appended_at(
                far,
                TemporalRef::Day(CivilDate("1970-01-31".into())),
                "annual review",
                Teller::Agent,
                Visibility::Public,
            ),
        ]);
        let out = compose_at_epoch(&graph, &Settings::default().brief, &[], None, &[]);
        assert!(out.contains("# Upcoming"));
        assert!(out.contains("cleaning"));
        assert!(!out.contains("annual review")); // beyond the 7-day window
    }

    #[test]
    fn upcoming_respects_the_subject_guard() {
        // A private aside about Phil with a near-future occurrence, told by Erin: visible in <upcoming/>
        // while only Erin is present, suppressed once Phil (its subject) is present.
        let phil = MemoryId::generate();
        let erin = MemoryId::generate();
        let (_store, graph) = materialized(vec![
            created(phil, "person/phil"),
            created(erin, "person/erin"),
            appended_at(
                phil,
                TemporalRef::Day(CivilDate("1970-01-04".into())),
                "farewell lunch",
                Teller::Participant(erin),
                Visibility::PrivateToTeller,
            ),
        ]);
        let settings = Settings::default().brief;
        let only_erin = compose_at_epoch(&graph, &settings, &[erin], None, &[]);
        assert!(only_erin.contains("farewell lunch"));
        let with_phil = compose_at_epoch(&graph, &settings, &[erin, phil], None, &[]);
        assert!(!with_phil.contains("farewell lunch"));
    }
}
