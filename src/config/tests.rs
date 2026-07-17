//! Environmental-config loading: defaults when the file is absent, parsing when present, and
//! relative storage paths resolved against the config file's own directory (spec §Initialization).
//!
//! The parse-and-resolve tests drive [`EnvConfig::load_from_string`] against a fixed [`base`]
//! directory — path resolution is pure `PathBuf` joins, so no file is ever written. Only the
//! missing-file test exercises [`EnvConfig::load`]'s I/O path, and it reads a path that does not exist
//! rather than creating one.

use std::path::{Path, PathBuf};

use super::EnvConfig;
use crate::ids::MemoryId;

/// A fixed stand-in for the config file's directory, against which relative storage paths resolve.
/// It need not exist: resolution only joins onto it.
fn base() -> &'static Path {
    Path::new("/base")
}

#[test]
fn missing_file_yields_defaults_resolved_against_its_directory() {
    // No file is written: `load` reads a path that does not exist, falls back to the all-defaults
    // config, and resolves the storage dir against the file's intended directory. The unique suffix
    // guarantees the path is absent without touching the filesystem.
    let dir = std::env::temp_dir().join(format!("zuihitsu-absent-{}", MemoryId::generate().0));
    let path = dir.join("config.toml");
    let config = EnvConfig::load(&path).unwrap();

    assert_eq!(config.storage.dir, dir.join("data"));
    assert_eq!(config.storage.event_log(), dir.join("data/events.sqlite"));
    assert_eq!(config.storage.graph(), dir.join("data/graph.sqlite"));
    assert_eq!(config.storage.vectors(), dir.join("data/vectors.sqlite"));
}

#[test]
fn parses_storage_and_resolves_relative_paths() {
    let config = EnvConfig::load_from_string("[storage]\ndir = \"db\"\n", base()).unwrap();
    assert_eq!(config.storage.dir, base().join("db"));
    assert_eq!(config.storage.event_log(), base().join("db/events.sqlite"));
    assert_eq!(config.storage.graph(), base().join("db/graph.sqlite"));
    assert_eq!(config.storage.vectors(), base().join("db/vectors.sqlite"));
}

#[test]
fn snapshots_default_on_with_a_dir_beside_the_graph() {
    // On by default (better safe than sorry), writing to `snapshots/` beside the graph.
    let config = EnvConfig::default();
    assert!(config.snapshots.enabled);
    assert_eq!(
        config
            .snapshots
            .effective_dir(std::path::Path::new("data/graph.sqlite")),
        PathBuf::from("data/snapshots")
    );
}

#[test]
fn snapshots_parse_an_override_and_resolve_the_dir() {
    let config = EnvConfig::load_from_string(
        "[snapshots]\nenabled = false\ndir = \"snaps\"\nkeep = 3\nmin_new_events = 100\n",
        base(),
    )
    .unwrap();
    assert!(!config.snapshots.enabled);
    assert_eq!(config.snapshots.keep, 3);
    assert_eq!(config.snapshots.min_new_events, 100);
    // An explicit dir is honored and resolved against the config's directory.
    assert_eq!(config.snapshots.dir, Some(base().join("snaps")));
    assert_eq!(
        config.snapshots.effective_dir(&config.storage.graph()),
        base().join("snaps")
    );
}

#[test]
fn serving_bind_defaults_to_loopback_and_parses_an_override() {
    // Absent, the server binds a loopback port; a `[serving]` block overrides it.
    assert_eq!(
        EnvConfig::default().serving.bind,
        "127.0.0.1:7777".parse().unwrap()
    );
    let config =
        EnvConfig::load_from_string("[serving]\nbind = \"127.0.0.1:9090\"\n", base()).unwrap();
    assert_eq!(config.serving.bind, "127.0.0.1:9090".parse().unwrap());
}

#[test]
fn control_keys_default_empty_and_parse_as_arrays() {
    // No keys by default — a loopback-only, no-remote-access posture.
    assert!(EnvConfig::default().serving.control_keys.is_empty());

    let config = EnvConfig::load_from_string(
        "[serving]\n\
         bind = \"0.0.0.0:7777\"\n\
         control_keys = [\"op-key\"]\n",
        base(),
    )
    .unwrap();
    assert_eq!(config.serving.control_keys, vec!["op-key"]);
}

#[test]
fn connectors_parse_as_a_platform_keyed_map() {
    // No connectors by default — nothing may reach the platform surface remotely.
    assert!(EnvConfig::default().connectors.is_empty());

    let config = EnvConfig::load_from_string(
        "[connectors]\n\
         discord = { key = \"discord-key\" }\n\
         slack = { key = \"slack-key\" }\n",
        base(),
    )
    .unwrap();
    assert_eq!(config.connectors.len(), 2);
    assert_eq!(config.connectors["discord"].key, "discord-key");
    assert_eq!(config.connectors["slack"].key, "slack-key");
}

#[test]
fn unknown_sections_are_ignored() {
    // A [model] section (consumed by a later stage) must not break loading.
    let config = EnvConfig::load_from_string(
        "[model]\nendpoint = \"http://example/v1\"\nllm = \"some-model\"\n",
        base(),
    )
    .unwrap();
    assert_eq!(
        config.storage.event_log(),
        base().join("data/events.sqlite")
    );
}

#[test]
fn malformed_toml_is_an_error() {
    assert!(EnvConfig::load_from_string("this is not = = valid toml", base()).is_err());
}

#[test]
fn parses_mcp_server_blocks() {
    let config = EnvConfig::load_from_string(
        "[mcp.browser]\n\
         command = \"mcp/browser\"\n\
         args = [\"mcp\"]\n\
         deny = [\"evaluate\"]\n",
        base(),
    )
    .unwrap();
    let server = config.mcp.get("browser").expect("the browser block");
    assert_eq!(server.command, "mcp/browser");
    assert_eq!(server.args, ["mcp"]);
    assert_eq!(
        server.deny.as_deref(),
        Some(["evaluate".to_owned()].as_slice())
    );
}

#[test]
fn an_mcp_server_name_that_is_not_a_lua_identifier_is_rejected() {
    // `light-panda` is not a valid Lua identifier, so it cannot be a `mcp.<name>` prefix.
    match EnvConfig::load_from_string("[mcp.\"light-panda\"]\ncommand = \"x\"\n", base())
        .unwrap_err()
    {
        super::ConfigError::InvalidMcpServerName(name) => assert_eq!(name, "light-panda"),
        other => panic!("unexpected error: {other}"),
    }
}
