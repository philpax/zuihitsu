//! Context and conversation API reference entries: `context.current` and `convo.turn`.

use super::super::super::api_doc::{ApiEntry, ApiEntry as AE, ApiType as AT};
use crate::ids::Namespace;

/// The always-on context entry.
pub(super) fn entries() -> Vec<ApiEntry> {
    let context = Namespace::Context.prefix();
    let context_current = AE::new("context.current")
        .description(format!(
            "The {context}* memory for the current conversation. Check its #confidential tag to \
                 know whether the room is confidential."
        ))
        .returns(AT::Handle.optional());

    vec![context_current]
}

/// The `convo.turn` entry, gated on the `transcripts` feature.
pub(super) fn convo_entries() -> Vec<ApiEntry> {
    let convo_turn = AE::new("convo.turn")
        .description(
            "Resolve a reference to an earlier moment — a [turn:<id>] token, pass the id here — to \
             that turn and the exchange around it. The result is a table { id, ref, text, speaker, \
             role, at, window }: ref is the canonical [turn:<id>] to cite it by (copy it into your \
             reply), window the surrounding turns (the linked one flagged focused), printing as a \
             transcript excerpt with the moment marked. A moment resolves only when everyone present \
             here was in its audience; otherwise it is an error naming the audience problem — recall \
             through memory instead of replaying the transcript. A malformed or unknown id is \
             likewise an error.",
        )
        .required(
            "id",
            AT::String,
            "the turn id — the value inside a [turn:<id>] token",
        )
        .returns(AT::Object(Vec::new()));

    vec![convo_turn]
}
