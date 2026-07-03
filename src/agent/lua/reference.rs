//! The agent-facing Lua API as a typed catalogue, rendered into the system prompt's API description.
//! Kept beside the functions installed in [`super::Session::execute`] so the prompt and the
//! implementation cannot drift.

use super::super::api_doc::{ApiEntry, ApiType, enum_of, object};
use crate::{InstanceFeatures, ids::Namespace};

/// The agent-facing Lua API, as a typed catalogue. Defined here, beside the functions installed in
/// [`super::Session::execute`], so the prompt and the implementation cannot drift: changing a function
/// means changing its entry right next to it. Rendered into the system prompt's API description
/// through [`crate::agent::api_doc::render`] — the same renderer MCP tools project through (spec §System
/// prompt → API description).
///
/// The catalogue is filtered by `features`: a disabled feature's entries are omitted, so the prompt's
/// "What you can do" section never describes a function the runtime rejects. This is the second of the
/// three gates (Lua registration, API reference, scaffold) that must stay in lockstep.
pub fn api_reference(features: &InstanceFeatures) -> Vec<ApiEntry> {
    use ApiEntry as AE;
    use ApiType as AT;

    let person = Namespace::Person.prefix();
    let topic = Namespace::Topic.prefix();
    let event = Namespace::Event.prefix();
    let context = Namespace::Context.prefix();

    let create = AE::new("memory.create")
        .description("Create a memory, optionally with a first content entry.")
        .required(
            "name",
            AT::String,
            format!(
                "the namespaced handle, e.g. \"{person}<name>\" or \"{topic}<subject>\". Names match \
                 exactly (case-sensitive), so prefer lowercase — \"{person}dave\", not \"{person}Dave\" — \
                 to avoid splitting one subject across casings"
            ),
        )
        .optional("content", AT::String, "an optional first content entry")
        .returns(AT::Handle);

    let get = AE::new("memory.get")
        .description(
            format!(
                "Fetch a memory by name. Read a merged identity through its canonical {person} handle, \
                 not a per-platform stub. The name must match exactly (case-sensitive); if a lookup \
                 returns nil, suspect the casing before creating a new memory. A former name still finds \
                 a renamed person: the result then carries a `former_handle` (the old name you looked up \
                 by), and any renamed memory carries `former_names` — they now go by `result.name`, so it \
                 is the same person, and their older entries written under an old name are still theirs. \
                 Answer under the current name without announcing the old one."
            ),
        )
        .required("name", AT::String, "the memory's handle (or a former one)")
        .returns(AT::Handle.optional());

    let search = AE::new("memory.search")
        .description(
            "Recall memories by meaning and wording, across your whole memory, ranked best-first. \
             Results are filtered to what may surface to who is present, so a teller-private aside \
             appears only while its teller is here (with a marker noting it). Each result is a table \
             { name, description, score, marker?, snippet?, occurred_at? } — snippet is the matched \
             content that produced the hit, so you can triage a result even when its description is \
             thin, and occurred_at is the memory's representative date (the same tagged table append \
             takes, e.g. occurred_at.day) when it holds a dated fact. A hit is a pointer, not the \
             whole record: fetch a name with memory.get to read every entry and occurrence in full.",
        )
        .required("query", AT::String, "what to look for, in natural language")
        .optional(
            "opts",
            object()
                .optional(
                    "namespace",
                    AT::String,
                    format!("restrict to a name prefix, e.g. \"{person}\""),
                )
                .optional(
                    "tags",
                    AT::String.list(),
                    "tags to prefer; a result carrying more of them ranks higher",
                )
                .optional("limit", AT::Integer, "how many results to return (default 8)"),
            "options",
        )
        .returns(AT::Object(Vec::new()).list());

    let append = AE::new("<memory>:append")
        .description(
            "Append a content entry. By default it is attributed to the current speaker, and an \
             aside about someone else defaults private to that speaker. When you record an entry \
             about a person as your own observation (a synthesis or a flush), there is no default — \
             you must set its visibility yourself, public or private.",
        )
        .required("text", AT::String, "the entry text")
        .optional(
            "opts",
            object()
                .optional(
                    "by_agent",
                    AT::Boolean,
                    "record it as your own observation instead of the speaker's",
                )
                .optional(
                    "visibility",
                    enum_of(["public", "private"]),
                    "force the visibility; required for an entry you author about a person",
                )
                .optional(
                    "occurred_at",
                    object(),
                    format!(
                        "when the fact is about a real-world time (distinct from now): a tagged table, \
                         one of {{ instant = <ms> }}, {{ day = \"YYYY-MM-DD\" }}, \
                         {{ range = {{ start = <ms>, end = <ms> }} }}, \
                         {{ approx = {{ center = <ms>, fuzz_days = <n> }} }}, {{ recurring = \"<rrule>\" }}, \
                         or {{ before_after = {{ dir = \"before\" | \"after\", anchor = \"{event}...\" }} }}"
                    ),
                ),
            "overrides",
        )
        .returns(AT::Entry);

    let entries = AE::new("<memory>:entries")
        .description(
            "The memory's live content entries, across its whole merged identity. Each is an entry \
             object — read its text with entry.text (it also prints as its text, prefixed by its \
             date, visibility, teller, and a disputed marker when contested), and pass the object \
             itself to <memory>:supersede to replace it. entry.occurred_at, when dated, is the same \
             tagged table append takes (e.g. entry.occurred_at.day), so you can match an entry by its \
             date and reuse it. Hold onto the object if you intend to supersede it. Capture the list \
             to see it — `local es = <memory>:entries()`, or iterate/print it; a bare \
             `<memory>:entries()` whose result you discard returns nothing to you, not an empty \
             memory.",
        )
        .returns(AT::Entry.list());

    let history = AE::new("<memory>:history")
        .description(
            "The memory's entries including superseded ones, oldest first — the full record, where \
             <memory>:entries shows only the live ones. Each is an entry object (entry.text for its \
             text).",
        )
        .returns(AT::Entry.list());

    let supersede = AE::new("<memory>:supersede")
        .description(
            "Correct or retract a fact: mark an old entry superseded by a new one. Append the \
             correction first to get the new entry object, then call supersede with the old entry \
             object (from <memory>:entries) and the new one. The old entry drops from live reads but \
             stays in <memory>:history. Use this only when the same fact has genuinely changed — a \
             correction, a retraction, or an update to newer information (often a teller revising \
             their own earlier statement). When two participants give conflicting accounts of the \
             same thing, do not supersede one with the other: record both as separate entries and \
             leave the disagreement standing, so the conflict stays visible to be reconciled rather \
             than silently resolved to whoever spoke last.",
        )
        .required(
            "old",
            AT::Entry,
            "the entry object being replaced (from <memory>:entries)",
        )
        .required(
            "new",
            AT::Entry,
            "the entry object that replaces it (from <memory>:append)",
        );

    let revise = AE::new("<memory>:revise")
        .description(
            "Correct a fact in one call: append new_text as a new entry and supersede the old entry \
             with it, returning the new entry. The same intent as append-then-supersede but without \
             the two-step — and it cannot half-apply: if the old entry is not a live one, the whole \
             revision (the new entry included) is rejected, so a correction never leaves the stale \
             value standing. Use it for a genuine change to the same fact (a teller revising their \
             own earlier statement, newer information replacing older); for two people's conflicting \
             accounts, record both separately instead and leave the disagreement standing.",
        )
        .required(
            "old",
            AT::Entry,
            "the entry being corrected (from <memory>:entries — match it by its occurred_at or text)",
        )
        .required("new_text", AT::String, "the corrected fact's text")
        .optional(
            "opts",
            object()
                .optional("visibility", enum_of(["public", "private"]), "force the visibility")
                .optional("occurred_at", object(), "the new value's occurrence, if it is dated"),
            "the same overrides <memory>:append takes",
        )
        .returns(AT::Entry);

    let link = AE::new("<memory>:link")
        .description(
            "Record a relationship between this memory and another under a registered relation. When \
             you learn that two memories relate — two people who know each other, an event that \
             belongs to a topic — capture it with link rather than only describing it in their text, \
             so the connection is queryable and can be traversed (pick the fitting relation from the \
             registry). For a symmetric relation (shown in the registry), link once — the reverse \
             direction is implied, so linking both ways is redundant.",
        )
        .required("relation", AT::String, "the relation from the registry, e.g. \"part_of\"")
        .required(
            "other",
            AT::Handle,
            "the memory to link to — a handle (e.g. context.current()) or its name as a string, \
             which is looked up",
        );

    let unlink = AE::new("<memory>:unlink")
        .description("Remove a link made with <memory>:link when the relationship no longer holds.")
        .required("relation", AT::String, "the relation")
        .required("other", AT::Handle, "the memory the link points to");

    let outgoing = AE::new("<memory>:outgoing")
        .description(
            "The memories this one links to under a relation, across its whole merged identity, in the \
             relation's forward direction — <memory>:outgoing(\"knows\") is who it knows. Each \
             result is a table { relation, memory, name, direction, source, told_by } that prints as \
             \"relation → name\"; reach the linked memory through result.memory to read or act on it, \
             and result.told_by names who asserted the relationship (the provenance of a belief-bearing \
             link). Use <memory>:incoming for the reverse direction (who knows it). For a symmetric \
             relation, outgoing and incoming return the same neighbours.",
        )
        .required("relation", AT::String, "the relation from the registry, e.g. \"knows\"")
        .returns(AT::Object(Vec::new()).list());

    let incoming = AE::new("<memory>:incoming")
        .description(
            "The memories that link to this one under a relation, across its whole merged identity — \
             <memory>:incoming(\"knows\") is who knows it. The reverse of <memory>:outgoing; the \
             result shape is the same.",
        )
        .required("relation", AT::String, "the relation from the registry, e.g. \"knows\"")
        .returns(AT::Object(Vec::new()).list());

    let links = AE::new("<memory>:links")
        .description(
            "Every link from this memory's merged identity out to other memories, in every relation \
             and both directions — the relationship overview. Each result is a table { relation, \
             memory, name, direction, source } that prints as \"relation → name\" (or \"← name\" for \
             an incoming link); reach a linked memory through result.memory.",
        )
        .returns(AT::Object(Vec::new()).list());

    let propose_merge = AE::new("<memory>:propose_merge")
        .description(
            format!(
                "Record that this {person} stub and another are the same human across platforms, for \
                 adjudication on the evidence. This does not merge them and surfaces nothing on its own — \
                 it is your judgment, weighed against the independently-recorded facts. Propose only from \
                 what you already hold about each, never from claims made to convince you in the moment. \
                 You cannot merge by asserting same_as yourself."
            ),
        )
        .required("other", AT::Handle, format!("the other {person} stub"));

    let tag = AE::new("<memory>:tag")
        .description(
            "Apply a tag to this memory. The tag must already exist in the vocabulary — create it \
             first with tags.create. Tagging is what it's about; the namespace is what it is.",
        )
        .required(
            "name",
            AT::String,
            "the tag, e.g. \"confidential\" on a context to mark the room confidential",
        );

    let untag = AE::new("<memory>:untag")
        .description("Remove a tag from this memory.")
        .required("name", AT::String, "the tag to clear");

    let rename = AE::new("<memory>:rename")
        .description(
            format!(
                "Give this memory a new handle, keeping it the same memory — when someone changes the \
                 name they go by (a new chosen name, a married name), rename their {person} memory rather \
                 than creating a new one. The memory keeps all its facts, links, and history under the new \
                 handle; a fresh memory would split the person in two. The old name stops resolving and is \
                 not surfaced again, so refer to them by the new name from now on. Renaming onto a handle \
                 that already belongs to a different memory is an error — that is two separate people, not \
                 a rename."
            ),
        )
        .required("name", AT::String, format!("the new handle, e.g. \"{person}sarah\""));

    let set_volatility = AE::new("<memory>:set_volatility")
        .description(
            "Set a memory's volatility — how fast its facts drift. \"high\" for fast-changing facts \
             (a current role, where someone is, what they are working on), \"medium\" the default, \
             \"low\" for durable facts like a name. A high-volatility memory surfaces later flagged as \
             possibly out of date.",
        )
        .required(
            "level",
            enum_of(["low", "medium", "high"]),
            "the volatility level",
        );

    let tags_create = AE::new("tags.create")
        .description(
            "Add a tag to the vocabulary with a one-line purpose. Creation is distinct from \
             application: creating forces a purpose, while <memory>:tag never mutates it.",
        )
        .required("name", AT::String, "the tag name, e.g. \"hobbies\"")
        .required("description", AT::String, "its one-line purpose");

    let tags_describe = AE::new("tags.describe")
        .description(
            "Change an existing tag's one-line purpose (create it first with tags.create).",
        )
        .required("name", AT::String, "the tag name")
        .required("description", AT::String, "the new purpose");

    let tags_list = AE::new("tags.list")
        .description(
            "The whole tag vocabulary, each a table { name, description, count } that prints as a \
             readable line — what you can apply with <memory>:tag.",
        )
        .returns(AT::Object(Vec::new()).list());

    let links_register = AE::new("links.register")
        .description(
            "Register a link relation, usable thereafter under either label by <memory>:link. Edges \
             are made with <memory>:link; this declares the relation they instantiate. \
             Re-registering a name updates it.",
        )
        .required(
            "spec",
            object()
                .required("name", AT::String, "the relation, e.g. \"reports_to\"")
                .required("inverse", AT::String, "its inverse label, e.g. \"manages\"")
                .required(
                    "from_card",
                    enum_of(["one", "many"]),
                    "how many of this relation a memory may have outgoing",
                )
                .required(
                    "to_card",
                    enum_of(["one", "many"]),
                    "how many it may have incoming (the inverse direction)",
                )
                .optional(
                    "symmetric",
                    AT::Boolean,
                    "whether the relation reads the same in both directions (default false)",
                )
                .optional(
                    "reflexive",
                    AT::Boolean,
                    "whether a memory may hold this relation to itself (default false)",
                )
                .optional(
                    "description",
                    AT::String,
                    "a one-line purpose so the agent knows when to use the relation",
                ),
            "the relation to register",
        );

    let links_list = AE::new("links.list")
        .description(
            "The whole relation registry, each a table { name, inverse, from_card, to_card, \
             symmetric, reflexive, description } that prints as a readable line — the relations \
             <memory>:link accepts.",
        )
        .returns(AT::Object(Vec::new()).list());

    let links_get = AE::new("links.get")
        .description("One registered relation by either label, or nil if it is not registered.")
        .required("name", AT::String, "the relation or its inverse label")
        .returns(AT::Object(Vec::new()).optional());

    let context = AE::new("context.current")
        .description(format!(
            "The {context}* memory for the current conversation. Check its #confidential tag to \
                 know whether the room is confidential."
        ))
        .returns(AT::Handle.optional());

    let convo_turn = AE::new("convo.turn")
        .description(
            "Resolve a reference to an earlier moment to that turn and the exchange around it. \
             References arrive two ways: a [turn:<id>] token, or a console link carrying the id as \
             ?turn=<id> — pass the id from either here. The result is a table { id, ref, text, \
             speaker, role, at, window } — the linked turn's fields (ref is the canonical [turn:<id>] \
             to cite it by, copy it into your reply), and window the surrounding turns (the linked \
             one flagged focused) — that prints as a transcript excerpt with the moment marked. A \
             moment resolves only when everyone present here was in its audience; if it wasn't, that \
             is an error naming the audience problem — recall through memory instead of replaying the \
             transcript. A malformed id and an unknown id are likewise errors.",
        )
        .required(
            "id",
            AT::String,
            "the turn id — the value inside a [turn:<id>] token or the ?turn=<id> of a pasted link",
        )
        .returns(AT::Object(Vec::new()));

    let abort = AE::new("block.abort")
        .description("Discard everything this block buffered and end it, recording the reason.")
        .optional("reason", AT::String, "why the block was abandoned");

    let upcoming = AE::new("calendar.upcoming")
        .description(
            "Memories with something happening soon (including the next instance of a recurring one), \
             soonest first. Each is a memory handle — read m.name and m.description, or call its \
             methods (m:entries() …) for detail.",
        )
        .optional(
            "opts",
            object().optional(
                "within",
                AT::String,
                "how far ahead to look, e.g. \"7 days\" or \"2 weeks\"; defaults to 7 days",
            ),
            "options",
        )
        .returns(AT::Handle.list());

    let on = AE::new("calendar.on")
        .description("Memories with something happening on a given day.")
        .required("date", AT::String, "the day as \"YYYY-MM-DD\"")
        .returns(AT::Handle.list());

    let recurring = AE::new("calendar.recurring")
        .description("Memories with a recurring occurrence.")
        .returns(AT::Handle.list());

    let cal_today = AE::new("calendar.today")
        .description(
            "Today's date as a date object — pass it straight to append as occurred_at, or do \
             arithmetic on it (:add_days, :add_weeks, :add_months, :weekday). Compute dates this way \
             rather than working one out yourself.",
        )
        .returns(AT::Handle);

    let cal_next = AE::new("calendar.next")
        .description(
            "The next date on or after today falling on a weekday, as a date object — \
             calendar.next(\"friday\") is this Friday (today if today is Friday). Use this for \"this \
             Friday\" instead of computing the date.",
        )
        .required("weekday", AT::String, "a weekday name, e.g. \"friday\"")
        .returns(AT::Handle);

    let cal_in_days = AE::new("calendar.in_days")
        .description("The date that many days from today, as a date object (negative goes back).")
        .required("days", AT::Number, "how many days from today")
        .returns(AT::Handle);

    let cal_in_weeks = AE::new("calendar.in_weeks")
        .description("The date that many weeks from today, as a date object.")
        .required("weeks", AT::Number, "how many weeks from today")
        .returns(AT::Handle);

    let cal_date = AE::new("calendar.date")
        .description("Parse an explicit \"YYYY-MM-DD\" into a date object.")
        .required("day", AT::String, "the day as \"YYYY-MM-DD\"")
        .returns(AT::Handle);

    let date_add_days = AE::new("<date>:add_days")
        .description("A new date shifted by this many days (negative goes back).")
        .required("days", AT::Number, "how many days to shift")
        .returns(AT::Handle);

    let date_add_weeks = AE::new("<date>:add_weeks")
        .description(
            "A new date shifted by this many weeks — \"the Friday after next\" is \
             calendar.next(\"friday\"):add_weeks(1).",
        )
        .required("weeks", AT::Number, "how many weeks to shift")
        .returns(AT::Handle);

    let date_add_months = AE::new("<date>:add_months")
        .description(
            "A new date shifted by this many months, keeping the day-of-month where it exists and \
             clamping where it does not (31 Jan + 1 month is 28/29 Feb).",
        )
        .required("months", AT::Number, "how many months to shift")
        .returns(AT::Handle);

    let date_weekday = AE::new("<date>:weekday")
        .description("The date's weekday name, e.g. \"Friday\".")
        .returns(AT::String);

    // Assemble the catalogue, gating each feature group on its flag. The memory group (create,
    // append, supersede, …) is always on — an agent without memory is not an agent — and includes
    // `set_volatility`, which the scaffold references (fixing the pre-existing drift where it was
    // installed and scaffold-referenced but absent from this catalogue). `context` and `block.abort`
    // are infrastructure, always on.
    let mut entries = vec![
        create,
        get,
        search,
        append,
        entries,
        history,
        supersede,
        revise,
        rename,
        set_volatility,
    ];
    if features.linking {
        entries.extend([link, unlink, outgoing, incoming, links]);
    }
    if features.merging {
        entries.push(propose_merge);
    }
    if features.tagging {
        entries.extend([tag, untag, tags_create, tags_describe, tags_list]);
    }
    if features.linking {
        entries.extend([links_register, links_list, links_get]);
    }
    entries.push(context);
    if features.transcripts {
        entries.push(convo_turn);
    }
    if features.calendar {
        entries.extend([
            upcoming,
            on,
            recurring,
            cal_today,
            cal_next,
            cal_in_days,
            cal_in_weeks,
            cal_date,
            date_add_days,
            date_add_weeks,
            date_add_months,
            date_weekday,
        ]);
    }
    entries.push(abort);
    entries
}

