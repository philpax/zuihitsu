//! Tag-related API reference entries: `<memory>:tag`, `:untag`, and the `tags.*` module
//! (`create`, `describe`, `list`).

use crate::agent::api_doc::{ApiEntry, ApiEntry as AE, ApiType as AT};

/// The tag handle methods, gated on the `tagging` feature.
pub(super) fn handle_methods() -> Vec<ApiEntry> {
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

    vec![tag, untag]
}

/// The `tags.*` module entries, gated on the `tagging` feature.
pub(super) fn module_entries() -> Vec<ApiEntry> {
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

    vec![tags_create, tags_describe, tags_list]
}
