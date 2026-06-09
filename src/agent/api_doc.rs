//! A structured, renderable description of a callable API — its parameters, their types, and its
//! return — shared by the hand-written Lua API ([`crate::agent::lua::api_reference`]) and, later, by MCP
//! tools projected from their JSON-Schema inputs (spec §External I/O via MCP). Both produce the same
//! [`ApiEntry`] shape and render through [`render`], so the system prompt's API description is one
//! consistent catalogue regardless of where a call originates.
//!
//! The type vocabulary ([`ApiType`]) is deliberately broad enough to map a JSON Schema onto it
//! losslessly for the common cases: `string`/`integer`/`number`/`boolean`, arrays, objects with
//! named fields, string enums, and nullability.

use std::fmt::Write as _;

/// A parameter (or object field) type. Broad enough to express the Lua API and a JSON-Schema input.
#[derive(Clone, Debug, PartialEq)]
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

    /// The short rendered name (`string`, `"a" | "b"`, `list of string`, `string or nil`, …). An
    /// `Object`'s fields render separately, as dotted parameters, so here it is just `table`.
    fn label(&self) -> String {
        match self {
            ApiType::String => "string".to_owned(),
            ApiType::Integer => "integer".to_owned(),
            ApiType::Number => "number".to_owned(),
            ApiType::Boolean => "boolean".to_owned(),
            ApiType::Handle => "memory handle".to_owned(),
            ApiType::Entry => "entry".to_owned(),
            ApiType::Object(_) => "table".to_owned(),
            ApiType::List(inner) => format!("list of {}", inner.label()),
            ApiType::Enum(values) => values
                .iter()
                .map(|value| format!("\"{value}\""))
                .collect::<Vec<_>>()
                .join(" | "),
            ApiType::Optional(inner) => format!("{} or nil", inner.label()),
            ApiType::Nil => "nil".to_owned(),
            ApiType::Any => "any".to_owned(),
        }
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
#[derive(Clone, Debug, PartialEq)]
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

/// One callable: its call form, what it does, its parameters, and what it returns. Built fluently —
/// `ApiEntry::new(call).description(…).required(…).optional(…).returns(…)` — defaulting to an empty
/// description, no parameters, and a `nil` return.
#[derive(Clone, Debug, PartialEq)]
pub struct ApiEntry {
    pub call: String,
    pub doc: String,
    pub params: Vec<ApiParam>,
    pub returns: ApiType,
}

impl ApiEntry {
    pub fn new(call: impl Into<String>) -> ApiEntry {
        ApiEntry {
            call: call.into(),
            doc: String::new(),
            params: Vec::new(),
            returns: ApiType::Nil,
        }
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

/// Render a catalogue into the system prompt's API-description block: a signature line per call
/// (optional parameters marked with `?`), then each parameter typed and dotted (an object's fields
/// as `opts.field`), then the return type when it is not `nil`.
pub fn render(entries: &[ApiEntry]) -> String {
    let mut out = String::new();
    for entry in entries {
        let signature = entry
            .params
            .iter()
            .map(|param| {
                if param.required {
                    param.name.clone()
                } else {
                    format!("{}?", param.name)
                }
            })
            .collect::<Vec<_>>()
            .join(", ");
        if entry.doc.is_empty() {
            let _ = writeln!(out, "{}({signature})", entry.call);
        } else {
            let _ = writeln!(out, "{}({signature}) — {}", entry.call, entry.doc);
        }
        for param in &entry.params {
            render_param(&mut out, "", param);
        }
        if entry.returns != ApiType::Nil {
            let _ = writeln!(out, "    → {}", entry.returns.label());
        }
    }
    out
}

/// Render one parameter line, recursing into an object's fields with a dotted name prefix.
fn render_param(out: &mut String, prefix: &str, param: &ApiParam) {
    let required = if param.required { " (required)" } else { "" };
    let _ = writeln!(
        out,
        "    {prefix}{}: {}{required} — {}",
        param.name,
        param.ty.label(),
        param.doc
    );
    if let ApiType::Object(fields) = &param.ty {
        for field in fields {
            render_param(out, &format!("{prefix}{}.", param.name), field);
        }
    }
}