/// Render [`api_reference`] as the system prompt's API-description block, filtered by `features`.
pub fn render_api_reference(features: &InstanceFeatures) -> String {
    super::super::api_doc::render(&api_reference(features))
}

#[cfg(test)]
mod tests {
    use super::api_reference;
    use crate::InstanceFeatures;

    /// The call names of a feature's entries, in order, for a readable diff on failure.
    fn names(features: &InstanceFeatures) -> Vec<String> {
        api_reference(features)
            .iter()
            .map(|entry| entry.call.clone())
            .collect()
    }

    #[test]
    fn disabling_linking_omits_every_link_entry() {
        let features = InstanceFeatures {
            linking: false,
            ..Default::default()
        };
        let entries = names(&features);
        // The write and read sides of linking both vanish.
        for name in [
            "<memory>:link",
            "<memory>:unlink",
            "<memory>:outgoing",
            "<memory>:incoming",
            "<memory>:links",
            "links.register",
            "links.list",
            "links.get",
        ] {
            assert!(
                !entries.contains(&name.to_owned()),
                "{name:?} should be absent"
            );
        }
        // Memory and context remain.
        assert!(entries.contains(&"memory.create".to_owned()));
        assert!(entries.contains(&"context.current".to_owned()));
    }

    #[test]
    fn disabling_merging_omits_propose_merge() {
        let features = InstanceFeatures {
            merging: false,
            ..Default::default()
        };
        let entries = names(&features);
        assert!(!entries.contains(&"<memory>:propose_merge".to_owned()));
    }

    #[test]
    fn disabling_transcripts_omits_convo_turn() {
        let features = InstanceFeatures {
            transcripts: false,
            ..Default::default()
        };
        assert!(!names(&features).contains(&"convo.turn".to_owned()));
        // On by default, it is present.
        assert!(names(&InstanceFeatures::default()).contains(&"convo.turn".to_owned()));
    }

    #[test]
    fn disabling_calendar_omits_every_calendar_entry() {
        let features = InstanceFeatures {
            calendar: false,
            ..Default::default()
        };
        let entries = names(&features);
        assert!(!entries.contains(&"calendar.today".to_owned()));
        assert!(!entries.contains(&"<date>:add_days".to_owned()));
    }
}
