//! The bot event handler: the `EventHandler` implementation that wires Discord events to the
//! platform client.
//!
//! The main flow is `message`: check addressing, debounce, construct the locator, gather the
//! present set, inject `[turn:<id>]` if replying to a mapped message, call the platform API stream,
//! watch for reply progress to start the typing indicator, and post the outcome back to Discord.

use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use parking_lot::Mutex as SyncMutex;
use serenity::{
    all::{ChannelId, GuildChannel, GuildId, Member, Message, Ready, ResumedEvent, User, UserId},
    async_trait,
    prelude::*,
};
use tokio::sync::Mutex;

use zuihitsu_connector_api::{
    LinkEndpoint, PlatformClient, PlatformMessage, StreamOutcome, TurnOutcome,
};
use zuihitsu_core::{
    ids::{ConversationLocator, PersonId, TurnId},
    progress::{ProgressKind, TurnProgress},
};

use crate::{
    addressing::{AddressingDecision, MessageContext, should_respond},
    config::DiscordConfig,
    context_sync::{ContextParams, ContextSync},
    error::{Error, Result},
    guild_sync::GuildSync,
    locator::{ChannelContext, DISCORD_PLATFORM, guild_locator},
    pacing::{DebounceState, PendingMessage},
    projection_sync::{ObservedAttribute, ProjectionSync},
    turn_map::TurnMap,
};

/// The shared bot state, stored in serenity's `TypeMap` via `Arc`.
pub struct BotState {
    pub config: DiscordConfig,
    pub platform: PlatformClient,
    pub bot_id: Mutex<Option<UserId>>,
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
}

impl BotState {
    pub fn new(config: DiscordConfig) -> Result<Self> {
        let debounce_ms = config.pacing.debounce_ms;
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
            turn_map: Mutex::new(turn_map),
            context_sync: ContextSync::new(),
            guild_sync: GuildSync::new(),
            projection_sync,
            present_members: Mutex::new(HashMap::new()),
            debounce: DebounceState::new(debounce_ms),
        })
    }
}

/// The event handler. Each method is the Discord-facing side of one platform concern.
pub struct Handler;

#[async_trait]
impl EventHandler for Handler {
    async fn ready(&self, ctx: Context, ready: Ready) {
        tracing::info!("discord connector: connected as {}", ready.user.name);

        // Store the bot's user id for mention/reply detection.
        {
            let data = ctx.data.read().await;
            if let Some(state) = data.get::<BotStateKey>() {
                *state.bot_id.lock().await = Some(ready.user.id);
            }
        }
    }

