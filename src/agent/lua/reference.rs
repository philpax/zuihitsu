//! The agent-facing Lua API as a typed catalogue, rendered into the system prompt's API description.
//! Kept beside the functions installed in [`super::Session::execute`] so the prompt and the
//! implementation cannot drift.

use super::super::api_doc::{ApiEntry, ApiType, enum_of, object};

/// The agent-facing Lua API, as a typed catalogue. Defined here, beside the functions installed in
/// [`super::Session::execute`], so the prompt and the implementation cannot drift: changing a function
/// means changing its entry right next to it. Rendered into the system prompt's API description
/// through [`crate::agent::api_doc::render`] — the same renderer MCP tools project through (spec §System
/// prompt → API description).
pub fn api_reference() -> Vec<ApiEntry> {
    use ApiEntry as AE;
    use ApiType as AT;

    let create = AE::new("memory.create")
        .description("Create a memory, optionally with a first content entry.")
        .required(
            "name",
            AT::String,
            "the namespaced handle, e.g. \"person/<name>\" or \"topic/<subject>\". Names match \
             exactly (case-sensitive), so prefer lowercase — \"person/dave\", not \"person/Dave\" — \
             to avoid splitting one subject across casings",
        )
        .optional("content", AT::String, "an optional first content entry")
        .returns(AT::Handle);

    let get = AE::new("memory.get")
        .description(
            "Fetch a memory by name. Read a merged identity through its canonical person/ handle, \
             not a per-platform stub. The name must match exactly (case-sensitive); if a lookup \
             returns nil, suspect the casing before creating a new memory.",
        )
        .required("name", AT::String, "the memory's handle")
        .returns(AT::Handle.optional());

    let search = AE::new("memory.search")
        .description(
            "Recall memories by meaning and wording, across your whole memory, ranked best-first. \
             Results are filtered to what may surface to who is present, so a teller-private aside \
             appears only while its teller is here (with a marker noting it). Each result is a table \
             { name, description, score, marker? } — fetch a name with memory.get to read more.",
        )
        .required("query", AT::String, "what to look for, in natural language")
        .optional(
            "opts",
            object()
                .optional(
                    "namespace",
                    AT::String,
                    "restrict to a name prefix, e.g. \"person/\"",
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
                    "when the fact is about a real-world time (distinct from now): a tagged table, \
                     one of { instant = <ms> }, { day = \"YYYY-MM-DD\" }, \
                     { range = { start = <ms>, end = <ms> } }, \
                     { approx = { center = <ms>, fuzz_days = <n> } }, { recurring = \"<rrule>\" }, \
                     or { before_after = { dir = \"before\" | \"after\", anchor = \"event/...\" } }",
                ),
            "overrides",
        )
        .returns(AT::Entry);

    let entries = AE::new("<memory>:entries")
        .description(
            "The memory's live content entries, across its whole merged identity. Each is an entry \
             object — read its text with entry.text (it also prints as its text), and pass the \
             object itself to <memory>:supersede to replace it. Hold onto the object if you intend \
             to supersede it.",
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

    let link = AE::new("<memory>:link")
        .description(
            "Record a relationship between this memory and another under a registered relation. When \
             you learn that two memories relate — two people who know each other, an event that \
             belongs to a topic — capture it with link rather than only describing it in their text, \
             so the connection is queryable and can be traversed (pick the fitting relation from the \
             registry). One such use is flagging a still-open thread active_in the current context, \
             so it carries into the next session across a compaction. For a symmetric relation (shown \
             in the registry), link once — the reverse direction is implied, so linking both ways is \
             redundant.",
        )
        .required("relation", AT::String, "the relation from the registry, e.g. \"active_in\"")
        .required(
            "other",
            AT::Handle,
            "the memory to link to, e.g. context.current()",
        );

    let unlink = AE::new("<memory>:unlink")
        .description(
            "Remove a link made with <memory>:link, e.g. clear active_in on a thread that has closed.",
        )
        .required("relation", AT::String, "the relation")
        .required("other", AT::Handle, "the memory the link points to");

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
                .required("name", AT::String, "the relation, e.g. \"mentor_of\"")
                .required("inverse", AT::String, "its inverse label, e.g. \"mentored_by\"")
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
                ),
            "the relation to register",
        );

    let links_list = AE::new("links.list")
        .description(
            "The whole relation registry, each a table { name, inverse, from_card, to_card, \
             symmetric, reflexive } that prints as a readable line — the relations <memory>:link \
             accepts.",
        )
        .returns(AT::Object(Vec::new()).list());

    let links_get = AE::new("links.get")
        .description("One registered relation by either label, or nil if it is not registered.")
        .required("name", AT::String, "the relation or its inverse label")
        .returns(AT::Object(Vec::new()).optional());

    let context = AE::new("context.current")
        .description(
            "The context/* memory for the current conversation. Check its #confidential tag to \
             know whether the room is confidential.",
        )
        .returns(AT::Handle.optional());

    let abort = AE::new("block.abort")
        .description("Discard everything this block buffered and end it, recording the reason.")
        .optional("reason", AT::String, "why the block was abandoned");

    let upcoming = AE::new("calendar.upcoming")
        .description(
            "Memories with something happening soon, soonest first — read each for detail.",
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

    vec![
        create,
        get,
        search,
        append,
        entries,
        history,
        supersede,
        link,
        unlink,
        tag,
        untag,
        tags_create,
        tags_describe,
        tags_list,
        links_register,
        links_list,
        links_get,
        context,
        abort,
        upcoming,
        on,
        recurring,
    ]
}

/// Render [`api_reference`] as the system prompt's API-description block.
pub fn render_api_reference() -> String {
    super::super::api_doc::render(&api_reference())
}
