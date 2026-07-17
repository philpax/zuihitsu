//! Rendering the structured Lua API description into the system prompt's API block. The description
//! types themselves — [`ApiEntry`] and its vocabulary — live in `zuihitsu_frontend_types::api`, since
//! they cross the wire to the console (`GET /control/lua-api`); this module re-exports them so
//! `crate::agent::api_doc::*` stays the one path the agent code builds and reads them through, and
//! owns [`render`], the text projection the agent's prompt is built from.

use std::fmt::Write as _;

pub use zuihitsu_frontend_types::api::{
    ApiEntry, ApiGate, ApiParam, ApiType, ObjectBuilder, enum_of, object,
};

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
        // MCP tools are called with one table of named arguments; the others take positional arguments.
        // Brace the signature for the former so the agent passes a table, not positional args.
        let signature = if entry.table_args {
            format!("{{ {signature} }}")
        } else {
            format!("({signature})")
        };
        if entry.doc.is_empty() {
            let _ = writeln!(out, "{}{signature}", entry.call);
        } else {
            let _ = writeln!(out, "{}{signature} — {}", entry.call, entry.doc);
        }
        for param in &entry.params {
            render_param(&mut out, "", param);
        }
        if entry.returns != ApiType::Nil {
            let _ = writeln!(out, "    → {}", label(&entry.returns));
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
        label(&param.ty),
        param.doc
    );
    if let ApiType::Object(fields) = &param.ty {
        for field in fields {
            render_param(out, &format!("{prefix}{}.", param.name), field);
        }
    }
}

/// The short rendered name of a type (`string`, `"a" | "b"`, `list of string`, `string or nil`, …).
/// An `Object`'s fields render separately, as dotted parameters, so here it is just `table`.
fn label(ty: &ApiType) -> String {
    match ty {
        ApiType::String => "string".to_owned(),
        ApiType::Integer => "integer".to_owned(),
        ApiType::Number => "number".to_owned(),
        ApiType::Boolean => "boolean".to_owned(),
        ApiType::Handle => "memory handle".to_owned(),
        ApiType::Entry => "entry".to_owned(),
        ApiType::Object(_) => "table".to_owned(),
        ApiType::List(inner) => format!("list of {}", label(inner)),
        ApiType::Enum(values) => values
            .iter()
            .map(|value| format!("\"{value}\""))
            .collect::<Vec<_>>()
            .join(" | "),
        ApiType::Optional(inner) => format!("{} or nil", label(inner)),
        ApiType::Nil => "nil".to_owned(),
        ApiType::Any => "any".to_owned(),
    }
}