    async fn message(&self, ctx: Context, msg: Message) {
        // Never process our own messages.
        if msg.author.bot {
            return;
        }

        let state = {
            let data = ctx.data.read().await;
            let Some(state) = data.get::<BotStateKey>().cloned() else {
                return;
            };
            state
        };

        let Some(bot_id) = *state.bot_id.lock().await else {
            return;
        };

        // Track the sender in the channel's present set (lazy presence tracking).
        state
            .present_members
            .lock()
            .await
            .entry(msg.channel_id)
            .or_default()
            .insert(msg.author.id);

        // Determine addressing.
        let mentions_bot = msg.mentions.iter().any(|u| u.id == bot_id);
        let replies_to_bot = msg
            .referenced_message
            .as_ref()
            .is_some_and(|rm| rm.author.id == bot_id);

        let msg_ctx = MessageContext {
            author_is_bot: msg.author.bot,
            guild_id: msg.guild_id.map(|g| g.get()),
            channel_id: msg.channel_id.get(),
            mentions_bot,
            replies_to_bot,
        };

        let AddressingDecision { should_forward, .. } =
            should_respond(&msg_ctx, &state.config.behavior);
        if !should_forward {
            // Dropped by the addressing filter — the channel is not on the allow-list, or `reply_to =
            // addressed` and this message did not mention or reply to the bot. Debug so it stays out of
            // default logging but is there when a "nothing happened" needs explaining.
            tracing::debug!(
                channel_id = msg.channel_id.get(),
                "discord connector: message not forwarded (channel not allowed, or unaddressed)"
            );
            return;
        }

        // Build the channel context for the locator.
        let channel_ctx = match msg.guild_id {
            Some(guild_id) => ChannelContext::Guild {
                guild_id: guild_id.get(),
                channel_id: msg.channel_id.get(),
            },
            None => ChannelContext::DirectMessage {
                channel_id: msg.channel_id.get(),
            },
        };
        let locator = channel_ctx.locator();
        let is_dm = channel_ctx.is_dm();

        // Inject `[turn:<id>]` if replying to a mapped message.
        let text = {
            let turn_map = state.turn_map.lock().await;
            let referenced_id = msg.referenced_message.as_ref().map(|rm| rm.id);
            turn_map.inject_turn_ref(&msg.content, referenced_id.as_ref())
        };

        // Ensure context is written on first contact.
        let guild_name = match msg.guild_id {
            Some(guild_id) => guild_name(&ctx, guild_id).await,
            None => String::new(),
        };
        let (channel_name, topic) = channel_metadata(&ctx, msg.channel_id).await;

        if let Err(error) = state
            .context_sync
            .ensure_context(
                &state.platform,
                &locator,
                &ContextParams {
                    channel_id: msg.channel_id,
                    guild_name: &guild_name,
                    channel_name: &channel_name,
                    topic: &topic,
                    is_dm,
                },
            )
            .await
        {
            tracing::warn!(%error, "discord connector: could not write context on first contact");
        }

        // Project the sender's current username, display name, and nickname onto their profile,
        // superseding whichever changed since we last saw them. Only the sender is projected — never
        // the bot, whose own messages are filtered above — so the agent's own identity is never minted
        // as another entity.
        let sender_person = PersonId::new(DISCORD_PLATFORM, msg.author.id.to_string());
        if let Err(error) = state
            .projection_sync
            .sync(
                &state.platform,
                &LinkEndpoint::Participant(sender_person.clone()),
                sender_person.id.as_str(),
                &observed_identity(&msg, &guild_name),
            )
            .await
        {
            tracing::warn!(%error, "discord connector: could not project participant identity");
        }

        // In a guild, keep the guild's context and its structural links current: project the server
        // name onto the guild's context memory (superseding it on a rename), place the channel in the
        // guild, and place the sender in the guild as a durable member.
        if let Some(guild_id) = msg.guild_id {
            let guild_id = guild_id.get();
            if let Err(error) = sync_guild_name(&state, guild_id, &guild_name).await {
                tracing::warn!(%error, "discord connector: could not sync guild name");
            }
            if let Err(error) = state
                .guild_sync
                .link_channel(&state.platform, guild_id, &locator, msg.channel_id.get())
                .await
            {
                tracing::warn!(%error, "discord connector: could not link channel to guild");
            }
            if let Err(error) = state
                .guild_sync
                .link_member(
                    &state.platform,
                    guild_id,
                    &sender_person,
                    msg.author.id.get(),
                )
                .await
            {
                tracing::warn!(%error, "discord connector: could not link member to guild");
            }
        }

        // Gather the present set. A DM is just the sender: the bot is the agent itself, not another
        // participant, so it is never added to presence (which would mint a phantom person stub).
        let present: Vec<PersonId> = if is_dm {
            vec![PersonId::new(DISCORD_PLATFORM, msg.author.id.to_string())]
        } else {
            let present = state.present_members.lock().await;
            present
                .get(&msg.channel_id)
                .map(|set| {
                    set.iter()
                        .map(|id| PersonId::new(DISCORD_PLATFORM, id.to_string()))
                        .collect()
                })
                .unwrap_or_default()
        };

        let sender = msg.author.id.to_string();

        // Submit to the debounce. The actor collects all messages within the debounce window,
        // then fires once with the batch — one turn for the whole burst. The latest message's
        // context (present set, locator) is used, since it's the freshest.
        let pending = PendingMessage { text, sender };
        let ctx_clone = ctx.clone();
        let state_clone = state.clone();
        let present_clone = present.clone();
        let channel_id = msg.channel_id;
        let locator_clone = locator.clone();
        state
            .debounce
            .submit(msg.channel_id, pending, move |batch| {
                let ctx = ctx_clone.clone();
                let state = state_clone.clone();
                let present = present_clone.clone();
                let locator = locator_clone.clone();
                tokio::spawn(async move {
                    process_message(&ctx, &state, &locator, batch, &present, channel_id).await;
                });
            });
    }

    async fn guild_member_removal(
        &self,
        ctx: Context,
        guild_id: GuildId,
        user: User,
        _member_data: Option<Member>,
    ) {
        let data = ctx.data.read().await;
        let Some(state) = data.get::<BotStateKey>() else {
            return;
        };
        // Remove the departing user from every channel's present set.
        {
            let mut present = state.present_members.lock().await;
            for set in present.values_mut() {
                set.remove(&user.id);
            }
        }
        // Retract the member's `part_of` guild link — guild membership is durable, so a departure must
        // undo it rather than leave the person a standing member.
        let person = PersonId::new(DISCORD_PLATFORM, user.id.to_string());
        if let Err(error) = state
            .guild_sync
            .unlink_member(&state.platform, guild_id.get(), &person, user.id.get())
            .await
        {
            tracing::warn!(%error, "discord connector: could not unlink departed member from guild");
        }
    }

