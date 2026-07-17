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
    /// The bearer key this connector authenticates with. The server resolves it to the connector's
    /// registration, which is the single source of truth for the connector's platform and its event
    /// attribution — the connector names neither itself.
    pub platform_key: String,
}

/// The Discord bot token.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct DiscordSection {
    pub token: String,
}

/// Channel authorisation and addressing rules.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct BehaviorConfig {
    /// Channel IDs the bot is authorised to operate in. Messages in guild channels not in this list
    /// are ignored. DMs are always open.
    #[serde(default)]
    pub allowed_channels: HashSet<ChannelId>,
    /// Which messages in an allowed guild channel the bot forwards to the agent. DMs are always
    /// forwarded regardless.
    #[serde(default)]
    pub reply_to: ReplyMode,
    /// Whether messages from *other* bots are seen. On (the default), another bot is treated like any
    /// participant — subject to the same reply mode, and repliable. Off drops every other bot's
    /// message. The connector never processes its own messages regardless, matching them by id.
    #[serde(default = "default_see_other_bots")]
    pub see_other_bots: bool,
    /// The cap on consecutive turns another bot may initiate in a channel before its messages are
    /// dropped until a human speaks. This bounds a bot-to-bot reply loop, where two agents answer
    /// each other endlessly. Only meaningful when [`see_other_bots`](Self::see_other_bots) is on.
    #[serde(default = "default_max_consecutive_bot_turns")]
    pub max_consecutive_bot_turns: u32,
}

impl Default for BehaviorConfig {
    fn default() -> Self {
        BehaviorConfig {
            allowed_channels: HashSet::new(),
            reply_to: ReplyMode::default(),
            see_other_bots: default_see_other_bots(),
            max_consecutive_bot_turns: default_max_consecutive_bot_turns(),
        }
    }
}

/// Which messages in an allowed guild channel the connector forwards to the agent.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, Default, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ReplyMode {
    /// Every message — the agent's own stay-silent terminal then decides whether a given one warrants
    /// a reply. The default: the bot is a full participant in the channel.
    #[default]
    All,
    /// Only messages that address the bot — a mention, or a reply to one of its messages.
    Addressed,
}

/// Persistent storage paths.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct StorageConfig {
    /// Path to the connector's SQLite state database. It holds the turn map (Discord message id →
    /// zuihitsu turn id) and the identity sync (the last-projected username, display name, and
    /// nickname per user, and the entry id to supersede on the next change), each in its own table.
    /// All of it survives a connector restart.
    pub db_path: PathBuf,
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

fn default_see_other_bots() -> bool {
    true
}

fn default_max_consecutive_bot_turns() -> u32 {
    10
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
db_path = "discord.db"
"#,
        )
        .unwrap();
        assert_eq!(config.pacing.debounce_ms, 500);
        assert_eq!(config.pacing.typing_refresh_secs, 8);
        assert!(config.behavior.allowed_channels.is_empty());
        assert!(config.behavior.see_other_bots);
        assert_eq!(config.behavior.max_consecutive_bot_turns, 10);
        assert_eq!(config.storage.db_path, PathBuf::from("discord.db"));
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
db_path = "discord.db"
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
db_path = "discord.db"
"#,
        );
        let config = result.unwrap();
        assert!(config.validate().is_err());
    }
}
