//! Lua marshalling for MCP: argument table → JSON, tool result → Lua value, and tool-name escaping
//! into valid Lua identifiers.

use std::{collections::HashMap, sync::Arc};

use mlua::{Function, Lua, LuaSerdeExt, Table, Value};

use crate::{
    agent::mcp_api::McpSession,
    mcp::{ContentBlock, McpError, McpOutput, McpTool},
};

/// Marshal a Lua argument table to JSON-RPC `arguments` (spec §Calling): a consecutive-integer-key
/// table becomes a JSON array, otherwise an object; Lua integers serialize as JSON integers. The empty
/// table is the no-argument case and must be an object (tool arguments are always a top-level object),
/// but the serde bridge renders an empty table as `[]`, so it is mapped back to `{}`.
pub(crate) fn lua_args_to_json(lua: &Lua, args: Table) -> mlua::Result<serde_json::Value> {
    let value: serde_json::Value = lua.from_value(Value::Table(args))?;
    Ok(match value {
        serde_json::Value::Array(items) if items.is_empty() => {
            serde_json::Value::Object(serde_json::Map::new())
        }
        other => other,
    })
}

/// Project a tool result back to Lua (spec §results): an all-text result with no `structuredContent`
/// returns a bare string (text blocks joined by `\n`); anything else returns a table
/// `{ content = { <block>, … }, structured = <decoded or nil> }`.
pub(crate) fn project_output(lua: &Lua, output: McpOutput) -> mlua::Result<Value> {
    let McpOutput {
        content,
        structured,
    } = output;
    let all_text = structured.is_none()
        && content
            .iter()
            .all(|block| matches!(block, ContentBlock::Text { .. }));
    if all_text {
        let joined = content
            .iter()
            .map(|block| match block {
                ContentBlock::Text { text } => text.as_str(),
                ContentBlock::Other(_) => "",
            })
            .collect::<Vec<_>>()
            .join("\n");
        return Ok(Value::String(lua.create_string(&joined)?));
    }

    let blocks = lua.create_table()?;
    for (index, block) in content.into_iter().enumerate() {
        let rendered = match block {
            ContentBlock::Text { text } => {
                let table = lua.create_table()?;
                table.set("type", "text")?;
                table.set("text", text)?;
                Value::Table(table)
            }
            ContentBlock::Other(value) => lua.to_value(&value)?,
        };
        blocks.set(index + 1, rendered)?;
    }
    let table = lua.create_table()?;
    table.set("content", blocks)?;
    let structured = match structured {
        Some(value) => lua.to_value(&value)?,
        None => Value::Nil,
    };
    table.set("structured", structured)?;
    Ok(Value::Table(table))
}

/// The escaped→raw tool-name map for a server's catalogue. Two tools that escape to the same Lua name
/// are a hard error — the operator must rename or `deny` one.
pub(crate) fn build_escape_map(tools: &[McpTool]) -> Result<HashMap<String, String>, String> {
    let mut map = HashMap::new();
    for tool in tools {
        let escaped = escape_tool_name(&tool.name);
        if let Some(existing) = map.insert(escaped.clone(), tool.name.clone()) {
            return Err(format!(
                "two tools escape to the same Lua name {escaped:?}: {existing:?} and {:?}",
                tool.name
            ));
        }
    }
    Ok(map)
}

/// Escape a raw MCP tool name into a callable Lua identifier (spec §Tool names escaped into valid
/// Lua): characters illegal in a Lua identifier — including a leading digit — map to `_`, then a Lua
/// keyword takes a trailing `_` (`goto` → `goto_`).
pub(crate) fn escape_tool_name(raw: &str) -> String {
    let mut escaped: String = raw
        .chars()
        .enumerate()
        .map(|(index, ch)| {
            let legal = ch == '_' || ch.is_ascii_alphabetic() || (index > 0 && ch.is_ascii_digit());
            if legal { ch } else { '_' }
        })
        .collect();
    if is_lua_keyword(&escaped) {
        escaped.push('_');
    }
    escaped
}

/// Whether `word` is a Luau reserved keyword. Luau does not reserve `goto` (it has no goto), but
/// keeping it in the set is a harmless over-escape — a superset of the reserved words only ever
/// suffixes a tool name that did not strictly need it.
fn is_lua_keyword(word: &str) -> bool {
    matches!(
        word,
        "and"
            | "break"
            | "do"
            | "else"
            | "elseif"
            | "end"
            | "false"
            | "for"
            | "function"
            | "goto"
            | "if"
            | "in"
            | "local"
            | "nil"
            | "not"
            | "or"
            | "repeat"
            | "return"
            | "then"
            | "true"
            | "until"
            | "while"
    )
}

/// Render an [`McpError`] as the catchable Lua error the agent sees — its `Display` is the
/// agent-facing wording (teachable variants are unprefixed prose, infra variants carry `mcp:`).
pub(crate) fn mcp_to_lua(error: McpError) -> mlua::Error {
    mlua::Error::RuntimeError(error.to_string())
}

/// The async function for one `(server, escaped tool)`: on call it marshals the argument table, runs
/// the tool, and projects the result (see [`McpSession::call`]).
pub(crate) fn tool_function(
    lua: &Lua,
    mcp: &Arc<McpSession>,
    server: &str,
    tool: String,
) -> mlua::Result<Function> {
    let mcp = mcp.clone();
    let server = server.to_owned();
    // Take the argument as a raw value rather than a `Table`, so a positional call (the natural mistake
    // — the tool takes one table of named arguments) gets a pointed error instead of mlua's opaque
    // "error converting … to table". A bare call with no arguments is an empty table, so a tool whose
    // fields are all optional can be invoked plainly.
    lua.create_async_function(move |lua, args: Value| {
        let mcp = mcp.clone();
        let server = server.clone();
        let tool = tool.clone();
        async move {
            let args = match args {
                Value::Table(table) => table,
                Value::Nil => lua.create_table()?,
                other => {
                    return Err(mlua::Error::RuntimeError(format!(
                        "mcp.{server}.{tool} takes a single table of named arguments, e.g. \
                         mcp.{server}.{tool}{{ url = \"…\" }} — got a {}",
                        other.type_name()
                    )));
                }
            };
            mcp.call(&lua, &server, &tool, args).await
        }
    })
}
