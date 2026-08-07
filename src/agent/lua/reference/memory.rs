//! Memory API reference entries: create, get, get_or_create, search, list, append, attest, entries,
//! find_entry, history, details, supersede, retract, revise, rename, set_volatility, and the
//! always-on block.abort.

use crate::{
    agent::{
        api_doc::{ApiEntry, ApiEntry as AE, ApiType as AT, enum_of, object},
        body_of, render_placeholders,
    },
    ids::Namespace,
};

/// The always-on memory entries plus the handle methods (append, entries, details, …).
pub(super) fn entries() -> Vec<ApiEntry> {
    let person = Namespace::Person.prefix();
    let topic = Namespace::Topic.prefix();
    let event = Namespace::Event.prefix();

    let create = AE::new("memory.create")
        .description(body_of(include_str!("prose/memory/create.md")))
        .required(
            "name",
            AT::String,
            render_placeholders(
                body_of(include_str!("prose/memory/create_name.md")),
                &[("person", person), ("topic", topic)],
            ),
        )
        .optional(
            "content",
            AT::String,
            "an optional first content entry (subject to the same character limit as append)",
        )
        .optional(
            "opts",
            object(),
            body_of(include_str!("prose/memory/create_opts.md")),
        )
        .returns(AT::Handle);

    let get = AE::new("memory.get")
        .description(render_placeholders(
            body_of(include_str!("prose/memory/get.md")),
            &[("person", person)],
        ))
        .required(
            "name",
            AT::String,
            "the memory's handle (or a former one), or an existing memory handle",
        )
        .returns(AT::Handle.optional());

    let get_or_create = AE::new("memory.get_or_create")
        .description(body_of(include_str!("prose/memory/get_or_create.md")))
        .required(
            "name",
            AT::String,
            "the memory's handle, or an existing memory handle",
        )
        .optional(
            "content",
            AT::String,
            "an optional first entry, used only when the memory is created (ignored if it exists)",
        )
        .optional(
            "opts",
            object(),
            body_of(include_str!("prose/memory/get_or_create_opts.md")),
        )
        .returns(AT::Handle);

    let search = AE::new("memory.search")
        .description(body_of(include_str!("prose/memory/search.md")))
        .required("query", AT::String, "what to look for, in natural language")
        .optional(
            "opts",
            object()
                .optional(
                    "namespace",
                    AT::String,
                    render_placeholders(
                        body_of(include_str!("prose/memory/search_opts_namespace.md")),
                        &[("person", person)],
                    ),
                )
                .optional(
                    "tags",
                    AT::String.list(),
                    "tags to prefer; a result carrying more of them ranks higher",
                )
                .optional(
                    "limit",
                    AT::Integer,
                    "how many results to return (default 8)",
                ),
            "options",
        )
        .returns(AT::Object(Vec::new()).list());

    let list = AE::new("memory.list")
        .description(render_placeholders(
            body_of(include_str!("prose/memory/list.md")),
            &[("person", person)],
        ))
        .required(
            "prefix",
            AT::String,
            render_placeholders(
                body_of(include_str!("prose/memory/list_prefix.md")),
                &[("person", person)],
            ),
        )
        .returns(AT::Handle.list());

    let append = AE::new("<memory>:append")
        .description(body_of(include_str!("prose/memory/append.md")))
        .required("text", AT::String, "the entry text (must be under the character limit — summarize what you learned rather than pasting source content). To build it from a value, use a backtick string, which interpolates: `booked for {date}`. A plain quoted string does not — \"booked for {date}\" stores those braces literally, and is refused")
        .optional(
            "opts",
            object()
                .optional(
                    "by_agent",
                    AT::Boolean,
                    "record it as your own observation instead of the speaker's",
                )
                .optional(
                    "told_by",
                    AT::Handle,
                    body_of(include_str!("prose/memory/append_opts_told_by.md")),
                )
                .optional(
                    "visibility",
                    enum_of(["public", "private"]),
                    "force the visibility; required for an entry you author about a person",
                )
                .optional("exclude", AT::Handle.list(), body_of(include_str!("prose/memory/append_opts_exclude.md")))
                .optional(
                    "occurred_at",
                    object(),
                    render_placeholders(
                        body_of(include_str!("prose/memory/append_opts_occurred_at.md")),
                        &[("event", event)],
                    ),
                )
                .optional(
                    "distinct_from",
                    AT::Entry,
                    body_of(include_str!("prose/memory/append_opts_distinct_from.md")),
                ),
            "overrides",
        )
        .returns(AT::Entry);

    let attest = AE::new("<memory>:attest")
        .description(body_of(include_str!("prose/memory/attest.md")))
        .required(
            "entry",
            AT::Entry,
            body_of(include_str!("prose/memory/attest_entry.md")),
        )
        .optional(
            "opts",
            object()
                .optional(
                    "by_agent",
                    AT::Boolean,
                    "attest it as your own observation instead of the speaker's",
                )
                .optional(
                    "told_by",
                    AT::Handle,
                    body_of(include_str!("prose/memory/attest_opts_told_by.md")),
                )
                .optional(
                    "visibility",
                    enum_of(["public", "private"]),
                    "the attestation's posture; it may not be wider than the entry's own",
                )
                .optional(
                    "exclude",
                    AT::Handle.list(),
                    "withhold the attestation from named parties, like append's exclude",
                ),
            "overrides",
        )
        .returns(AT::Entry);

    let entries = AE::new("<memory>:entries")
        .description(body_of(include_str!("prose/memory/entries.md")))
        .returns(AT::Entry.list());

    let find_entry = AE::new("<memory>:find_entry")
        .description(body_of(include_str!("prose/memory/find_entry.md")))
        .required(
            "text",
            AT::String,
            body_of(include_str!("prose/memory/find_entry_text.md")),
        )
        .returns(AT::Entry.optional());

    let history = AE::new("<memory>:history")
        .description(body_of(include_str!("prose/memory/history.md")))
        .returns(AT::Entry.list());

    let details = AE::new("<memory>:details")
        .description(body_of(include_str!("prose/memory/details.md")))
        .returns(AT::String);

    let supersede = AE::new("<memory>:supersede")
        .description(body_of(include_str!("prose/memory/supersede.md")))
        .required(
            "old",
            AT::Entry,
            body_of(include_str!("prose/memory/supersede_old.md")),
        )
        .required(
            "new",
            AT::Entry,
            body_of(include_str!("prose/memory/supersede_new.md")),
        );

    let retract = AE::new("<memory>:retract")
        .description(body_of(include_str!("prose/memory/retract.md")))
        .required(
            "entry",
            AT::Entry,
            body_of(include_str!("prose/memory/retract_entry.md")),
        )
        .required(
            "reason",
            AT::String,
            "why the fact is being withdrawn — kept in history for audit",
        );

    let revise = AE::new("<memory>:revise")
        .description(body_of(include_str!("prose/memory/revise.md")))
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

    let rename = AE::new("<memory>:rename")
        .description(render_placeholders(
            body_of(include_str!("prose/memory/rename.md")),
            &[("person", person)],
        ))
        .required(
            "name",
            AT::String,
            format!("the new handle, e.g. \"{person}sarah\""),
        );

    let set_volatility = AE::new("<memory>:set_volatility")
        .description(body_of(include_str!("prose/memory/set_volatility.md")))
        .required(
            "level",
            enum_of(["low", "medium", "high"]),
            "the volatility level",
        );

    vec![
        create,
        get,
        get_or_create,
        search,
        list,
        append,
        attest,
        entries,
        find_entry,
        details,
        history,
        supersede,
        retract,
        revise,
        rename,
        set_volatility,
    ]
}

