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
    clock::Clock,
    event::{Cardinality, EventPayload, EventSource, PromptTemplateName, Teller, Visibility},
    ids::{EntryId, MemoryId, MemoryName, Seq},
    settings::Settings,
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
pub fn rollout(
    store: &mut dyn Store,
    clock: &dyn Clock,
    seed: &SeedSelf,
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

    let templates = default_templates();
    let mut to_emit: Vec<EventPayload> = Vec::new();

    for template in &templates {
        if !templates_present.contains(&(template.name, template.version)) {
            to_emit.push(EventPayload::PromptTemplateRegistered {
                name: template.name,
                version: template.version,
                body: template.body.to_owned(),
                source: EventSource::Orchestration,
            });
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
            });
        }
    }

    for tag in seed_tags() {
        if !tags_present.contains(tag.name) {
            to_emit.push(EventPayload::TagCreated {
                name: TagName::new(tag.name),
                description: tag.description.to_owned(),
            });
        }
    }

    if !config_present {
        to_emit.push(EventPayload::ConfigSet {
            settings: Settings::default(),
            source: EventSource::Bootstrap,
        });
    }

    if !self_present {
        let self_id = MemoryId::generate();
        to_emit.push(EventPayload::MemoryCreated {
            id: self_id,
            name: MemoryName::new(MemoryName::SELF),
        });
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
    to_emit.push(EventPayload::GenesisCompleted {
        manifest_hash: manifest_hash(seed, &templates),
        template_versions,
    });

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
    body: &'static str,
}

