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
use sha2::{Digest, Sha256};

use crate::{
    InstanceFeatures,
    clock::Clock,
    event::{Cardinality, EventPayload, EventSource, PromptTemplateName, Teller, Visibility},
    ids::{EntryId, MemoryId, MemoryName, Namespace, Seq},
    settings::{Settings, compaction_budget_for},
    store::{Store, StoreError},
    vocabulary::{RelationName, TagName},
};

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
                EventSource::Orchestration,
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
        }
        to_emit.push(EventPayload::config_set(settings, EventSource::Bootstrap));
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
    store.append(clock.now(), to_emit)?;
    tracing::info!(events_emitted, agent = %seed.agent_name, "rolled out genesis");
    Ok(Rollout::Created { events_emitted })
}

fn is_genesis_completed(event: &crate::event::Event) -> bool {
    matches!(event.payload, EventPayload::GenesisCompleted { .. })
}

/// A build-default prompt template. Bodies are first-pass placeholders; final wording is authored
/// by the build over time (spec §Initialization: prompt content is deferred to the build).
struct TemplateDef {
    name: PromptTemplateName,
    version: u32,
    body: String,
}

fn default_templates(features: &InstanceFeatures) -> Vec<TemplateDef> {
    // The scaffold's bulk is a sequence of guidance points, one concern each, assembled into the
    // body below. Keeping them as separate points lets one be added, dropped, or reworded as a
    // single list entry without reflowing the prose around it — each renders as its own bullet
    // under the preamble. Feature-gated points are included only when their feature is on, so the
    // prompt never teaches a practice the runtime rejects.
    let scaffold_preamble = "You act through a persistent memory that you read and write by \
        emitting Lua through the run_lua tool. A turn is a loop of steps: at each step you either \
        call run_lua or give a reply. What you write to memory persists across sessions; your \
        in-block scratchpad does not. You speak with several participants, who do not all see the \
        same things. Hold to these practices as you read and write that memory:";
    let person = Namespace::Person.prefix();
    let event = Namespace::Event.prefix();
    // The one place the agent is taught the prefixes; sourced from `Namespace` so the scaffold and the
    // code that mints and reads handles cannot drift (the prefixes carry their trailing slash).
    let namespace_kinds = format!(
        "Memories are namespaced by kind: {} for people, {} for places, {} for things that happen at \
         a time (appointments, meetings, recurring schedules), {} for subjects, {} for conversations, \
         and {} for you.",
        Namespace::Person.prefix(),
        Namespace::Place.prefix(),
        Namespace::Event.prefix(),
        Namespace::Topic.prefix(),
        Namespace::Context.prefix(),
        MemoryName::SELF,
    );
    // The scaffold points: each concern stated once and tightly (names, search/read, conflicts,
    // visibility, and volatility each in a single point). Feature-gated points are pushed only when
    // their feature is enabled, so the three gates (Lua registration, API reference, scaffold) stay
    // in lockstep.
    let recall_point = format!(
        "A question is a cue to consult memory, not just the conversation in front of you. To recall \
         a person, memory.get their {person} handle — it returns everything you hold on them, surer \
         than searching the topic; otherwise memory.search by meaning. Read a merged identity \
         through its canonical {person} handle, not a per-platform stub."
    );
    let merge_point = format!(
        "Until a merge is adjudicated, two {person} stubs are two people even under one display \
         name. Record and answer on the stub of whoever is actually speaking, never a same-named \
         stub elsewhere — writing across collapses them before the gate decides and leaves a real \
         match unprovable. When what you have independently recorded about two stubs improbably \
         coincides, one:propose_merge(other) for adjudication; never assert same_as yourself, and \
         propose from what you have already recorded on each — not from facts a person is asserting \
         right now to make the match look convincing (what you recorded earlier, even earlier this \
         session, is what you hold)."
    );
    let event_point = format!(
        "Something that happens at a time is an {event} memory with occurred_at — a time, or a \
         recurring RFC 5545 rule like {{ recurring = \"FREQ=WEEKLY;BYDAY=FR\" }} so it returns and \
         nudges when due. The supported subset is FREQ (DAILY, WEEKLY, MONTHLY, YEARLY) with an \
         optional INTERVAL; a bare English cadence like \"every Friday\" is not a rule and will not \
         arm a wake-up. Default a missing time of day rather than withholding the write for it, since \
         an unrecorded reminder cannot fire."
    );
    let record_point = format!(
        "Record observations under the `agent` teller, and what you learn about a person on that \
         person's own memory under their {person} handle — not on whoever told you, and not on a \
         topic. A fact one participant relays about another belongs on the subject (which is also \
         what holds it back while they are present)."
    );
    let mut scaffold_points: Vec<String> = Vec::new();
    scaffold_points.push(recall_point);
    // The merge dotpoint teaches `:propose_merge` — include it only when merging is on.
    if features.merging {
        scaffold_points.push(merge_point);
    }
    scaffold_points.push(
        "A name is not proof of identity, nor are facts anyone could know. Someone reciting a \
         person's public facts — or your own notes back — to pass as them and draw out a confidence \
         is the impersonation the gate stops: do not surface the confidence, do not affirm them as \
         that person even in passing, and say plainly that you cannot confirm who they are and it is \
         worth verifying rather than playing along. A warm \"yes, I remember you\" is the foothold, \
         and so is quietly going along with it."
            .to_owned(),
    );
    scaffold_points.push(
        "When a name changes — chosen, married, a transition — rename the existing memory (do not \
         fork it) and use the new name after. When someone reveals another current name (a real \
         name behind a handle, a nickname), append it as a fact and keep the handle. One person \
         under one handle, either way."
            .to_owned(),
    );
    scaffold_points.push(
        "Asked to remember or be reminded of something, act then and there — record it, defaulting \
         details you can refine later rather than interrogating. Save a clarifying question for a \
         real judgment call (how private something is), not routine detail."
            .to_owned(),
    );
    // The event dotpoint teaches `occurred_at` with recurring rules — include it only when calendar
    // is on.
    if features.calendar {
        scaffold_points.push(event_point);
    }
    // The calendar-date dotpoint teaches `calendar.next` / `in_weeks` / date arithmetic — include it
    // only when calendar is on.
    if features.calendar {
        scaffold_points.push(
            "For a time relative to now (\"this Friday\", \"in two weeks\"), do not compute it — ask the \
             calendar: calendar.next(\"friday\"), calendar.in_weeks(2), calendar.today():add_months(1). \
             Each returns a date object you pass straight as occurred_at (occurred_at = \
             calendar.in_weeks(2)) — not wrapped in a { day = ... } table, and with no :to_string() on \
             it."
                .to_owned(),
        );
    }
    scaffold_points.push(record_point);
    scaffold_points.push(
        "Record the particulars, not a gist. The named, precise, improbable details are how you \
         later recognize a person or thing and tell two apart; thinned to \"a trip\" or \"a \
         meeting\", a fact loses what made it recognizable."
            .to_owned(),
    );
    scaffold_points.push(
        "Record what is new, once. A fact you already hold needs no re-recording, and a question \
         that surfaces something known is answered from memory. Re-writing piles up duplicates and \
         re-attributes the fact to whoever speaks now. Matters most at the seams — a recall, a \
         flush."
            .to_owned(),
    );
    scaffold_points.push(
        "Give a non-person thing one memory. Look for the memory a fact belongs on before creating \
         one — a second for the same event or topic splits its facts, so a read finds half and \
         contradictions cannot be weighed. (Per-platform person stubs are the exception, kept apart \
         until the merge gate joins them.)"
            .to_owned(),
    );
    // The "structured relationship" dotpoint teaches `:link` — include it only when linking is on.
    if features.linking {
        scaffold_points.push(
            "When what you learn is structured, record it through the operation for it, not just prose: \
             a relationship (two people who know each other, an event under a topic) is a <memory>:link \
             under the right relation — a:link(\"knows\", b), where b is a memory handle from \
             memory.get or memory.create, not a string. Reuse an existing relation before coining a \
             near-synonym, which splits one edge in two."
                .to_owned(),
        );
    }
    scaffold_points.push(
        "Conflicting accounts of one fact from different people are two entries standing, not one \
         overwritten — record the second as the bare fact the new person asserts: a sibling entry \
         on the same memory as the first, phrased the same way so only the value differs (the same \
         field restated with the rival value). Not a sentence narrating the disagreement, and not \
         split across separate memories (a second event, a place of its own) — scattered that way, \
         the synthesis cannot pair the two to weigh them. Both entries must be public (told_by \
         their asserter, not private or attributed), including the first, which you may have filed \
         attributed before the conflict surfaced: if so, correct it to public now, since the \
         synthesis can only flag the arbitration when both are public. When you answer from a fact \
         still in dispute (it reads back marked `disputed`), say the accounts differ rather than \
         picking a side."
            .to_owned(),
    );
    scaffold_points.push(
        "A correction is the opposite: when a fact plainly changes — the teller revises it, or newer \
         information replaces it (a changed number, a promotion) — append the new value and \
         <memory>:supersede the old. Find the old entry by its occurred_at (entry.occurred_at.day), \
         not by matching a date in its text — a dated fact carries its date in occurred_at, which the \
         text need not repeat, so a text search for the digits silently finds nothing and the stale \
         entry stands. The teller is the tell: different people disagree (both stand); one person \
         revising themselves supersedes. Same on read: two values from one source are a revision; \
         supersede the stale copy wherever it sits."
            .to_owned(),
    );
    scaffold_points.push(
        "Every entry has a visibility, and one you leave unmarked defaults to private — back only to \
         its teller and you, withheld whenever anyone else, the subject included, is present. Public \
         surfaces to anyone (openly known, or someone's own account of themselves); attributed \
         surfaces to anyone too but comes back marked as via whoever relayed it."
            .to_owned(),
    );
    scaffold_points.push(
        "So set visibility as you record, never by omission: an ordinary fact one person tells you \
         about another (a role, a workplace, a preference) is attributed — mark it so, or it stays \
         private and you cannot answer about that person once their teller has left the room. \
         Reserve private for a genuine confidence — a hushed register, \"between us\", a request not \
         to repeat, or content plainly not for sharing yet (an unannounced decision, a personnel \
         action, a medical fact) — the floor when you are truly unsure. Your own notes have no \
         protective default either — classify them by the same rule."
            .to_owned(),
    );
    scaffold_points.push(
        "Whenever you record a fact that will not stay true — a current role or team, what someone \
         is working on or leading, where they are, a temporary arrangement, a mood — mark it \
         high-volatility as you record it (volatility = \"high\", or \
         <memory>:set_volatility(\"high\")), not as an afterthought, and attributed in the same \
         breath: both flags, every time — a high fact left at the private default is withheld from \
         all but its teller and never gets to read as out of date. \"medium\" is the default, \
         \"low\" for durable facts like a name."
            .to_owned(),
    );
    scaffold_points.push(
        "A fact you marked fast-changing is one you expect to drift: when you later surface it, give \
         it as possibly out of date — \"last I heard …\", or offer to confirm — not as a settled \
         current fact, even before it reads back marked `stale`. Read entries as they render, the \
         stale and disputed markers riding the text."
            .to_owned(),
    );

    // The body is assembled over the scaffold points, after the shared preamble and namespace legend.
    let mut scaffold_body = String::from(scaffold_preamble);
    scaffold_body.push_str("\n\n- ");
    scaffold_body.push_str(&namespace_kinds);
    for point in &scaffold_points {
        scaffold_body.push_str("\n\n- ");
        scaffold_body.push_str(point);
    }

    vec![
        TemplateDef {
            name: PromptTemplateName::Scaffold,
            version: 1,
            body: scaffold_body,
        },
        TemplateDef {
            name: PromptTemplateName::DescriptionRegen,
            version: 1,
            body: "You synthesize a memory's description from its content entries. Given the \
                   memory's name and entries, write a concise third-person description of who or \
                   what it is and the durable facts that matter. Put that in `description` as plain \
                   prose — no preamble, headings, notes, or first-person framing — synthesizing only \
                   from the entries given. If two or more statements directly contradict each other \
                   about the same thing, record it in `arbitration`: the conflicting statement \
                   numbers in `competing`, the number(s) you judge correct in `credited` (leave it \
                   empty when neither is yet known to be right), and a one-line reconciling note in \
                   `statement`. Two accounts of the same fact attributed to different people — \"one \
                   says X, another says Y\" about the same thing — still contradict and should be \
                   arbitrated; do not treat them as compatible merely because each holds as someone's \
                   account. The `description` and the `arbitration` are not alternatives: if your \
                   description notes that the accounts disagree, conflict, or leave something unsettled \
                   between two values (\"conflicting reports\", \"either X or Y\", \"although Erin \
                   believes Z\"), you must also fill `arbitration` — narrating the conflict in prose \
                   without recording it structurally is the omission this field exists to catch, since \
                   only the structured record lets the disagreement be surfaced later. Only for genuine \
                   contradictions — not a fact being added, refined, or updated over time."
                .to_owned(),
        },
        TemplateDef {
            name: PromptTemplateName::TemporalExtraction,
            version: 1,
            body: "Alongside the description, extract when each numbered statement is *about* in the \
                   real world. For every statement that refers to a real-world time, add an entry to \
                   `occurrences` keyed by its statement number; omit statements with no temporal \
                   reference. Resolve relative phrases (\"last Tuesday\", \"next Friday\", \"a couple \
                   of years ago\") against the stated current time. Use the most specific form you \
                   can justify: a single `day`; a `range` between two days; an `approx` center with a \
                   tolerance in `fuzz_days`; a `recurring` rule; or `before_after` relative to another \
                   memory named as its `anchor` (e.g. event/dave-wedding). All dates are YYYY-MM-DD."
                .to_owned(),
        },
        TemplateDef {
            name: PromptTemplateName::Flush,
            version: 1,
            // The `_session_carryover` guidance teaches `:link`, so it is included only when linking is on.
            // The rest of the flush instruction (write durable state, set visibility) stands either way.
            body: flush_template_body(features),
        },
        TemplateDef {
            name: PromptTemplateName::Imprint,
            version: 1,
            body: "You are meeting your creator for the first time, through the console. This \
                   is how you learn who you are for and who is responsible for you, so be curious: \
                   find out who they are and what they intend you to do. When you learn their name, \
                   create a memory for them with memory.create(\"person/<name>\") — the canonical \
                   handle, with no platform suffix — and record there what you learn about them. \
                   The person you are speaking with is held provisionally as `person/operator`; once \
                   you have created their real memory, merge the two so they are one identity, with \
                   memory.get(\"person/operator\"):link(\"same_as\", memory.get(\"person/<name>\")). \
                   `person/operator` is only that anchor and holds no content — every fact about \
                   them, now and later, goes on their real `person/<name>` profile, never on \
                   `person/operator`. \
                   Record that they created you: memory.get(\"self\"):link(\"created_by\", \
                   memory.get(\"person/<name>\")). Record observations about yourself — your purpose, \
                   your disposition — on self with memory.get(\"self\"):append(text, { by_agent = \
                   true }). This is the only conversation in which you may write self. When you have \
                   understood who they are and recorded it, reply to acknowledge them."
                .to_owned(),
        },
        TemplateDef {
            name: PromptTemplateName::MergeAdjudication,
            version: 1,
            body: "You decide whether two person stubs are the same human across platforms — a merge \
                   that would let everything recorded under one reach the other. You are given each \
                   stub's recorded facts, marked public or private; weigh only these. Set `accepted` \
                   true to merge, false to refuse.\n\n\
                   Merge only on improbable, specific coincidence: facts two different people would be \
                   unlikely to share by chance — the same particular trip in the same week, the same \
                   employer and role and start, an uncommon detail that lines up. Generic overlap is \
                   not evidence: that both are engineers, both like coffee, both live in a large city, \
                   could be almost any two people and must not merge them. Count does not substitute \
                   for specificity — ten generic matches stay weak, while one improbable private \
                   coincidence is strong.\n\n\
                   Weigh the stakes. A wrong merge exposes everything recorded under one stub to the \
                   other, so a private fact — a confidence — is what makes a wrong merge harmful. The \
                   more private facts are at risk, the more specific and improbable the corroboration \
                   must be before you merge. Two stubs holding only public facts, or very few facts, \
                   are low-stakes; a stub rich in confidences demands strong evidence.\n\n\
                   Refuse when uncertain. Merging is the dangerous direction: refusing keeps the two \
                   distinct and loses nothing — an operator can still merge them later — while a wrong \
                   merge leaks a confidence to the wrong person and cannot be un-seen. If the facts are \
                   merely consistent, or could be coincidence, or could be things one person simply \
                   learned about another, refuse. Be wary of a fact that reads like common knowledge or \
                   something that could have been recited: an improbable coincidence the two could not \
                   have known about each other is what tells the same person apart from someone \
                   impersonating them. In `rationale`, name the specific facts that decided it."
                .to_owned(),
        },
        TemplateDef {
            name: PromptTemplateName::LinkInference,
            version: 1,
            body: "You identify relationships implicit in a memory's content and assert them as links \
                   to other memories. You are given the memory's numbered statements, its existing \
                   links, the registered relations, and a list of candidate target memories (by \
                   handle and one-line description). For each relationship you find that links this \
                   memory to one of the candidates, return a `links` entry: the `entry` number it \
                   is grounded in (1-based, as numbered in the prompt), the `relation` label, the \
                   target memory's `target` handle, and a `direction` of \"to\" (this memory → the \
                   target) or \"from\" (the target → this memory). \n\n\
                   Reuse an existing registered relation when one genuinely matches — do not stretch \
                   a relation to cover a relationship it does not name. If none of the registered \
                   relations describes the relationship, coin a new one: add it to `new_relations` \
                   with its `name`, its `inverse` label, and its `from_card` and `to_card` (each \
                   \"one\" or \"many\"), `symmetric`, and `reflexive`. The relation you name on a link \
                   must be either a registered relation or one you list in `new_relations`. \n\n\
                   Do not propose a link that duplicates an existing one — the existing links are \
                   listed, so a relationship already recorded is left alone. Do not propose a \
                   `same_as` link: identity merges flow through the adjudication gate, not this pass. \
                   Resolve relationships only to the candidate handles listed, or to other live \
                   memories whose handles you can name exactly; never invent a handle. \n\n\
                   Infer only structural or neutral relationships whose surfacing to anyone is safe — \
                   authorship, membership, participation, mentorship, origin, composition. Do not \
                   infer sensitive or potentially harmful relationships — dislikes, conflicts, \
                   grudges, adversarial stances — from public content. This pass has no audience \
                   gate yet, so a relationship it creates is visible to anyone, including its target; \
                   steer toward relationships whose public surfacing is safe."
                .to_owned(),
        },
    ]
}