/// The merge entry (`<memory>:propose_merge`), gated on the `merging` feature.
pub(super) fn merge_entries() -> Vec<ApiEntry> {
    let person = Namespace::Person.prefix();
    let propose_merge = AE::new("<memory>:propose_merge")
        .description(render_placeholders(
            body_of(include_str!("prose/memory/propose_merge.md")),
            &[("person", person)],
        ))
        .required("other", AT::Handle, format!("the other {person} stub"))
        .optional(
            "opts",
            object().optional(
                "rationale",
                AT::String,
                body_of(include_str!("prose/memory/propose_merge_opts_rationale.md")),
            ),
            "options",
        );
    vec![propose_merge]
}

/// The `block.abort` entry — always on, infrastructure.
pub(super) fn block_entries() -> Vec<ApiEntry> {
    let abort = AE::new("block.abort")
        .description(body_of(include_str!("prose/memory/block_abort.md")))
        .optional("reason", AT::String, "why the block was abandoned");
    vec![abort]
}

/// The `turn.skip` entry — always on, infrastructure.
pub(super) fn turn_entries() -> Vec<ApiEntry> {
    let skip = AE::new("turn.skip")
        .description(body_of(include_str!("prose/memory/turn_skip.md")))
        .optional("reason", AT::String, "why the turn was skipped");
    vec![skip]
}
