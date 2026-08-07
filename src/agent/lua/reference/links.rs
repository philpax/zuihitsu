//! Link-related API reference entries: the `<memory>:outgoing`/`:incoming`/`:links` readers, and
//! the `links.*` module (`create`, `remove`, `register`, `list`, `get`).

use crate::agent::{
    api_doc::{ApiEntry, ApiEntry as AE, ApiType as AT, enum_of, object},
    body_of,
};

/// The link reader handle methods (`:outgoing`/`:incoming`/`:links`), gated on the `linking`
/// feature. The link *writers* are `links.*` module functions (see [`module_entries`]).
pub(super) fn handle_methods() -> Vec<ApiEntry> {
    let outgoing = AE::new("<memory>:outgoing")
        .description(body_of(include_str!("prose/links/outgoing.md")))
        .required(
            "relation",
            AT::String,
            "the relation from the registry, e.g. \"knows\"",
        )
        .returns(AT::Object(Vec::new()).list());

    let incoming = AE::new("<memory>:incoming")
        .description(body_of(include_str!("prose/links/incoming.md")))
        .required(
            "relation",
            AT::String,
            "the relation from the registry, e.g. \"knows\"",
        )
        .returns(AT::Object(Vec::new()).list());

    let links = AE::new("<memory>:links")
        .description(body_of(include_str!("prose/links/links.md")))
        .returns(AT::Object(Vec::new()).list());

    vec![outgoing, incoming, links]
}

/// The `links.*` module entries, gated on the `linking` feature — the `create`/`remove` edge writers
/// and the `register`/`list`/`get` registry.
pub(super) fn module_entries() -> Vec<ApiEntry> {
    let links_create = AE::new("links.create")
        .description(body_of(include_str!("prose/links/create.md")))
        .required(
            "subject",
            AT::Handle,
            body_of(include_str!("prose/links/create_subject.md")),
        )
        .required(
            "relation",
            AT::String,
            "the relation from the registry, e.g. \"part_of\"",
        )
        .required(
            "object",
            AT::Handle,
            body_of(include_str!("prose/links/create_object.md")),
        )
        .optional(
            "opts",
            object()
                .optional(
                    "visibility",
                    enum_of(["public", "attributed", "private"]),
                    body_of(include_str!("prose/links/create_opts_visibility.md")),
                )
                .optional(
                    "exclude",
                    AT::Handle.list(),
                    body_of(include_str!("prose/links/create_opts_exclude.md")),
                ),
            body_of(include_str!("prose/links/create_opts.md")),
        );

    let links_remove = AE::new("links.remove")
        .description(body_of(include_str!("prose/links/remove.md")))
        .required(
            "subject",
            AT::Handle,
            "the memory the relation runs from — a handle or its name as a string, which is looked up",
        )
        .required("relation", AT::String, "the relation")
        .required(
            "object",
            AT::Handle,
            "the memory the relation runs to — a handle or its name as a string, which is looked up",
        );

    let links_register = AE::new("links.register")
        .description(body_of(include_str!("prose/links/register.md")))
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
        .description(body_of(include_str!("prose/links/list.md")))
        .returns(AT::Object(Vec::new()).list());

    let links_get = AE::new("links.get")
        .description(body_of(include_str!("prose/links/get.md")))
        .required("name", AT::String, "the relation or its inverse label")
        .returns(AT::Object(Vec::new()).optional());

    vec![
        links_create,
        links_remove,
        links_register,
        links_list,
        links_get,
    ]
}
