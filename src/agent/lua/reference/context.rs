//! Context and conversation API reference entries: `context.current` and `convo.turn`.

use crate::{
    agent::{
        api_doc::{ApiEntry, ApiEntry as AE, ApiType as AT},
        body_of, render_placeholders,
    },
    ids::Namespace,
};

/// The always-on context entry.
pub(super) fn entries() -> Vec<ApiEntry> {
    let context = Namespace::Context.prefix();
    let context_current = AE::new("context.current")
        .description(render_placeholders(
            body_of(include_str!("prose/context/current.md")),
            &[("context", context)],
        ))
        .returns(AT::Handle.optional());

    vec![context_current]
}

/// The `convo.turn` entry, gated on the `transcripts` feature.
pub(super) fn convo_entries() -> Vec<ApiEntry> {
    let convo_turn = AE::new("convo.turn")
        .description(body_of(include_str!("prose/context/turn.md")))
        .required(
            "id",
            AT::String,
            "the turn id — the value inside a [turn:<id>] token",
        )
        .returns(AT::Object(Vec::new()));

    vec![convo_turn]
}