    async fn channel_update(&self, ctx: Context, _old: Option<GuildChannel>, new: GuildChannel) {
        let data = ctx.data.read().await;
        let Some(state) = data.get::<BotStateKey>() else {
            return;
        };

        let channel_id = new.id;
        let channel_name = new.name.clone();
        let topic = new.topic.clone().unwrap_or_default();
        let guild_id = new.guild_id;

        let guild_name = ctx
            .cache
            .guild(guild_id)
            .map(|g| g.name.clone())
            .unwrap_or_default();

        let channel_ctx = ChannelContext::Guild {
            guild_id: guild_id.get(),
            channel_id: channel_id.get(),
        };
        let locator = channel_ctx.locator();

        if let Err(error) = state
            .context_sync
            .update_context(
                &state.platform,
                &locator,
                channel_id,
                &guild_name,
                &channel_name,
                &topic,
            )
            .await
        {
            tracing::warn!(%error, "discord connector: could not update context on channel update");
        }
    }

    async fn guild_update(
        &self,
        ctx: Context,
        _old: Option<serenity::all::Guild>,
        new: serenity::all::PartialGuild,
    ) {
        let data = ctx.data.read().await;
        let Some(state) = data.get::<BotStateKey>() else {
            return;
        };
        // Supersede the guild's context name if the server was renamed.
        if let Err(error) = sync_guild_name(state, new.id.get(), &new.name).await {
            tracing::warn!(%error, "discord connector: could not sync guild name on update");
        }
    }

    async fn resume(&self, _ctx: Context, _: ResumedEvent) {
        tracing::info!("discord connector: gateway resumed");
    }
}

/// The identity attributes to project for a message's sender: the account username and display name
/// (global to Discord), and the server nickname (per guild, keyed by guild id). An unset display name
/// or nickname is carried as `None`, so clearing it later retracts the prior projection rather than
/// leaving a stale handle. The nickname is only observed in a guild — a DM has none.
fn observed_identity(msg: &Message, guild_name: &str) -> Vec<ObservedAttribute> {
    let mut observed = vec![
        ObservedAttribute {
            key: "username".to_owned(),
            value: Some(msg.author.name.clone()),
            entry_text: format!("Discord username: {}", msg.author.name),
        },
        ObservedAttribute {
            key: "display_name".to_owned(),
            entry_text: msg
                .author
                .global_name
                .as_deref()
                .map(|name| format!("Discord display name: {name}"))
                .unwrap_or_default(),
            value: msg.author.global_name.clone(),
        },
    ];
    if let Some(guild_id) = msg.guild_id {
        let nick = msg.member.as_ref().and_then(|member| member.nick.clone());
        observed.push(ObservedAttribute {
            key: format!("nickname:{}", guild_id.get()),
            entry_text: nick
                .as_deref()
                .map(|nick| format!("Discord nickname in {guild_name}: {nick}"))
                .unwrap_or_default(),
            value: nick,
        });
    }
    observed
}

/// Project a guild's server name onto its context memory, superseding it on a rename. A blank name (the
/// cache cold and the fetch failed) is skipped rather than recorded as an empty attribute.
async fn sync_guild_name(state: &BotState, guild_id: u64, guild_name: &str) -> Result<()> {
    if guild_name.is_empty() {
        return Ok(());
    }
    state
        .projection_sync
        .sync(
            &state.platform,
            &LinkEndpoint::Context(guild_locator(guild_id)),
            &format!("guild/{guild_id}"),
            &[ObservedAttribute {
                key: "server_name".to_owned(),
                value: Some(guild_name.to_owned()),
                entry_text: format!("Discord server: {guild_name}"),
            }],
        )
        .await
}

/// The guild's name — from the cache when it is warm, else fetched over HTTP, so the very first
/// message (before `GUILD_CREATE` populates the cache) still resolves the real name. Empty only if
/// both the cache miss and the fetch fails.
async fn guild_name(ctx: &Context, guild_id: GuildId) -> String {
    if let Some(name) = ctx.cache.guild(guild_id).map(|guild| guild.name.clone()) {
        return name;
    }
    guild_id
        .to_partial_guild(&ctx.http)
        .await
        .map(|guild| guild.name)
        .unwrap_or_default()
}

