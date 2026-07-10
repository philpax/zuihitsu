//! Link-related API reference entries: `<memory>:link`, `:unlink`, `:outgoing`, `:incoming`, `:links`,
//! and the `links.*` module (`register`, `list`, `get`).

use super::super::super::api_doc::{ApiEntry, ApiEntry as AE, ApiType as AT, enum_of, object};

/// The link handle methods, gated on the `linking` feature.
pub(super) fn handle_methods() -> Vec<ApiEntry> {
    let link = AE::new("<memory>:link")
        .description(
            "Record a relationship between this memory and another under a registered relation. When \
             two memories relate — two people who know each other, an event that belongs to a topic — \
             capture it with link rather than only describing it in prose, so the connection is \
             queryable and traversable (pick the fitting relation from the registry). The relation \
             reads as a sentence from this memory to the other: person:link(\"participates_in\", \
             event) records that the person participates in the event. Linking from the other side, \
             use the registry's inverse label — event:link(\"has_participant\", person) — rather \
             than the forward label, which would record the relationship backwards. For a symmetric \
             relation (shown in the registry), link once — the reverse direction is implied. A \
             relationship you record about someone — a belief, a judgment — defaults private to the \
             teller when a participant asserts it, so an aside about B stays hidden from B; a relayed \
             fact (the teller is neither endpoint) surfaces to anyone carrying provenance. Force the \
             posture with opts.visibility when the default does not fit.",
        )
        .required("relation", AT::String, "the relation from the registry, e.g. \"part_of\"")
        .required(
            "other",
            AT::Handle,
            "the memory to link to — a handle (e.g. context.current()) or its name as a string, \
             which is looked up",
        )
        .optional(
            "opts",
            object().optional(
                "visibility",
                enum_of(["public", "attributed", "private"]),
                "force the link's visibility instead of the write-time default — same postures as \
                 content: public, attributed (secondhand), or private (teller-gated, subject-guarded \
                 at the target)",
            ),
            "overrides for the link — visibility forces the posture instead of the write-time default",
        );

    let unlink = AE::new("<memory>:unlink")
        .description("Remove a link made with <memory>:link when the relationship no longer holds.")
        .required("relation", AT::String, "the relation")
        .required(
            "other",
            AT::Handle,
            "the memory the link points to — a handle or its name as a string, which is looked up",
        );

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

    vec![link, unlink, outgoing, incoming, links]
}

/// The `links.*` module entries, gated on the `linking` feature.
pub(super) fn module_entries() -> Vec<ApiEntry> {
    let links_register = AE::new("links.register")
        .description(
            "Register a link relation, usable thereafter under either label by <memory>:link — this \
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
             <memory>:link accepts.",
        )
        .returns(AT::Object(Vec::new()).list());

    let links_get = AE::new("links.get")
        .description("One registered relation by either label, or nil if it is not registered.")
        .required("name", AT::String, "the relation or its inverse label")
        .returns(AT::Object(Vec::new()).optional());

    vec![links_register, links_list, links_get]
}
