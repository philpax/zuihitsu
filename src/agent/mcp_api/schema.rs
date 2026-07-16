//! JSON-Schema → system-prompt projection for MCP tools: the subset of a tool's `input_schema` the
//! prompt rendering understands, plus the allow/deny filter and the per-tool API entry builder.

use std::collections::BTreeMap;

use serde::Deserialize;

use crate::{
    agent::api_doc::{ApiEntry, ApiParam, ApiType},
    mcp::McpTool,
};

use crate::agent::mcp_api::lua::escape_tool_name;

/// Apply a server's `allow`/`deny` filter (spec §Allowlisting): the full advertised list, intersected
/// with `allow` (if present), minus `deny`, matching raw names case-sensitively. An `allow`/`deny`
/// entry that matches no advertised tool is a hard error (a stale policy must force a reconfirm).
pub(crate) fn filter_tools(
    tools: &[McpTool],
    allow: Option<&[String]>,
    deny: Option<&[String]>,
) -> Result<Vec<McpTool>, String> {
    for entry in allow
        .into_iter()
        .flatten()
        .chain(deny.into_iter().flatten())
    {
        if !tools.iter().any(|tool| &tool.name == entry) {
            return Err(format!(
                "allow/deny entry {entry:?} matches no advertised tool"
            ));
        }
    }
    Ok(tools
        .iter()
        .filter(|tool| {
            allow.is_none_or(|allow| allow.iter().any(|name| name == &tool.name))
                && !deny.is_some_and(|deny| deny.iter().any(|name| name == &tool.name))
        })
        .cloned()
        .collect())
}

/// One filtered tool as a system-prompt [`ApiEntry`]: `mcp.<server>.<escaped>`, its description, and
/// its arguments derived from the JSON-Schema input. MCP results vary (string or table), so no return
/// type is rendered.
pub(crate) fn tool_to_api_entry(server: &str, tool: &McpTool) -> ApiEntry {
    let schema: InputSchema = serde_json::from_value(tool.input_schema.clone()).unwrap_or_default();
    ApiEntry {
        call: format!("mcp.{server}.{}", escape_tool_name(&tool.name)),
        doc: tool.description.clone(),
        params: schema.params(),
        returns: ApiType::Nil,
        // MCP tools take one table of named arguments (the tool's JSON input), so the signature
        // renders as `mcp.server.tool{ … }`, not positional.
        table_args: true,
    }
}

/// The subset of a JSON-Schema object the prompt projection understands, deserialized from a tool's
/// `input_schema` so the converter walks typed fields rather than a raw `serde_json::Value`. Permissive
/// by design — every field defaults, unknown keywords are ignored, and `type` may be a name or a list —
/// so an unmodeled shape degrades to [`ApiType::Any`] rather than failing the parse.
#[derive(Deserialize, Default)]
#[serde(default)]
pub(crate) struct InputSchema {
    #[serde(rename = "type")]
    ty: Option<TypeSpec>,
    description: Option<String>,
    /// Enum literals are heterogeneous JSON; only string members render as an [`ApiType::Enum`].
    #[serde(rename = "enum")]
    enumeration: Vec<serde_json::Value>,
    properties: BTreeMap<String, InputSchema>,
    required: Vec<String>,
    items: Option<Box<InputSchema>>,
}

/// A JSON-Schema type name, parsed straight into a variant by serde (`#[serde(other)]` keeps an
/// unrecognized name from failing the whole schema parse).
#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
enum SchemaType {
    String,
    Integer,
    Number,
    Boolean,
    Array,
    Object,
    Null,
    #[serde(other)]
    Other,
}

/// A JSON-Schema `type`: a single name (`"string"`) or a list of them (`["string", "null"]`).
#[derive(Deserialize)]
#[serde(untagged)]
enum TypeSpec {
    One(SchemaType),
    Many(Vec<SchemaType>),
}

impl TypeSpec {
    /// The primary (non-`null`) type.
    fn primary(&self) -> Option<SchemaType> {
        match self {
            TypeSpec::One(ty) => Some(*ty),
            TypeSpec::Many(types) => types
                .iter()
                .copied()
                .find(|ty| !matches!(ty, SchemaType::Null)),
        }
    }
}

impl InputSchema {
    /// This object's properties as API parameters, each required per the schema's `required` list. A
    /// property-less schema yields none.
    pub(crate) fn params(&self) -> Vec<ApiParam> {
        self.properties
            .iter()
            .map(|(name, property)| ApiParam {
                name: name.clone(),
                ty: property.api_type(),
                required: self.required.iter().any(|entry| entry == name),
                doc: property.description.clone().unwrap_or_default(),
            })
            .collect()
    }

    /// This node as an [`ApiType`], best-effort: a string enum, then the scalar/array/object shapes,
    /// falling back to `Any` for anything unmodeled.
    fn api_type(&self) -> ApiType {
        let labels: Vec<String> = self
            .enumeration
            .iter()
            .filter_map(|value| value.as_str().map(str::to_owned))
            .collect();
        if !labels.is_empty() {
            return ApiType::Enum(labels);
        }
        match self.ty.as_ref().and_then(TypeSpec::primary) {
            Some(SchemaType::String) => ApiType::String,
            Some(SchemaType::Integer) => ApiType::Integer,
            Some(SchemaType::Number) => ApiType::Number,
            Some(SchemaType::Boolean) => ApiType::Boolean,
            Some(SchemaType::Array) => ApiType::List(Box::new(
                self.items
                    .as_deref()
                    .map(InputSchema::api_type)
                    .unwrap_or(ApiType::Any),
            )),
            Some(SchemaType::Object) => ApiType::Object(self.params()),
            Some(SchemaType::Null | SchemaType::Other) | None => ApiType::Any,
        }
    }
}
