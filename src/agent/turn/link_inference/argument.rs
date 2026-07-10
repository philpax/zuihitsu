//! The schema types and lenient parser for the structured link-inference reply.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The `link_inference` reply shape; doubles as the schema sent to the model, so prompt and parser
/// cannot drift. Constructed directly by tests so the JSON a test emits cannot drift from the schema.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct LinkInferenceArgs {
    /// New relation types to register before creating links of that type. May be empty.
    #[serde(default)]
    pub new_relations: Vec<NewRelationSpec>,
    /// Relationships to create. May be empty.
    #[serde(default)]
    pub links: Vec<InferredLink>,
}

/// A relation the model coins for a relationship no registered relation fits.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct NewRelationSpec {
    pub name: String,
    pub inverse: String,
    pub from_card: String,
    pub to_card: String,
    pub symmetric: bool,
    pub reflexive: bool,
    /// A one-line purpose so the agent knows when to use the relation.
    #[serde(default)]
    pub description: String,
}

/// A relationship the model identifies, grounded in a numbered statement. The three fields read as
/// a literal sentence — "`subject` `relation` `object`" — so the direction is carried by the
/// sentence itself, never by a separate flag: "person/theo mentored_by person/clara" says Theo is
/// mentored by Clara. An abstract direction field detached from the sentence invited inversions.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct InferredLink {
    /// The statement number (1-based) that grounds this relationship.
    #[allow(dead_code)]
    pub entry: usize,
    /// The sentence's subject: a memory handle, e.g. "person/theo". One of `subject` and `object`
    /// must be the memory under consideration; the other must be one of the listed candidates.
    pub subject: String,
    /// The relation label, read in the sentence's active direction. Must be a registered relation
    /// or one in `new_relations`.
    pub relation: String,
    /// The sentence's object: a memory handle, e.g. "person/clara".
    pub object: String,
}

/// Parse a structured reply leniently. A well-formed `links` array with a malformed `new_relations`
/// entry still produces the links that do not need a new relation, rather than discarding the whole
/// reply on one bad field — the same salvage discipline as `synthesize_argument`. The caller is
/// responsible for extracting the JSON object from the model's fenced reply; this function takes the
/// parsed `Value` and salvages each field independently.
pub(super) fn link_inference_argument(value: &serde_json::Value) -> Option<LinkInferenceArgs> {
    let new_relations = value
        .get("new_relations")
        .and_then(serde_json::Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| serde_json::from_value::<NewRelationSpec>(item.clone()).ok())
                .collect()
        })
        .unwrap_or_default();
    let links = value
        .get("links")
        .and_then(serde_json::Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| serde_json::from_value::<InferredLink>(item.clone()).ok())
                .collect()
        })
        .unwrap_or_default();
    Some(LinkInferenceArgs {
        new_relations,
        links,
    })
}