/// Extract `(name, topic)` from a channel, returning empty strings if unavailable.
async fn channel_metadata(ctx: &Context, channel_id: serenity::all::ChannelId) -> (String, String) {
    match channel_id.to_channel(&ctx.http).await {
        Ok(serenity::all::Channel::Guild(gc)) => {
            (gc.name.clone(), gc.topic.clone().unwrap_or_default())
        }
        _ => (String::new(), String::new()),
    }
}

/// Process a debounced batch: send it to the platform, watch progress, post the outcome.
async fn process_message(
    ctx: &Context,
    state: &Arc<BotState>,
    locator: &ConversationLocator,
    batch: Vec<PendingMessage>,
    present: &[PersonId],
    channel_id: serenity::all::ChannelId,
) {
    let messages: Vec<PlatformMessage> = batch
        .into_iter()
        .map(|m| PlatformMessage {
            sender: PersonId::new(DISCORD_PLATFORM, &m.sender),
            text: m.text,
        })
        .collect();

    // The typing indicator starts on the first Reply progress fragment and is refreshed until
    // the outcome arrives. The callback fires as each fragment streams in, so typing starts
    // during reply emission, not after the whole stream completes.
    //
    // Only the participant turn's own progress frames drive typing. A compaction flush may run
    // inside the same `route_message` call (after the reply but before the outcome frame), and
    // its progress frames carry a different `turn_id` — those are an internal system detail the
    // connector must not surface. So we record the first turn_id we see and ignore frames from
    // any other turn.
    let typing_started = std::sync::atomic::AtomicBool::new(false);
    let typing_handle: SyncMutex<Option<tokio::task::JoinHandle<()>>> = SyncMutex::new(None);
    let active_turn_id: SyncMutex<Option<TurnId>> = SyncMutex::new(None);
    let refresh_secs = state.config.pacing.typing_refresh_secs;
    let ctx_for_typing = ctx.clone();
    let channel_for_typing = channel_id;

    let on_progress = |progress: &TurnProgress| {
        // Track which turn the first progress frame belongs to. Frames from a different turn
        // (a compaction flush) are ignored — the connector must not surface internal work.
        {
            let mut active = active_turn_id.lock();
            match *active {
                None => *active = Some(progress.turn_id),
                Some(id) if id != progress.turn_id => return,
                _ => {}
            }
        }
        if progress.kind == ProgressKind::Reply
            && !typing_started.swap(true, std::sync::atomic::Ordering::Relaxed)
        {
            let ctx_clone = ctx_for_typing.clone();
            let channel = channel_for_typing;
            *typing_handle.lock() = Some(tokio::spawn(async move {
                let interval = std::time::Duration::from_secs(refresh_secs);
                loop {
                    let _ = channel.broadcast_typing(&ctx_clone.http).await;
                    tokio::time::sleep(interval).await;
                }
            }));
        }
    };

    // Send via the streaming endpoint, processing progress as it arrives.
    let outcome = match state
        .platform
        .send_message_stream(locator, &messages, present, on_progress)
        .await
    {
        Ok(outcome) => outcome,
        Err(error) => {
            tracing::warn!(%error, "discord connector: platform stream failed");
            return;
        }
    };

    // Abort the typing task — the outcome has arrived.
    if let Some(handle) = typing_handle.lock().take() {
        handle.abort();
    }

    match outcome {
        StreamOutcome::Outcome(response) => match response.outcome {
            TurnOutcome::Reply(reply_text) => match channel_id.say(&ctx.http, &reply_text).await {
                Ok(sent_msg) => {
                    // Record the last participant turn id (the most recent message) for
                    // [turn:<id>] injection when a user replies to the bot's message.
                    if let Some(tid_str) = response.participant_turn_ids.last()
                        && let Ok(tid) = tid_str.parse::<ulid::Ulid>()
                    {
                        let mut turn_map = state.turn_map.lock().await;
                        turn_map.record(sent_msg.id, TurnId(tid));
                    }
                }
                Err(error) => {
                    tracing::warn!(%error, "discord connector: could not post reply");
                }
            },
            TurnOutcome::Silent => {}
            TurnOutcome::MaxStepsExceeded => {
                tracing::warn!("discord connector: turn exceeded max steps");
            }
            TurnOutcome::Deferred => {
                tracing::info!("discord connector: turn deferred");
            }
        },
        StreamOutcome::Error(error) => {
            tracing::warn!(%error, "discord connector: turn error from platform");
        }
    }
}

/// A `TypeMap` key for the bot state.
pub struct BotStateKey;

impl TypeMapKey for BotStateKey {
    type Value = Arc<BotState>;
}
