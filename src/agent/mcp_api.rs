//! The `mcp.<server>.<tool>{ ... }` projection: each configured MCP server's tools surfaced into the
//! session VM as one async Lua function apiece (spec §External I/O via MCP).
//!
//! A server instance is owned by the session VM and spawned lazily on first use (most sessions never
//! browse, so most never spawn anything); the `server → instance` map lives here, on the
//! [`McpSession`] the VM holds for its whole lifetime. Because the VM runs its blocks one at a time
//! and a block's `mcp.*` calls are sequential `await`s, the map is accessed serially — no intra-session
//! race — so plain [`RefCell`] interior mutability suffices. The functions are installed once (they
//! depend on session state, not the per-block transaction), so they persist across blocks like the
//! agent scratchpad.
//!
//! Following the block-API discipline ([`crate::agent::lua`]): the async functions and their futures
//! are `'static` (`create_async_function` requires it), capturing [`Rc`] clones of the session state,
//! and **no `RefCell` borrow is ever held across an `.await`**.

use std::{
    cell::RefCell,
    collections::{BTreeMap, HashMap},
    rc::Rc,
};

use mlua::{Function, Lua, LuaSerdeExt, Table, Value};

use crate::mcp::{
    ContentBlock, McpError, McpHost, McpInstance, McpOutput, McpServerConfig, McpTool,
};

/// The VM's MCP state: the host that spawns servers, the configured servers, and the lazily-spawned
/// per-session instances. Held behind an [`Rc`] so the projected Lua functions can share it.
pub(crate) struct McpSession {
    host: Rc<dyn McpHost>,
    servers: BTreeMap<String, McpServerConfig>,
    instances: RefCell<HashMap<String, Rc<SpawnedServer>>>,
}

/// One spawned server: its live instance and the escaped→raw tool-name map (built from `tools()` at
/// spawn, so the Lua-callable `goto_` resolves back to the raw `goto` the server expects).
struct SpawnedServer {
    instance: Box<dyn McpInstance>,
    escaped_to_raw: HashMap<String, String>,
}

impl McpSession {
    pub(crate) fn new(
        host: Rc<dyn McpHost>,
        servers: BTreeMap<String, McpServerConfig>,
    ) -> McpSession {
        McpSession {
            host,
            servers,
            instances: RefCell::new(HashMap::new()),
        }
    }

    /// Shut every spawned instance down (best-effort), draining the map first so no borrow is held
    /// across the awaits. Called at session end.
    pub(crate) async fn shutdown(&self) {
        let instances: Vec<Rc<SpawnedServer>> = self
            .instances
            .borrow_mut()
            .drain()
            .map(|(_, spawned)| spawned)
            .collect();
        for spawned in instances {
            spawned.instance.shutdown().await;
        }
    }

    /// Run one `mcp.<server>.<escaped_tool>(args)` call: spawn the server if needed, resolve the
    /// escaped name to the raw tool, marshal the argument table, call, and project the result. Every
    /// failure is a catchable Lua error.
    async fn call(
        &self,
        lua: &Lua,
        server: &str,
        escaped_tool: &str,
        args: Table,
    ) -> mlua::Result<Value> {
        let spawned = self.ensure_spawned(server).await.map_err(mcp_to_lua)?;
        let raw = spawned.escaped_to_raw.get(escaped_tool).ok_or_else(|| {
            mlua::Error::RuntimeError(format!(
                "mcp: server {server:?} has no tool {escaped_tool:?}"
            ))
        })?;
        let arguments = lua_args_to_json(lua, args)?;
        let output = spawned
            .instance
            .call(raw, arguments)
            .await
            .map_err(mcp_to_lua)?;
        project_output(lua, output)
    }

    /// The cached instance for `server`, spawning it on first use. The borrow that checks the map is
    /// dropped before the spawn `await` (the lock is not reentrant and a borrow may not cross an await).
    async fn ensure_spawned(&self, server: &str) -> Result<Rc<SpawnedServer>, McpError> {
        let existing = self.instances.borrow().get(server).cloned();
        if let Some(spawned) = existing {
            return Ok(spawned);
        }
        let config = self.servers.get(server).cloned().unwrap_or_default();
        let instance = self.host.spawn(server, &config).await?;
        let escaped_to_raw = build_escape_map(instance.tools())?;
        let spawned = Rc::new(SpawnedServer {
            instance,
            escaped_to_raw,
        });
        self.instances
            .borrow_mut()
            .insert(server.to_owned(), spawned.clone());
        Ok(spawned)
    }
}

/// Install the `mcp` global: a table with one proxy per configured server. An unconfigured `mcp.<x>`
/// is `nil`, so calling it is a plain "attempt to call a nil value".
pub(crate) fn install(lua: &Lua, mcp: &Rc<McpSession>) -> mlua::Result<()> {
    let mcp_table = lua.create_table()?;
    for server in mcp.servers.keys() {
        mcp_table.set(server.as_str(), server_proxy(lua, mcp, server)?)?;
    }
    lua.globals().set("mcp", mcp_table)?;
    Ok(())
}