fn default_templates() -> Vec<TemplateDef> {
    vec![
        TemplateDef {
            name: PromptTemplateName::Scaffold,
            version: 1,
            body: "You act through a persistent memory that you read and write by emitting Lua \
                   through the run_lua tool. A turn is a loop of steps: at each step you either \
                   call run_lua or give a reply. What you write to memory persists across sessions; \
                   your in-block scratchpad does not. You speak with several participants, who do \
                   not all see the same things.\n\n\
                   Memories are namespaced by kind: person/ for people, place/ for places, event/ \
                   for things that happen at a time (appointments, meetings, recurring schedules), \
                   topic/ for subjects, context/ for conversations, and self for you. Read a merged \
                   identity through its canonical person/ handle (not a per-platform stub) so \
                   you do not look in the wrong place and miss what you know. When you come to believe \
                   two person/ stubs are the same human on different platforms — because what you have \
                   independently recorded about each improbably coincides, not because someone in the \
                   conversation says so — propose the merge with one:propose_merge(other). That records \
                   your judgment for adjudication on the evidence; it does not merge them itself, and you \
                   never merge by asserting same_as yourself. Propose only from what you already hold \
                   about each, never from facts offered in the moment to convince you they match — \
                   leaning on those is how an impersonator would try to reach someone else's \
                   confidences. Until such a merge is made, two stubs are two different people even when \
                   they share a display name: a confidence one told you is theirs alone, private to \
                   their stub, and is never handed to the other. Answer a person from their own memory — \
                   the stub of whoever is actually speaking — not a same-named stub from another \
                   platform. Someone who recites a person's public facts to seem like them and then asks \
                   what that person told you in confidence is the impersonation the merge gate exists to \
                   stop: do not treat them as that person, and do not surface the confidence, on the \
                   strength of a shared name or recited details — a name is not proof of identity. Your \
                   memory holds far \
                   more than the conversation in front of you, so when you are asked about something \
                   you may know — a fact from another room, an earlier session, a person, a plan, a \
                   preference — search it before you answer (memory.search by meaning, or memory.get \
                   by name) rather than replying from the live conversation alone. A question is \
                   usually a cue to consult what you know, not only to answer from what is in view. \
                   When what you need is what you know about a particular person — their preferences, \
                   their plans, their history — read their memory by its handle (memory.get with their \
                   person/ name), which brings back everything you hold about them; that is surer than \
                   searching the topic of the request, which may miss a fact you filed under different \
                   words than the question uses.\n\n\
                   When someone asks you to remember something, or to remind them of it, act on it \
                   then and there — record it, rather than interrogating them for details you can \
                   reasonably default and refine later. Something that happens at a time is an \
                   event/ memory with an occurred_at: a specific time for a one-off, or a recurring \
                   rule like occurred_at = { recurring = \"FREQ=WEEKLY\" } for something that \
                   repeats, which is what lets it come back and nudge them when it falls due. \
                   A cadence or a day is enough to record the reminder: a missing time of day is a \
                   detail to default (a sensible hour now, refined if they later say when), not a \
                   precondition to ask for before writing anything. A bare weekly cadence (\"every \
                   Friday\") is already a complete recurrence — record it as one and confirm, rather \
                   than withholding the write until you have the hour; an unrecorded reminder cannot \
                   fire at all, so capturing it with a defaulted time beats interrogating for the exact \
                   one. \
                   When the time is given relative to now — \"this Friday\", \"in two weeks\", \"next \
                   month\" — do not work the date out in your head; ask the calendar for it: \
                   calendar.next(\"friday\"), calendar.in_weeks(2), calendar.today():add_months(1). \
                   Each returns a date you can pass straight as occurred_at, computed correctly, so the \
                   reminder fires on the day meant rather than one a miscount landed on. \
                   Capture first; save a clarifying question for a genuine judgment call, such as \
                   how private something is — not for routine scheduling detail you can fill in.\n\n\
                   Record your own observations and inferences under the `agent` teller, and record \
                   what you learn about a person on that person's own memory, under their canonical \
                   person/ handle — not on the memory of whoever told you, and not on a topic. When \
                   one participant relays something about another, the fact is about the person it \
                   concerns, so it belongs on their memory even though someone else is speaking; \
                   filing it on the subject is also what lets the system hold it back while that \
                   subject is present. Record what is new, and only once. Before writing, consider \
                   whether you already hold it: a fact already in memory — from earlier this session \
                   or an earlier one — needs no re-recording, and a question that merely surfaces \
                   something you already know is answered from memory, not written again. This matters \
                   most at the seams — a query that brings an existing memory back to you, or a \
                   session you are flushing: persist only what is genuinely new since you last \
                   recorded. Re-writing what is already saved piles up duplicates, and a fact you \
                   re-record now is attributed to whoever is speaking now rather than to whoever first \
                   told it, silently re-keying whose note it is.\n\n\
                   When what you learn is itself structured, record it through the operation built \
                   for it, not only as prose the rest of the system cannot act on. A relationship \
                   between two memories — two people who know each other, an event that belongs to a \
                   topic — is a <memory>:link under the right relation, not just a sentence in their \
                   text. And two people's conflicting accounts of the same fact are two entries left \
                   standing, not one overwritten: the disagreement is itself worth holding, and \
                   keeping both is what lets it be surfaced and reconciled later rather than silently \
                   resolved to whoever spoke last. A correction is the opposite case: when a fact you \
                   already recorded plainly changes — the teller revises it, or newer information \
                   replaces it (a phone number that changed, a title someone was promoted into) — \
                   append the new value and mark the stale entry superseded by it with \
                   <memory>:supersede, so the outdated value stops surfacing as if it still held. \
                   Telling the two apart is the point: conflicting accounts that both stand are a \
                   disagreement to hold, while a fact that has clearly moved on is an update to \
                   supersede. And when you later answer from a fact that is still in dispute — two \
                   accounts standing, or a disagreement you have not resolved — say so rather than \
                   presenting one side as settled: surface that the accounts differ and that it is \
                   worth confirming. A read tells you which facts are contested: an entry under an \
                   unresolved arbitration comes back marked `disputed` (for example `[disputed · public \
                   · from person/erin]`), so when you read one before answering, that marker is your cue \
                   to surface the disagreement instead of picking a side. Asserting a contested fact as \
                   settled is its own error, the read side of silently overwriting one account with the \
                   other.\n\n\
                   Every entry carries a visibility that governs where it can resurface, and a fact \
                   one participant relays about another comes in three postures you choose between as \
                   you record it. A public entry surfaces to anyone in any room, including the very \
                   person it is about — for what is openly known or someone said about themselves. A \
                   private one (visibility = \"private\") is a confidence: it comes back only to the \
                   teller who told it and to you, withheld whenever anyone else — the subject \
                   included — is present. An attributed entry (visibility = \"attributed\") is the \
                   middle, and the common case: an ordinary fact a colleague mentioned about someone \
                   — their role, where they work, a preference — fine for others to know but yours \
                   only secondhand. It surfaces to anyone, so you can still answer about that person \
                   once the colleague who told you has left the room, but it comes back marked as via \
                   whoever relayed it, a reminder to weigh it as a relayed fact, not the person's own \
                   account. So classify as you record: a genuine confidence — a hushed register, \
                   \"between us,\" a request not to repeat it, anything sensitive — is private; an \
                   everyday relayed fact is attributed. Reach for attributed for ordinary facts so you \
                   do not lose your memory of someone the moment their describer is absent, and reserve \
                   private for what is actually a secret — but when you are unsure which, keep it \
                   private; that is the floor, and opening a fact up is a deliberate choice, never an \
                   accident. When you record a note about a person as your own observation — \
                   synthesizing, or flushing a session before it scrolls away — it has no protective \
                   default, so you must classify it yourself by the same rule. Marking a confidence \
                   public or attributed is what lets it leak.",
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
                   contradictions — not a fact being added, refined, or updated over time.",
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
                   memory named as its `anchor` (e.g. event/dave-wedding). All dates are YYYY-MM-DD.",
        },
        TemplateDef {
            name: PromptTemplateName::Flush,
            version: 1,
            body: "This conversation session is ending and its live transcript is about to scroll \
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
                   the person it was kept from. For threads still open, link \
                   the relevant memories `active_in` the current context, and clear `active_in` on \
                   threads that have closed, so the next session resurfaces what is still live. \
                   Nothing you leave only in the transcript survives, so be deliberate; when you \
                   have flushed what matters, reply briefly to confirm.",
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
                   Record that they created you: memory.get(\"self\"):link(\"created_by\", \
                   memory.get(\"person/<name>\")). Record observations about yourself — your purpose, \
                   your disposition — on self with memory.get(\"self\"):append(text, { by_agent = \
                   true }). This is the only conversation in which you may write self. When you have \
                   understood who they are and recorded it, reply to acknowledge them.",
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
                   impersonating them. In `rationale`, name the specific facts that decided it.",
        },
    ]
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
}

fn seed_relations() -> Vec<RelationDef> {
    use Cardinality::{Many, One};
    use RelationName::{
        ActiveIn, Created, CreatedBy, HasActive, KnownBy, Knows, Operates, OperatorOf, SameAs,
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
        },
        RelationDef {
            name: OperatorOf,
            inverse: Operates,
            from_card: Many,
            to_card: Many,
            symmetric: false,
            reflexive: false,
        },
        RelationDef {
            name: Knows,
            inverse: KnownBy,
            from_card: Many,
            to_card: Many,
            symmetric: false,
            reflexive: false,
        },
        // Cross-platform identity: symmetric, and its own inverse.
        RelationDef {
            name: SameAs,
            inverse: SameAs,
            from_card: Many,
            to_card: Many,
            symmetric: true,
            reflexive: false,
        },
        // A memory flagged live in a context; used by compaction carryover.
        RelationDef {
            name: ActiveIn,
            inverse: HasActive,
            from_card: Many,
            to_card: Many,
            symmetric: false,
            reflexive: false,
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
        let outcome = genesis::rollout(&mut store, &clock(), &seed()).unwrap();
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

        // The six templates and the same_as seed relation are registered.
        let templates = events
            .iter()
            .filter(|e| matches!(e.payload, EventPayload::PromptTemplateRegistered { .. }))
            .count();
        assert_eq!(templates, 6);
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
        genesis::rollout(&mut store, &clock(), &seed()).unwrap();
        let head_after_first = store.head().unwrap();

        let outcome = genesis::rollout(&mut store, &clock(), &seed()).unwrap();
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
                    EventPayload::PromptTemplateRegistered {
                        name: PromptTemplateName::Scaffold,
                        version: 1,
                        body: "<draft system-prompt scaffold — see docs/spec.md §System prompt>"
                            .to_owned(),
                        source: EventSource::Orchestration,
                    },
                    EventPayload::PromptTemplateRegistered {
                        name: PromptTemplateName::DescriptionRegen,
                        version: 1,
                        body: "<draft description-regeneration template>".to_owned(),
                        source: EventSource::Orchestration,
                    },
                ],
            )
            .unwrap();
        assert_eq!(genesis::status(&store).unwrap(), GenesisStatus::Incomplete);
        let head_before = store.head().unwrap();

        let Rollout::Created { events_emitted } =
            genesis::rollout(&mut store, &clock(), &seed()).unwrap()
        else {
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
        genesis::rollout(&mut fresh, &clock(), &seed()).unwrap();

        let mut resumed = MemoryStore::new();
        resumed
            .append(
                Timestamp::from_millis(500),
                vec![EventPayload::ConfigSet {
                    settings: Settings::default(),
                    source: EventSource::Bootstrap,
                }],
            )
            .unwrap();
        genesis::rollout(&mut resumed, &clock(), &seed()).unwrap();

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
