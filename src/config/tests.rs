//! Environmental-config loading: defaults when the file is absent, parsing when present, and
//! relative storage paths resolved against the config file's own directory (spec §Initialization).
use std::path::PathBuf;

use super::EnvConfig;
use crate::ids::MemoryId;

fn temp_dir() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("zuihitsu-cfg-{}", MemoryId::generate().0));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn missing_file_yields_defaults_resolved_against_its_directory() {
    let dir = temp_dir();
    let path = dir.join("config.toml"); // does not exist
    let config = EnvConfig::load(&path).unwrap();

    assert_eq!(config.storage.dir, dir.join("data"));
    assert_eq!(config.storage.event_log(), dir.join("data/events.sqlite"));
    assert_eq!(config.storage.graph(), dir.join("data/graph.sqlite"));
    assert_eq!(config.storage.vectors(), dir.join("data/vectors.sqlite"));

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn parses_storage_and_resolves_relative_paths() {
    let dir = temp_dir();
    let path = dir.join("config.toml");
    std::fs::write(&path, "[storage]\ndir = \"db\"\n").unwrap();

    let config = EnvConfig::load(&path).unwrap();
    assert_eq!(config.storage.dir, dir.join("db"));
    assert_eq!(config.storage.event_log(), dir.join("db/events.sqlite"));
    assert_eq!(config.storage.graph(), dir.join("db/graph.sqlite"));
    assert_eq!(config.storage.vectors(), dir.join("db/vectors.sqlite"));

    std::fs::remove_dir_all(&dir).ok();
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
    let dir = temp_dir();
    let path = dir.join("config.toml");
    std::fs::write(
        &path,
        "[snapshots]\nenabled = false\ndir = \"snaps\"\nkeep = 3\nmin_new_events = 100\n",
    )
    .unwrap();

    let config = EnvConfig::load(&path).unwrap();
    assert!(!config.snapshots.enabled);
    assert_eq!(config.snapshots.keep, 3);
    assert_eq!(config.snapshots.min_new_events, 100);
    // An explicit dir is honored and resolved against the config's directory.
    assert_eq!(config.snapshots.dir, Some(dir.join("snaps")));
    assert_eq!(
        config.snapshots.effective_dir(&config.storage.graph()),
        dir.join("snaps")
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn serving_bind_defaults_to_loopback_and_parses_an_override() {
    // Absent, the server binds a loopback port; a `[serving]` block overrides it.
    assert_eq!(
        EnvConfig::default().serving.bind,
        "127.0.0.1:7777".parse().unwrap()
    );
    let dir = temp_dir();
    let path = dir.join("config.toml");
    std::fs::write(&path, "[serving]\nbind = \"127.0.0.1:9090\"\n").unwrap();
    let config = EnvConfig::load(&path).unwrap();
    assert_eq!(config.serving.bind, "127.0.0.1:9090".parse().unwrap());
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn serving_api_keys_default_empty_and_parse_as_arrays() {
    // No keys by default — a loopback-only, no-remote-access posture.
    assert!(EnvConfig::default().serving.control_keys.is_empty());
    assert!(EnvConfig::default().serving.platform_keys.is_empty());

    let dir = temp_dir();
    let path = dir.join("config.toml");
    std::fs::write(
        &path,
        "[serving]\n\
         bind = \"0.0.0.0:7777\"\n\
         control_keys = [\"op-key\"]\n\
         platform_keys = [\"discord-key\", \"web-key\"]\n",
    )
    .unwrap();
    let config = EnvConfig::load(&path).unwrap();
    assert_eq!(config.serving.control_keys, vec!["op-key"]);
    assert_eq!(config.serving.platform_keys, vec!["discord-key", "web-key"]);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn unknown_sections_are_ignored() {
    let dir = temp_dir();
    let path = dir.join("config.toml");
    // A [model] section (consumed by a later stage) must not break loading.
    std::fs::write(
        &path,
        "[model]\nendpoint = \"http://example/v1\"\nllm = \"some-model\"\n",
    )
    .unwrap();

    let config = EnvConfig::load(&path).unwrap();
    assert_eq!(config.storage.event_log(), dir.join("data/events.sqlite"));

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn malformed_toml_is_an_error() {
    let dir = temp_dir();
    let path = dir.join("config.toml");
    std::fs::write(&path, "this is not = = valid toml").unwrap();
    assert!(EnvConfig::load(&path).is_err());
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn parses_mcp_server_blocks() {
    let dir = temp_dir();
    let path = dir.join("config.toml");
    std::fs::write(
        &path,
        "[mcp.browser]\n\
         command = \"mcp/browser\"\n\
         args = [\"mcp\"]\n\
         deny = [\"evaluate\"]\n",
    )
    .unwrap();

    let config = EnvConfig::load(&path).unwrap();
    let server = config.mcp.get("browser").expect("the browser block");
    assert_eq!(server.command, "mcp/browser");
    assert_eq!(server.args, ["mcp"]);
    assert_eq!(
        server.deny.as_deref(),
        Some(["evaluate".to_owned()].as_slice())
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn an_mcp_server_name_that_is_not_a_lua_identifier_is_rejected() {
    let dir = temp_dir();
    let path = dir.join("config.toml");
    // `light-panda` is not a valid Lua identifier, so it cannot be a `mcp.<name>` prefix.
    std::fs::write(&path, "[mcp.\"light-panda\"]\ncommand = \"x\"\n").unwrap();

    match EnvConfig::load(&path).unwrap_err() {
        super::ConfigError::InvalidMcpServerName(name) => assert_eq!(name, "light-panda"),
        other => panic!("unexpected error: {other}"),
    }

    std::fs::remove_dir_all(&dir).ok();
}