/// A proxy table for one server: indexing it by a tool name (already Lua-escaped — see
/// [`escape_tool_name`]) mints that tool's async function via the metatable's `__index`. The catalogue
/// is unknown until the server is spawned on first call, so the projection is metatable-driven rather
/// than a pre-built table of functions.
fn server_proxy(lua: &Lua, mcp: &Rc<McpSession>, server: &str) -> mlua::Result<Table> {
    let proxy = lua.create_table()?;
    let metatable = lua.create_table()?;
    let mcp = mcp.clone();
    let server = server.to_owned();
    metatable.set(
        "__index",
        lua.create_function(move |lua, (_, tool): (Table, String)| {
            tool_function(lua, &mcp, &server, tool)
        })?,
    )?;
    proxy.set_metatable(Some(metatable))?;
    Ok(proxy)
}

/// The async function for one `(server, escaped tool)`: on call it marshals the argument table, runs
/// the tool, and projects the result (see [`McpSession::call`]).
fn tool_function(
    lua: &Lua,
    mcp: &Rc<McpSession>,
    server: &str,
    tool: String,
) -> mlua::Result<Function> {
    let mcp = mcp.clone();
    let server = server.to_owned();
    lua.create_async_function(move |lua, args: Table| {
        let mcp = mcp.clone();
        let server = server.clone();
        let tool = tool.clone();
        async move { mcp.call(&lua, &server, &tool, args).await }
    })
}

/// Marshal a Lua argument table to JSON-RPC `arguments` (spec §Calling): a consecutive-integer-key
/// table becomes a JSON array, otherwise an object; Lua integers serialize as JSON integers. The empty
/// table is the no-argument case and must be an object (tool arguments are always a top-level object),
/// but the serde bridge renders an empty table as `[]`, so it is mapped back to `{}`.
fn lua_args_to_json(lua: &Lua, args: Table) -> mlua::Result<serde_json::Value> {
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
fn project_output(lua: &Lua, output: McpOutput) -> mlua::Result<Value> {
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
/// are a hard error — the operator must rename or `deny` one (config-load validation lands in a later
/// increment; here it fails the spawn).
fn build_escape_map(tools: &[McpTool]) -> Result<HashMap<String, String>, McpError> {
    let mut map = HashMap::new();
    for tool in tools {
        let escaped = escape_tool_name(&tool.name);
        if let Some(existing) = map.insert(escaped.clone(), tool.name.clone()) {
            return Err(McpError::Spawn(format!(
                "two tools escape to the same Lua name {escaped:?}: {existing:?} and {:?}",
                tool.name
            )));
        }
    }
    Ok(map)
}

/// Escape a raw MCP tool name into a callable Lua identifier (spec §Tool names escaped into valid
/// Lua): characters illegal in a Lua identifier — including a leading digit — map to `_`, then a Lua
/// keyword takes a trailing `_` (`goto` → `goto_`).
fn escape_tool_name(raw: &str) -> String {
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

/// Whether `word` is a Lua 5.4 reserved keyword.
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

/// Render an [`McpError`] as the catchable Lua error the agent sees (its `Display` already leads with
/// an `mcp:` context prefix).
fn mcp_to_lua(error: McpError) -> mlua::Error {
    mlua::Error::RuntimeError(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::{escape_tool_name, lua_args_to_json};
    use mlua::Lua;

    /// Build a Lua table from a snippet returning one, for marshalling tests.
    fn table_from(lua: &Lua, expr: &str) -> mlua::Table {
        lua.load(expr).eval().unwrap()
    }

    #[test]
    fn integer_valued_numbers_marshal_as_json_integers() {
        let lua = Lua::new();
        let args = table_from(&lua, "return { timeout = 10000 }");
        let json = lua_args_to_json(&lua, args).unwrap();
        // A Lua integer is a JSON integer, not 10000.0.
        assert_eq!(json, serde_json::json!({ "timeout": 10000 }));
        assert!(json["timeout"].is_i64());
    }

    #[test]
    fn an_empty_table_marshals_to_an_object() {
        let lua = Lua::new();
        let args = table_from(&lua, "return {}");
        let json = lua_args_to_json(&lua, args).unwrap();
        assert_eq!(json, serde_json::json!({}));
        assert!(json.is_object());
    }

    #[test]
    fn a_consecutive_integer_key_table_marshals_to_an_array() {
        let lua = Lua::new();
        let args = table_from(&lua, r#"return { "a", "b", "c" }"#);
        let json = lua_args_to_json(&lua, args).unwrap();
        assert_eq!(json, serde_json::json!(["a", "b", "c"]));
    }

    #[test]
    fn keywords_and_illegal_characters_are_escaped() {
        // A keyword takes a trailing underscore.
        assert_eq!(escape_tool_name("goto"), "goto_");
        assert_eq!(escape_tool_name("navigate"), "navigate");
        // Illegal identifier characters (and a leading digit) map to underscore.
        assert_eq!(escape_tool_name("find-element"), "find_element");
        assert_eq!(escape_tool_name("3d"), "_d");
    }
}
