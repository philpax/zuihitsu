//! Link-related API reference entries: the `<memory>:outgoing`/`:incoming`/`:links` readers, and
//! the `links.*` module (`create`, `remove`, `register`, `list`, `get`).

use crate::agent::api_doc::{ApiEntry, ApiEntry as AE, ApiType as AT, enum_of, object};

/// The link reader handle methods (`:outgoing`/`:incoming`/`:links`), gated on the `linking`
/// feature. The link *writers* are `links.*` module functions (see [`module_entries`]).
pub(super) fn handle_methods() -> Vec<ApiEntry> {
    let outgoing = AE::new("<memory>:outgoing")
        .description(
            "The memories this one links to under a relation, across its whole merged identity, \
             forward — <memory>:outgoing(\"knows\") is who it knows. Each result is a table \
             { relation, memory, name, direction, source, told_by, occurred_at } printing as \
             \"relation → name\" (a dated occurrence appended); reach the linked memory through \
             result.memory, result.told_by is who asserted the relationship, result.occurred_at the \
             far memory's date when dated. Use <memory>:incoming for the reverse (who knows it). For \
             a symmetric relation, both return the same neighbours. A stored edge's direction \
             reflects how the fact was told, so for a who-is-connected question, prefer \
             <memory>:links or <memory>:details, which read both directions — betting on one \
             direction can miss edges told the other way. Private links are filtered out when an \
             audience is present, mirroring content entry reads.",
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
             memory, name, direction, source, occurred_at } printing as \"relation → name\" (or \
             \"← name\" incoming), a dated occurrence appended; reach a linked memory through \
             result.memory, and each result renders as its own text for interpolation into a reply. \
             Call it with a colon — <memory>:links() — since <memory>.links is the method itself. \
             Private links are filtered out when an audience is present, mirroring content entry reads.",
        )
        .returns(AT::Object(Vec::new()).list());

    vec![outgoing, incoming, links]
}

/// The `links.*` module entries, gated on the `linking` feature — the `create`/`remove` edge writers
/// and the `register`/`list`/`get` registry.
pub(super) fn module_entries() -> Vec<ApiEntry> {
    let links_create = AE::new("links.create")
        .description(
            "Record a relationship between two memories under a registered relation. When two \
             memories relate — two people who know each other, an event that belongs to a topic — \
             capture it with links.create rather than only describing it in prose, so the connection \
             is queryable and traversable (pick the fitting relation from the registry). The \
             arguments read as a sentence: links.create(person, \"participates_in\", event) records \
             that the person participates in the event — give the subject first, then the relation, \
             then the object, and the edge is stored subject → object. The registry's inverse label \
             names the same edge read the other way, so the readers surface it from the object's side \
             too (that event's incoming participants). For a symmetric relation (shown in the \
             registry), create it once — the reverse direction is implied. A relationship you record \
             about someone — a belief, a judgment — defaults private to the teller when a participant \
             asserts it, so an aside about B stays hidden from B; a relayed fact (the teller is \
             neither endpoint) surfaces to anyone carrying provenance. Force the posture with \
             opts.visibility when the default does not fit.",
        )
        .required(
            "subject",
            AT::Handle,
            "the memory the relation runs from — a handle (e.g. context.current()) or its name as a \
             string, which is looked up",
        )
        .required("relation", AT::String, "the relation from the registry, e.g. \"part_of\"")
        .required(
            "object",
            AT::Handle,
            "the memory the relation runs to — a handle or its name as a string, which is looked up",
        )
        .optional(
            "opts",
            object()
                .optional(
                    "visibility",
                    enum_of(["public", "attributed", "private"]),
                    "force the link's visibility instead of the write-time default — same postures as \
                     content: public, attributed (secondhand), or private (teller-gated, \
                     subject-guarded at the target)",
                )
                .optional(
                    "exclude",
                    AT::Handle.list(),
                    "record the link as a confidence additionally withheld whenever any named party \
                     is present — a list of person handles or names to keep it from, on top of the \
                     private posture. Mutually exclusive with visibility",
                ),
            "overrides for the link — visibility or exclude forces the posture instead of the \
             write-time default",
        );

    let links_remove = AE::new("links.remove")
        .description(
            "Remove a link made with links.create when the relationship no longer holds; name the \
             same subject, relation, and object.",
        )
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
        .description(
            "Register a link relation, usable thereafter under either label by links.create — this \
             declares the relation that edges instantiate. Re-registering a name updates it.",
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
             links.create accepts.",
        )
        .returns(AT::Object(Vec::new()).list());

    let links_get = AE::new("links.get")
        .description("One registered relation by either label, or nil if it is not registered.")
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
