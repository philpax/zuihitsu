//! The bot: its shared state, the Discord event handler, and the helpers that turn Discord events
//! into platform API calls.
//!
//! [`BotState`] holds the connector's shared state and opens its backing tables; [`Handler`] is the
//! `EventHandler` that wires Discord events to the platform client (see [`handler`]). The supporting
//! concerns live in sibling modules: [`identity`] projects sender, mention, and guild identity;
//! [`mentions`] rewrites raw Discord mentions into memory tokens; and [`process`] drives a debounced
//! batch through the platform stream and posts the outcome.

mod handler;
mod identity;
mod mentions;
mod process;

use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use serenity::{
    all::{ChannelId, UserId},
    prelude::*,
};
use tokio::sync::{Mutex, OnceCell};

use crate::{
    bot_loop::BotLoopGuard,
    config::DiscordConfig,
    context_sync::ContextSync,
    error::{Error, Result},
    guild_sync::GuildSync,
    pacing::DebounceState,
    projection_sync::ProjectionSync,
    turn_map::TurnMap,
};
use zuihitsu_core::ids::MemoryId;
use zuihitsu_platform_connector_api::PlatformClient;

pub use handler::Handler;

/// The shared bot state, stored in serenity's `TypeMap` via `Arc`.
pub struct BotState {
    pub config: DiscordConfig,
    pub platform: PlatformClient,
    pub bot_id: Mutex<Option<UserId>>,
    /// The agent's own reserved `self` memory id, fetched from the server on first use and cached —
    /// the splice target for the bot's own mention. The agent is never projected as a person, but its
    /// mention resolves to this canonical memory token like any other. Genesis-stable, so a successful
    /// fetch holds for the life of the process; a failed fetch leaves the cell empty, so the next
    /// mention retries.
    pub self_memory: OnceCell<MemoryId>,
    pub turn_map: Mutex<TurnMap>,
    pub context_sync: ContextSync,
    pub guild_sync: GuildSync,
    pub projection_sync: ProjectionSync,
    /// Per-channel present sets: users who have spoken in a channel the bot operates in. Grown
    /// lazily — a user is added when they send a message the bot processes, not eagerly from the
    /// guild member list. Keyed by channel so presence is per-conversation, not global. A user who
    /// leaves the guild is removed from every channel they were in.
    pub present_members: Mutex<HashMap<ChannelId, HashSet<UserId>>>,
    pub debounce: DebounceState,
    /// Guards against a bot-to-bot reply loop by capping consecutive bot-initiated turns per channel.
    pub bot_loop: BotLoopGuard,
}

impl BotState {
    pub fn new(config: DiscordConfig) -> Result<Self> {
        let debounce_ms = config.pacing.debounce_ms;
        let max_consecutive_bot_turns = config.behavior.max_consecutive_bot_turns;
        let db_path = config.storage.db_path.clone();
        let platform = PlatformClient::new(
            config.server.url.clone(),
            config.server.platform_key.clone(),
        );
        // The turn map and the identity sync are two tables in the connector's one SQLite state DB,
        // each opening its own connection to `db_path`.
        let turn_map = TurnMap::open(&db_path).map_err(|e| {
            Error::config(format!(
                "could not open the state db at {}: {e}",
                db_path.display()
            ))
        })?;
        let projection_sync = ProjectionSync::open(&db_path).map_err(|e| {
            Error::config(format!(
                "could not open the state db at {}: {e}",
                db_path.display()
            ))
        })?;
        Ok(BotState {
            platform,
            config,
            bot_id: Mutex::new(None),
            self_memory: OnceCell::new(),
            turn_map: Mutex::new(turn_map),
            context_sync: ContextSync::new(),
            guild_sync: GuildSync::new(),
            projection_sync,
            present_members: Mutex::new(HashMap::new()),
            debounce: DebounceState::new(debounce_ms),
            bot_loop: BotLoopGuard::new(max_consecutive_bot_turns),
        })
    }
}

/// A `TypeMap` key for the bot state.
pub struct BotStateKey;

impl TypeMapKey for BotStateKey {
    type Value = Arc<BotState>;
}
