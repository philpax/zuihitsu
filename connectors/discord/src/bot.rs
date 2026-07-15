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

use zuihitsu_connector_api::{PlatformClient, PlatformMessage, StreamOutcome, TurnOutcome};
use zuihitsu_core::{
    ids::{ConversationLocator, TurnId},
    progress::{ProgressKind, TurnProgress},
};

use crate::{
    addressing::{AddressingDecision, MessageContext, should_respond},
    config::DiscordConfig,
    context_sync::{ContextParams, ContextSync},
    locator::ChannelContext,
    pacing::{DebounceState, PendingMessage},
    turn_map::TurnMap,
};

/// The shared bot state, stored in serenity's `TypeMap` via `Arc`.
pub struct BotState {
    pub config: DiscordConfig,
    pub platform: PlatformClient,
    pub bot_id: Mutex<Option<UserId>>,
    pub turn_map: Mutex<TurnMap>,
    pub context_sync: ContextSync,
    /// Per-channel present sets: users who have spoken in a channel the bot operates in. Grown
    /// lazily — a user is added when they send a message the bot processes, not eagerly from the
    /// guild member list. Keyed by channel so presence is per-conversation, not global. A user who
    /// leaves the guild is removed from every channel they were in.
    pub present_members: Mutex<HashMap<ChannelId, HashSet<UserId>>>,
    pub debounce: DebounceState,
}

impl BotState {
    pub fn new(config: DiscordConfig) -> Self {
        let debounce_ms = config.pacing.debounce_ms;
        let connector_id = config.server.connector_id.clone();
        let platform = PlatformClient::new(
            config.server.url.clone(),
            config.server.platform_key.clone(),
        );
        BotState {
            platform,
            config,
            bot_id: Mutex::new(None),
            turn_map: Mutex::new(TurnMap::new()),
            context_sync: ContextSync::new(connector_id),
            present_members: Mutex::new(HashMap::new()),
            debounce: DebounceState::new(debounce_ms),
        }
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
        let guild_name = msg
            .guild_id
            .and_then(|g| ctx.cache.guild(g))
            .map(|g| g.name.clone())
            .unwrap_or_default();
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

        // Gather the present set (for DMs, it's [sender, bot]).
        let present: Vec<String> = if is_dm {
            vec![msg.author.id.to_string(), bot_id.to_string()]
        } else {
            let present = state.present_members.lock().await;
            present
                .get(&msg.channel_id)
                .map(|set| set.iter().map(|id| id.to_string()).collect())
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
        if let Some(state) = data.get::<BotStateKey>() {
            // Remove the departing user from every channel's present set.
            let mut present = state.present_members.lock().await;
            for (_, set) in present.iter_mut() {
                set.remove(&user.id);
            }
        }
        // Departures are eventless by design.
        let _ = guild_id;
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

    async fn resume(&self, _ctx: Context, _: ResumedEvent) {
        tracing::info!("discord connector: gateway resumed");
    }
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
    present: &[String],
    channel_id: serenity::all::ChannelId,
) {
    let present_refs: Vec<&str> = present.iter().map(String::as_str).collect();
    let messages: Vec<PlatformMessage> = batch
        .into_iter()
        .map(|m| PlatformMessage {
            sender: m.sender,
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
        .send_message_stream(locator, &messages, &present_refs, on_progress)
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
