use std::collections::BTreeMap;

use mlua::Lua;

use crate::{
    agent::mcp_api::{
        ApiType, InputSchema, McpCatalogue, escape_tool_name, filter_tools, lua_args_to_json,
    },
    mcp::{FakeMcpHost, FakeServer, McpServerConfig, McpTool},
};

/// Build a Lua table from a snippet returning one, for marshalling tests.
fn table_from(lua: &Lua, expr: &str) -> mlua::Table {
    lua.load(expr).eval().unwrap()
}

/// A tool advertised under `name`, with the given JSON-Schema input.
fn tool(name: &str, input_schema: serde_json::Value) -> McpTool {
    McpTool {
        name: name.to_owned(),
        description: format!("the {name} tool"),
        input_schema,
    }
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

/// The raw names of a filtered set, for terse assertions.
fn names(tools: &[McpTool]) -> Vec<&str> {
    tools.iter().map(|tool| tool.name.as_str()).collect()
}

#[test]
fn the_filter_intersects_allow_then_subtracts_deny() {
    let tools = vec![
        tool("navigate", serde_json::json!({})),
        tool("markdown", serde_json::json!({})),
        tool("evaluate", serde_json::json!({})),
    ];
    // No lists: the whole catalogue.
    assert_eq!(names(&filter_tools(&tools, None, None).unwrap()).len(), 3);
    // allow intersects, preserving the advertised order.
    let allow = vec!["navigate".to_owned(), "markdown".to_owned()];
    assert_eq!(
        names(&filter_tools(&tools, Some(&allow), None).unwrap()),
        ["navigate", "markdown"]
    );
    // deny subtracts (after allow).
    let deny = vec!["markdown".to_owned()];
    assert_eq!(
        names(&filter_tools(&tools, Some(&allow), Some(&deny)).unwrap()),
        ["navigate"]
    );
}

#[test]
fn a_filter_entry_matching_no_tool_is_an_error() {
    let tools = vec![tool("navigate", serde_json::json!({}))];
    let allow = vec!["bogus".to_owned()];
    let error = filter_tools(&tools, Some(&allow), None).unwrap_err();
    assert!(error.contains("bogus"), "error was {error:?}");
}

#[test]
fn schema_properties_become_typed_params() {
    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "url": { "type": "string", "description": "the page url" },
            "timeout": { "type": "integer" },
            "mode": { "enum": ["fast", "slow"] },
            "label": { "type": ["string", "null"] },
        },
        "required": ["url"],
    });
    let schema: InputSchema = serde_json::from_value(schema).unwrap();
    let params = schema.params();
    // Properties come out sorted by name (the `BTreeMap` ordering).
    assert_eq!(params.len(), 4);
    let url = params.iter().find(|param| param.name == "url").unwrap();
    assert_eq!(url.ty, ApiType::String);
    assert!(url.required);
    assert_eq!(url.doc, "the page url");
    let timeout = params.iter().find(|param| param.name == "timeout").unwrap();
    assert_eq!(timeout.ty, ApiType::Integer);
    assert!(!timeout.required);
    let mode = params.iter().find(|param| param.name == "mode").unwrap();
    assert_eq!(
        mode.ty,
        ApiType::Enum(vec!["fast".to_owned(), "slow".to_owned()])
    );
    // A `["string", "null"]` union resolves to its non-null primary type.
    let label = params.iter().find(|param| param.name == "label").unwrap();
    assert_eq!(label.ty, ApiType::String);
}

#[tokio::test]
async fn probe_applies_the_filter_and_renders_entries() {
    let host = FakeMcpHost::new().with(
        "browser",
        FakeServer::new(vec![
            tool("navigate", serde_json::json!({})),
            tool("evaluate", serde_json::json!({})),
        ]),
    );
    let configs = BTreeMap::from([(
        "browser".to_owned(),
        McpServerConfig {
            deny: Some(vec!["evaluate".to_owned()]),
            ..McpServerConfig::default()
        },
    )]);
    let catalogue = McpCatalogue::probe(&host, &configs).await.unwrap();
    // `evaluate` was denied, so only `navigate` is projected.
    let calls: Vec<String> = catalogue
        .api_entries()
        .into_iter()
        .map(|entry| entry.call)
        .collect();
    assert_eq!(calls, ["mcp.browser.navigate"]);
}

#[test]
fn an_mcp_tool_renders_with_a_braced_table_signature() {
    // The projection takes one table of named arguments, so the rendered signature must brace it
    // (`mcp.server.tool{ … }`) — a positional `(…)` signature is what led the agent to call it the
    // wrong way (passing positional args, which cannot convert to the expected table).
    let entry = super::tool_to_api_entry(
        "browser",
        &tool(
            "markdown",
            serde_json::json!({
                "type": "object",
                "properties": { "url": { "type": "string" } },
                "required": ["url"],
            }),
        ),
    );
    assert!(entry.table_args);
    // The console reads this to mark the tool as needing the `allow_mcp` opt-in.
    assert_eq!(entry.gate, Some(crate::agent::api_doc::ApiGate::Mcp));
    let rendered = crate::agent::api_doc::render(&[entry]);
    assert!(
        rendered.contains("mcp.browser.markdown{ url }"),
        "expected a braced table signature, got:\n{rendered}"
    );
}

#[tokio::test]
async fn probe_drops_a_server_that_fails_to_spawn() {
    let host = FakeMcpHost::new()
        .with(
            "ok",
            FakeServer::new(vec![tool("markdown", serde_json::json!({}))]),
        )
        .with("broken", FakeServer::spawn_failure("no binary"));
    let configs = BTreeMap::from([
        ("ok".to_owned(), McpServerConfig::default()),
        ("broken".to_owned(), McpServerConfig::default()),
    ]);
    let catalogue = McpCatalogue::probe(&host, &configs).await.unwrap();
    // The broken server is dropped; the working one's tool is still projected.
    assert_eq!(
        catalogue
            .api_entries()
            .into_iter()
            .map(|entry| entry.call)
            .collect::<Vec<_>>(),
        ["mcp.ok.markdown"]
    );
}

#[tokio::test]
async fn probe_rejects_an_allow_entry_matching_no_tool() {
    let host = FakeMcpHost::new().with(
        "browser",
        FakeServer::new(vec![tool("navigate", serde_json::json!({}))]),
    );
    let configs = BTreeMap::from([(
        "browser".to_owned(),
        McpServerConfig {
            allow: Some(vec!["renamed".to_owned()]),
            ..McpServerConfig::default()
        },
    )]);
    let error = McpCatalogue::probe(&host, &configs).await.unwrap_err();
    assert!(error.to_string().contains("renamed"), "error was {error}");
}