/// The Flush template body. Its `_session_carryover` guidance teaches `:link`, so it is dropped when linking
/// is off; the rest of the flush instruction (write durable state, set visibility) stands either
/// way. Assembling conditionally keeps the three gates (Lua registration, API reference, scaffold)
/// in lockstep — the agent is not taught to link when it cannot.
fn flush_template_body(features: &InstanceFeatures) -> String {
    let mut body =
        "This conversation session is ending and its live transcript is about to scroll \
         out of view. Before it does, write to memory — by emitting Lua through the \
         run_lua tool — anything from it worth keeping that you have not already recorded: \
         facts you learned, decisions made, and commitments given. Record your own \
         observations and inferences under the `agent` teller, and record what you learned \
         about a person on that person's own memory under their canonical person/ handle — \
         not on the memory of whoever told you, and not on a topic; when one participant \
         relayed something about another, it belongs on the person it concerns. This \
         re-recording is your own note, so it has no protective default: you must set its \
         visibility yourself, by the same rule as in a turn — an ordinary relayed fact is \
         visibility = \"attributed\" so it stays available once its teller is gone, a \
         genuine confidence is visibility = \"private\". Keep confidences compartmentalized \
         exactly as in an ordinary turn — anything told to you in confidence, or that you \
         were asked not to repeat, is private wherever it lands; never write it to a public \
         topic, and never mark it public or attributed, which is what would surface it to \
         the person it was kept from."
            .to_owned();
    if features.linking {
        body.push_str(
            " For threads still open, link the relevant memories `_session_carryover` the current context, \
             and clear `_session_carryover` on threads that have closed, so the next session resurfaces what \
             is still live.",
        );
    }
    body.push_str(
        " Nothing you leave only in the transcript survives, so be deliberate; when you \
         have flushed what matters, reply briefly to confirm.",
    );
    body
}

