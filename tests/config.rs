//! Environmental-config loading: defaults when the file is absent, parsing when present, and
//! relative storage paths resolved against the config file's own directory (spec §Initialization).

use std::path::PathBuf;

use zuihitsu::{MemoryId, config::EnvConfig};

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

    assert_eq!(config.storage.event_log, dir.join("zuihitsu.events.sqlite"));
    assert_eq!(config.storage.graph, dir.join("zuihitsu.graph.sqlite"));

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn parses_storage_and_resolves_relative_paths() {
    let dir = temp_dir();
    let path = dir.join("config.toml");
    std::fs::write(
        &path,
        "[storage]\nevent_log = \"db/events.sqlite\"\ngraph = \"db/graph.sqlite\"\n",
    )
    .unwrap();

    let config = EnvConfig::load(&path).unwrap();
    assert_eq!(config.storage.event_log, dir.join("db/events.sqlite"));
    assert_eq!(config.storage.graph, dir.join("db/graph.sqlite"));

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
    assert_eq!(config.storage.event_log, dir.join("zuihitsu.events.sqlite"));

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
