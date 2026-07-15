//! Connector configuration: loaded from `config.discord.toml`.

use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use serenity::model::id::ChannelId;

use crate::error::{Error, Result};

/// The top-level connector config.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct DiscordConfig {
    pub server: ServerConfig,
    pub discord: DiscordSection,
    #[serde(default)]
    pub behavior: BehaviorConfig,
    #[serde(default)]
    pub pacing: PacingConfig,
    pub storage: StorageConfig,
}

/// The zuihitsu server connection.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ServerConfig {
    pub url: String,
    pub platform_key: String,
    /// The identifier this connector presents to the server, used to attribute context writes in
    /// the event log. Defaults to "discord" if unset.
    #[serde(default = "default_connector_id")]
    pub connector_id: String,
}

fn default_connector_id() -> String {
    "discord".to_owned()
}

/// The Discord bot token.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct DiscordSection {
    pub token: String,
}

/// Channel authorisation and addressing rules.
#[derive(Clone, Debug, Deserialize, Serialize, Default)]
pub struct BehaviorConfig {
    /// Channel IDs the bot is authorised to operate in. Messages in guild channels not in this list
    /// are ignored. DMs are always open.
    #[serde(default)]
    pub allowed_channels: HashSet<ChannelId>,
}

/// Persistent storage paths.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct StorageConfig {
    /// Path to the SQLite database for the turn map (Discord message ID → zuihitsu turn ID).
    /// The mapping survives connector restarts.
    pub turn_map_path: PathBuf,
}

/// Pacing tunables.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PacingConfig {
    /// How long to wait after a message before processing, to coalesce rapid-fire messages.
    #[serde(default = "default_debounce_ms")]
    pub debounce_ms: u64,
    /// How often to refresh the typing indicator while the agent is emitting reply tokens.
    #[serde(default = "default_typing_refresh_secs")]
    pub typing_refresh_secs: u64,
}

impl Default for PacingConfig {
    fn default() -> Self {
        PacingConfig {
            debounce_ms: default_debounce_ms(),
            typing_refresh_secs: default_typing_refresh_secs(),
        }
    }
}

fn default_debounce_ms() -> u64 {
    500
}

fn default_typing_refresh_secs() -> u64 {
    8
}

impl DiscordConfig {
    /// Load config from a TOML file at `path`.
    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| Error::io(format!("could not read config at {}", path.display()), e))?;
        let config: DiscordConfig = toml::from_str(&text)
            .map_err(|e| Error::config(format!("could not parse config: {e}")))?;
        config.validate()?;
        Ok(config)
    }

    /// Validate that required fields are present and non-empty.
    fn validate(&self) -> Result<()> {
        if self.server.url.trim().is_empty() {
            return Err(Error::config("server.url is required"));
        }
        if self.server.platform_key.trim().is_empty() {
            return Err(Error::config("server.platform_key is required"));
        }
        if self.discord.token.trim().is_empty() {
            return Err(Error::config("discord.token is required"));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_defaults() {
        let config: DiscordConfig = toml::from_str(
            r#"
[server]
url = "http://127.0.0.1:7777"
platform_key = "key"

[discord]
token = "tok"

[storage]
turn_map_path = "turn_map.db"
"#,
        )
        .unwrap();
        assert_eq!(config.pacing.debounce_ms, 500);
        assert_eq!(config.pacing.typing_refresh_secs, 8);
        assert!(config.behavior.allowed_channels.is_empty());
        assert_eq!(config.storage.turn_map_path, PathBuf::from("turn_map.db"));
    }

    #[test]
    fn config_missing_required() {
        let result: std::result::Result<DiscordConfig, _> = toml::from_str(
            r#"
[server]
url = ""
platform_key = "key"

[discord]
token = "tok"

[storage]
turn_map_path = "turn_map.db"
"#,
        );
        let config = result.unwrap();
        assert!(config.validate().is_err());
    }

    #[test]
    fn config_missing_token() {
        let result: std::result::Result<DiscordConfig, _> = toml::from_str(
            r#"
[server]
url = "http://127.0.0.1:7777"
platform_key = "key"

[discord]
token = ""

[storage]
turn_map_path = "turn_map.db"
"#,
        );
        let config = result.unwrap();
        assert!(config.validate().is_err());
    }
}