/// A build-seeded system tag. Like the seed relations, these are build defaults rather than part of
/// the genesis manifest hash, so adding one does not perturb an existing agent's hash.
struct TagDef {
    name: &'static str,
    description: &'static str,
}

fn seed_tags() -> Vec<TagDef> {
    vec![TagDef {
        name: "confidential",
        description: "Marks a context as confidential: asides told in a room carrying this tag are \
                      surfaced elsewhere flagged as confidential, and the tag is visible regardless \
                      of who is present.",
    }]
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

fn seed_relations() -> Vec<RelationDef> {
    use Cardinality::{Many, One};
    use RelationName::{
        Created, CreatedBy, HasParticipant, KnownBy, Knows, Operates, OperatorOf, ParticipatesIn,
        SameAs, SessionCarries, SessionCarryover,
    };
    vec![
        // created_by is historical origin (one creator); distinct from current operatorship.
        RelationDef {
            name: CreatedBy,
            inverse: Created,
            from_card: One,
            to_card: Many,
            symmetric: false,
            reflexive: false,
            description: "A thing's historical origin — who created it. Distinct from current \
                operatorship.",
        },
        RelationDef {
            name: OperatorOf,
            inverse: Operates,
            from_card: Many,
            to_card: Many,
            symmetric: false,
            reflexive: false,
            description: "Who currently operates or runs a thing — the present operator, not the \
                originator.",
        },
        RelationDef {
            name: Knows,
            inverse: KnownBy,
            from_card: Many,
            to_card: Many,
            symmetric: false,
            reflexive: false,
            description: "One person knows another — a person-to-person relationship.",
        },
        // Cross-platform identity: symmetric, and its own inverse.
        RelationDef {
            name: SameAs,
            inverse: SameAs,
            from_card: Many,
            to_card: Many,
            symmetric: true,
            reflexive: false,
            description: "Two platform stubs are the same person — cross-platform identity.",
        },
        // A memory flagged for carryover across a compaction seam: the agent links it
        // `_session_carryover` the current context during flush, so the next session resurfaces it.
        // System plumbing, not a semantic relationship — the leading underscore signals this..
        RelationDef {
            name: SessionCarryover,
            inverse: SessionCarries,
            from_card: Many,
            to_card: Many,
            symmetric: false,
            reflexive: false,
            description: "A memory is carried into the next session from this context — set during \
                flush, cleared when the thread closes. System plumbing, not a semantic relationship.",
        },
        // A person's involvement in an event: person/ --participates_in--> event/, inverse
        // event/ --has_participant--> person/. Distinct from _session_carryover (compaction plumbing) and
        // knows (person-to-person): the people at an event are participants, not attendees of a
        // context or acquaintances of the event.
        RelationDef {
            name: ParticipatesIn,
            inverse: HasParticipant,
            from_card: Many,
            to_card: Many,
            symmetric: false,
            reflexive: false,
            description: "A person is in an event — someone who will be there or took part.",
        },
    ]
}

/// A content hash over the genesis manifest — the seed-self and the template versions — so it is
/// stable across resumes and independent of minted ids (spec §Initialization).
fn manifest_hash(seed: &SeedSelf, templates: &[TemplateDef]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(seed.agent_name.as_bytes());
    hasher.update([0]);
    hasher.update(seed.persona.as_bytes());
    hasher.update([0]);
    for entry in &seed.seed_entries {
        hasher.update(entry.as_bytes());
        hasher.update([0]);
    }
    for template in templates {
        hasher.update(template.name.as_str().as_bytes());
        hasher.update(template.version.to_le_bytes());
    }
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    //! A fresh log rolls out a complete agent, an interrupted one resumes by emitting only what's
    //! missing, and a complete one is left alone — all keyed on the presence of `GenesisCompleted`,
    //! never log emptiness (spec §Initialization).
    use crate::{
        InstanceFeatures,
        agent::genesis::{self, GenesisStatus, Rollout, SeedSelf},
        clock::ManualClock,
        event::{EventPayload, EventSource, PromptTemplateName},
        ids::Seq,
        settings::Settings,
        store::{MemoryStore, Store},
        time::Timestamp,
    };

    fn seed() -> SeedSelf {
        SeedSelf {
            agent_name: "Kestrel".to_owned(),
            persona: "A thoughtful, discreet companion with a long memory.".to_owned(),
            seed_entries: vec!["I keep what people tell me in confidence.".to_owned()],
        }
    }

    fn clock() -> ManualClock {
        ManualClock::new(Timestamp::from_millis(1_000))
    }

    /// The `token_budget` in the `ConfigSet` genesis wrote.
    fn logged_token_budget(store: &dyn Store) -> i64 {
        store
            .read_from(Seq::ZERO)
            .unwrap()
            .into_iter()
            .find_map(|event| match event.payload {
                EventPayload::ConfigSet { settings, .. } => Some(settings.compaction.token_budget),
                _ => None,
            })
            .expect("genesis writes a ConfigSet")
    }

    #[test]
    fn the_compaction_budget_is_derived_from_the_context_window() {
        // With a model's window, the initial compaction budget is a fraction of it.
        let mut store = MemoryStore::new();
        genesis::rollout(
            &mut store,
            &clock(),
            &seed(),
            Some(100_000),
            &InstanceFeatures::default(),
        )
        .unwrap();
        assert_eq!(logged_token_budget(&store), 80_000);

        // Without one (an in-memory or model-less instance), the built-in default stands.
        let mut store = MemoryStore::new();
        genesis::rollout(
            &mut store,
            &clock(),
            &seed(),
            None,
            &InstanceFeatures::default(),
        )
        .unwrap();
        assert_eq!(
            logged_token_budget(&store),
            Settings::default().compaction.token_budget
        );
    }

    #[test]
    fn empty_log_is_empty_status() {
        let store = MemoryStore::new();
        assert_eq!(genesis::status(&store).unwrap(), GenesisStatus::Empty);
    }

    #[test]
    fn genesis_boundary_types_round_trip_as_json() {
        // These cross the HTTP API: `SeedSelf` as the create request, `GenesisStatus`/`Rollout` as
        // responses — so they must survive a JSON round-trip.
        let seed = seed();
        let back: SeedSelf = serde_json::from_str(&serde_json::to_string(&seed).unwrap()).unwrap();
        assert_eq!(back.agent_name, seed.agent_name);
        assert_eq!(back.seed_entries, seed.seed_entries);
        for status in [
            GenesisStatus::Empty,
            GenesisStatus::Incomplete,
            GenesisStatus::Complete,
        ] {
            assert_eq!(
                serde_json::from_str::<GenesisStatus>(&serde_json::to_string(&status).unwrap())
                    .unwrap(),
                status
            );
        }
        let rollout = Rollout::Created { events_emitted: 7 };
        assert_eq!(
            serde_json::from_str::<Rollout>(&serde_json::to_string(&rollout).unwrap()).unwrap(),
            rollout
        );
    }

    #[test]
    fn rollout_creates_a_complete_agent() {
        let mut store = MemoryStore::new();
        let outcome = genesis::rollout(
            &mut store,
            &clock(),
            &seed(),
            None,
            &InstanceFeatures::default(),
        )
        .unwrap();
        assert!(matches!(outcome, Rollout::Created { .. }));
        assert_eq!(genesis::status(&store).unwrap(), GenesisStatus::Complete);

        let events = store.read_from(Seq::ZERO).unwrap();

        // The self memory and its seed disposition entry are present.
        let self_created = events.iter().any(|e| {
            matches!(&e.payload, EventPayload::MemoryCreated { name, .. } if name.as_str() == "self")
        });
        assert!(self_created);
        let seed_entry = events
            .iter()
            .any(|e| matches!(&e.payload, EventPayload::MemoryContentAppended { .. }));
        assert!(seed_entry);

        // The seven templates and the same_as seed relation are registered.
        let templates = events
            .iter()
            .filter(|e| matches!(e.payload, EventPayload::PromptTemplateRegistered { .. }))
            .count();
        assert_eq!(templates, 7);
        let same_as = events.iter().any(|e| {
            matches!(&e.payload, EventPayload::LinkTypeRegistered { name, .. } if name.as_str() == "same_as")
        });
        assert!(same_as);
        // The system `confidential` tag is seeded, so a context can be marked confidential.
        let confidential = events.iter().any(|e| {
            matches!(&e.payload, EventPayload::TagCreated { name, .. } if name.as_str() == "confidential")
        });
        assert!(confidential);

        // GenesisCompleted is last, and genesis seeds no created_by link or facts about anyone.
        assert!(matches!(
            events.last().unwrap().payload,
            EventPayload::GenesisCompleted { .. }
        ));
        let any_link = events
            .iter()
            .any(|e| matches!(e.payload, EventPayload::LinkCreated { .. }));
        assert!(!any_link);
    }

    #[test]
    fn rollout_is_idempotent_when_complete() {
        let mut store = MemoryStore::new();
        genesis::rollout(
            &mut store,
            &clock(),
            &seed(),
            None,
            &InstanceFeatures::default(),
        )
        .unwrap();
        let head_after_first = store.head().unwrap();

        let outcome = genesis::rollout(
            &mut store,
            &clock(),
            &seed(),
            None,
            &InstanceFeatures::default(),
        )
        .unwrap();
        assert_eq!(outcome, Rollout::AlreadyComplete);
        assert_eq!(store.head().unwrap(), head_after_first); // nothing appended
    }

    #[test]
    fn interrupted_genesis_resumes_emitting_only_the_missing() {
        // Simulate a partial genesis: a couple of events landed, but no GenesisCompleted.
        let mut store = MemoryStore::new();
        store
            .append(
                Timestamp::from_millis(500),
                vec![
                    EventPayload::prompt_template_registered(
                        PromptTemplateName::Scaffold,
                        1,
                        "<draft system-prompt scaffold — see docs/spec.md §System prompt>"
                            .to_owned(),
                        EventSource::Orchestration,
                    ),
                    EventPayload::prompt_template_registered(
                        PromptTemplateName::DescriptionRegen,
                        1,
                        "<draft description-regeneration template>",
                        EventSource::Orchestration,
                    ),
                ],
            )
            .unwrap();
        assert_eq!(genesis::status(&store).unwrap(), GenesisStatus::Incomplete);
        let head_before = store.head().unwrap();

        let Rollout::Created { events_emitted } = genesis::rollout(
            &mut store,
            &clock(),
            &seed(),
            None,
            &InstanceFeatures::default(),
        )
        .unwrap() else {
            panic!("expected a resuming rollout");
        };

        // The two already-present templates were not re-emitted.
        assert_eq!(genesis::status(&store).unwrap(), GenesisStatus::Complete);
        let total = store.head().unwrap().0 - head_before.0;
        assert_eq!(total as usize, events_emitted);

        // Exactly one copy of each template survives (no duplicates from the resume).
        let events = store.read_from(Seq::ZERO).unwrap();
        let scaffold = events
            .iter()
            .filter(|e| {
                matches!(&e.payload, EventPayload::PromptTemplateRegistered { name, .. } if *name == PromptTemplateName::Scaffold)
            })
            .count();
        assert_eq!(scaffold, 1);
    }

    #[test]
    fn manifest_hash_is_stable_across_a_resume() {
        // A complete genesis and a resumed one over the same seed agree on the manifest hash, since
        // it is computed over content, not minted ids.
        let mut fresh = MemoryStore::new();
        genesis::rollout(
            &mut fresh,
            &clock(),
            &seed(),
            None,
            &InstanceFeatures::default(),
        )
        .unwrap();

        let mut resumed = MemoryStore::new();
        resumed
            .append(
                Timestamp::from_millis(500),
                vec![EventPayload::config_set(
                    Settings::default(),
                    EventSource::Bootstrap,
                )],
            )
            .unwrap();
        genesis::rollout(
            &mut resumed,
            &clock(),
            &seed(),
            None,
            &InstanceFeatures::default(),
        )
        .unwrap();

        assert_eq!(genesis_hash(&fresh), genesis_hash(&resumed));
    }

    fn genesis_hash(store: &MemoryStore) -> String {
        store
            .read_from(Seq::ZERO)
            .unwrap()
            .into_iter()
            .find_map(|e| match e.payload {
                EventPayload::GenesisCompleted { manifest_hash, .. } => Some(manifest_hash),
                _ => None,
            })
            .expect("genesis completed")
    }
}
