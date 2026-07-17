//! The structured Lua API description: a renderable catalogue of a callable's parameters, their
//! types, and its return, shared by the hand-written Lua API ([`crate`]'s consumer builds it through
//! these builders) and the MCP tools projected from their JSON-Schema inputs. Both produce the same
//! [`ApiEntry`] shape, so the system prompt's API description and the console's reference are one
//! consistent catalogue regardless of where a call originates. These types cross the wire to the
//! console (`GET /control/lua-api`), so they live here; the main crate renders them to prompt text.
//!
//! The type vocabulary ([`ApiType`]) is deliberately broad enough to map a JSON Schema onto it
//! losslessly for the common cases: `string`/`integer`/`number`/`boolean`, arrays, objects with
//! named fields, string enums, and nullability.

use serde::Serialize;

/// A parameter (or object field) type. Broad enough to express the Lua API and a JSON-Schema input.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub enum ApiType {
    String,
    Integer,
    Number,
    Boolean,
    /// A memory handle — the object the memory API returns.
    Handle,
    /// An entry handle — an addressable content entry that reads as its text (returned by
    /// `mem:append` / `mem:entries` / `mem:history`, passed to `mem:supersede`).
    Entry,
    /// A table / object with named fields (an opts table, or a JSON-Schema `object`).
    Object(Vec<ApiParam>),
    /// A list of elements of the given type.
    List(Box<ApiType>),
    /// One of a fixed set of string values.
    Enum(Vec<String>),
    /// A value that may be absent (nil / not present).
    Optional(Box<ApiType>),
    /// No value.
    Nil,
    /// Unconstrained.
    Any,
}

impl ApiType {
    /// This type, made nil-able: `ApiType::Handle.optional()` → "memory handle or nil".
    pub fn optional(self) -> ApiType {
        ApiType::Optional(Box::new(self))
    }

    /// A list of this type: `ApiType::String.list()` → "list of string".
    pub fn list(self) -> ApiType {
        ApiType::List(Box::new(self))
    }
}

/// A string-enum type from a list of literals: `enum_of(["public", "private"])`.
pub fn enum_of<I, S>(values: I) -> ApiType
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    ApiType::Enum(values.into_iter().map(Into::into).collect())
}

/// Start building a table/object type: `object().optional("by_agent", ApiType::Boolean, "…")`. The
/// result coerces to an [`ApiType`] wherever one is expected (it implements `Into<ApiType>`), so it
/// drops straight into a parameter's type.
pub fn object() -> ObjectBuilder {
    ObjectBuilder { fields: Vec::new() }
}

/// One named parameter (or object field): its type, whether it is required, and a one-line doc.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ApiParam {
    pub name: String,
    pub ty: ApiType,
    pub required: bool,
    pub doc: String,
}

/// A fluent builder for an [`ApiType::Object`], so nested option tables read as a chain rather than
/// a hand-built `vec![ApiParam { … }]`.
#[derive(Clone, Debug, PartialEq)]
pub struct ObjectBuilder {
    fields: Vec<ApiParam>,
}

impl ObjectBuilder {
    pub fn required(
        mut self,
        name: impl Into<String>,
        ty: impl Into<ApiType>,
        doc: impl Into<String>,
    ) -> ObjectBuilder {
        self.fields.push(field(name, ty, true, doc));
        self
    }

    pub fn optional(
        mut self,
        name: impl Into<String>,
        ty: impl Into<ApiType>,
        doc: impl Into<String>,
    ) -> ObjectBuilder {
        self.fields.push(field(name, ty, false, doc));
        self
    }
}

impl From<ObjectBuilder> for ApiType {
    fn from(builder: ObjectBuilder) -> ApiType {
        ApiType::Object(builder.fields)
    }
}

/// The runtime opt-in a call depends on, for surfaces that gate outward reach. In a live turn every
/// projection the agent sees is already connected, so this is advisory metadata the renderer ignores;
/// the operator Lua console reads it to mark which calls need their toggle before they will run.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub enum ApiGate {
    /// Needs the web fetcher — the `allow_web` opt-in (`web.markdown`).
    Web,
    /// Needs a connected MCP host — the `allow_mcp` opt-in (`mcp.<server>.*`).
    Mcp,
}

/// One callable: its call form, what it does, its parameters, and what it returns. Built fluently —
/// `ApiEntry::new(call).description(…).required(…).optional(…).returns(…)` — defaulting to an empty
/// description, no parameters, and a `nil` return.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ApiEntry {
    pub call: String,
    pub doc: String,
    pub params: Vec<ApiParam>,
    pub returns: ApiType,
    /// The params are passed as one table literal (`call{ field = … }`) rather than positional
    /// arguments — the calling convention of the MCP projection, where the table is the tool's JSON
    /// input. The signature renders with braces so the convention is unmistakable.
    pub table_args: bool,
    /// The runtime opt-in this call depends on, or `None` for an always-available call. Set for the
    /// outward-reaching projections (`web.markdown`, the MCP tools) so the console can mark them.
    pub gate: Option<ApiGate>,
}

impl ApiEntry {
    pub fn new(call: impl Into<String>) -> ApiEntry {
        ApiEntry {
            call: call.into(),
            doc: String::new(),
            params: Vec::new(),
            returns: ApiType::Nil,
            table_args: false,
            gate: None,
        }
    }

    /// Mark this call as gated on a runtime opt-in (see [`ApiGate`]).
    pub fn gated(mut self, gate: ApiGate) -> ApiEntry {
        self.gate = Some(gate);
        self
    }

    pub fn description(mut self, doc: impl Into<String>) -> ApiEntry {
        self.doc = doc.into();
        self
    }

    pub fn required(
        mut self,
        name: impl Into<String>,
        ty: impl Into<ApiType>,
        doc: impl Into<String>,
    ) -> ApiEntry {
        self.params.push(field(name, ty, true, doc));
        self
    }

    pub fn optional(
        mut self,
        name: impl Into<String>,
        ty: impl Into<ApiType>,
        doc: impl Into<String>,
    ) -> ApiEntry {
        self.params.push(field(name, ty, false, doc));
        self
    }

    pub fn returns(mut self, ty: impl Into<ApiType>) -> ApiEntry {
        self.returns = ty.into();
        self
    }
}

fn field(
    name: impl Into<String>,
    ty: impl Into<ApiType>,
    required: bool,
    doc: impl Into<String>,
) -> ApiParam {
    ApiParam {
        name: name.into(),
        ty: ty.into(),
        required,
        doc: doc.into(),
    }
}
