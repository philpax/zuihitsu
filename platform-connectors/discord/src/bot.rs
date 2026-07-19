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

use zuihitsu_core::{
    ids::{ConversationLocator, MemoryId, PersonId, TurnId},
    mem_ref,
    progress::{ProgressKind, TurnProgress},
};
use zuihitsu_platform_connector_api::{
    LinkEndpoint, PlatformClient, PlatformMessage, StreamOutcome, TurnOutcome,
};

use crate::{
    addressing::{AddressingDecision, MessageContext, should_respond},
    bot_loop::BotLoopGuard,
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

        // Never process our own messages — matched precisely by id, not the coarse bot flag, so that
        // other bots in the channel remain visible. Whether they are forwarded is decided below.
        if msg.author.id == bot_id {
            return;
        }
        let author_is_other_bot = msg.author.bot;

        // A human message breaks any bot-to-bot streak in this channel, clearing the loop guard.
        if !author_is_other_bot {
            state.bot_loop.note_human(msg.channel_id);
        }

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
            author_is_bot: author_is_other_bot,
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

        // Loop safeguard: cap consecutive turns another bot may initiate, so two agents cannot answer
        // each other forever. Trips only for other bots — a human message clears the streak above.
        if author_is_other_bot && !state.bot_loop.admit_bot(msg.channel_id) {
            tracing::warn!(
                channel_id = msg.channel_id.get(),
                "discord connector: bot-to-bot loop guard tripped; dropping further bot messages until a human speaks"
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
        // the connector's own bot, whose messages are filtered by id above — so the agent's own
        // identity is never minted as another entity. Another bot, seen as a participant, is projected
        // like any other sender.
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

        // Project every @mentioned user's identity — username and display name, the fields a mention
        // carries — the same path the sender takes, minus presence: a mentioned user is referenced, not
        // present, so it is never added to the channel's present set (which would mint a phantom stub and
        // mislead the subject guard). The bot's own mention is addressing, not a reference, so it is
        // skipped. Each projection caches the mentioned user's memory id, which the splice below reads to
        // rewrite the raw `<@id>` mention as a canonical `[mem:<id>]` token.
        let mut mention_memory_ids: HashMap<UserId, MemoryId> = HashMap::new();
        for user in &msg.mentions {
            if user.id == bot_id {
                continue;
            }
            let person = PersonId::new(DISCORD_PLATFORM, user.id.to_string());
            if let Err(error) = state
                .projection_sync
                .sync(
                    &state.platform,
                    &LinkEndpoint::Participant(person.clone()),
                    person.id.as_str(),
                    &observed_mention_identity(user),
                )
                .await
            {
                // A failed projection degrades to the raw mention — the message still posts, just
                // without a spliced token for this user. Never a dropped message.
                tracing::warn!(
                    %error,
                    user_id = user.id.get(),
                    "discord connector: could not project mentioned user identity"
                );
                continue;
            }
            if let Some(memory_id) = state
                .projection_sync
                .memory_id_for(person.id.as_str())
                .await
            {
                mention_memory_ids.insert(user.id, memory_id);
            }
        }
        // Splice a `[mem:<id>]` token in place of each projected mention, alongside the reply-turnref
        // injection already folded into `text` above. The bot's own mention keeps its raw form.
        let text = splice_mentions(&text, &mention_memory_ids);

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

/// The identity attributes to project for an @mentioned user: the account username and display name a
/// mention carries. A mention's `User` has no guild member data, so no nickname is observed — the keys
/// match [`observed_identity`]'s `username` and `display_name` exactly, so a later message from this user
/// as a sender adds the nickname and updates in place rather than fighting the mention's entries.
fn observed_mention_identity(user: &User) -> Vec<ObservedAttribute> {
    vec![
        ObservedAttribute {
            key: "username".to_owned(),
            value: Some(user.name.clone()),
            entry_text: format!("Discord username: {}", user.name),
        },
        ObservedAttribute {
            key: "display_name".to_owned(),
            entry_text: user
                .global_name
                .as_deref()
                .map(|name| format!("Discord display name: {name}"))
                .unwrap_or_default(),
            value: user.global_name.clone(),
        },
    ]
}

/// Rewrite each raw Discord mention (`<@id>` and the nickname form `<@!id>`) of a projected user as the
/// canonical `[mem:<id>]` memory token, so the agent reads a stable reference rather than an opaque
/// platform mention and the console renders a link. A user absent from `memory_ids` keeps its raw
/// mention: the bot's own mention (addressing, not reference) is never in the map, and a mention whose
/// projection failed degrades to its raw form. Parsing `<@…>` is the connector reading its own platform's
/// syntax.
///
/// A mention inside a Discord code span — a backtick-delimited inline run or a triple-backtick fenced
/// block — is left raw, since there it is a literal code sample, not a reference. An unclosed backtick
/// run is not a span at all (Discord renders the backticks literally), so scanning resumes after it and
/// any mention in the prose beyond still splices.
fn splice_mentions(text: &str, memory_ids: &HashMap<UserId, MemoryId>) -> String {
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'`' {
            let run = backtick_run(bytes, i);
            if let Some(end) = closing_run(bytes, i + run, run) {
                // A closed code span: copy it verbatim, backticks and content alike, so a mention
                // inside stays a literal code sample.
                out.push_str(&text[i..end]);
                i = end;
            } else {
                // An unclosed run: the backticks are literal, not a span. Emit them and resume normal
                // scanning, so a mention in the prose beyond still splices.
                out.push_str(&text[i..i + run]);
                i += run;
            }
            continue;
        }
        if let Some((user_id, len)) = mention_at(text, i)
            && let Some(memory_id) = memory_ids.get(&user_id)
        {
            out.push_str(&mem_ref::construct(*memory_id));
            i += len;
            continue;
        }
        // Not a mention we splice: copy one whole character so the scan never slices mid-character.
        let ch = text[i..].chars().next().expect("i is a char boundary");
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

/// The length in bytes of the run of backticks starting at `i` (where `bytes[i]` is a backtick). Backticks
/// are ASCII, so the run boundary is always a character boundary.
fn backtick_run(bytes: &[u8], i: usize) -> usize {
    let mut n = 0;
    while i + n < bytes.len() && bytes[i + n] == b'`' {
        n += 1;
    }
    n
}

/// The byte offset just past the backtick run that closes a code span of length `run`, searching from
/// `from`, or `None` when the span never closes. Discord closes a span on the next run of exactly the
/// opening length: a run of a different length is skipped whole, so its backticks are never miscounted as
/// a partial close.
fn closing_run(bytes: &[u8], from: usize, run: usize) -> Option<usize> {
    let mut j = from;
    while j < bytes.len() {
        if bytes[j] == b'`' {
            let len = backtick_run(bytes, j);
            if len == run {
                return Some(j + run);
            }
            j += len;
        } else {
            j += 1;
        }
    }
    None
}

/// The Discord mention starting at byte `i`, if `text[i..]` opens `<@id>` or the nickname form `<@!id>`
/// with a numeric id, and the mention's byte length. `None` when no mention opens there.
fn mention_at(text: &str, i: usize) -> Option<(UserId, usize)> {
    let rest = text.get(i..)?.strip_prefix("<@")?;
    // The nickname form carries a leading `!` before the id; both forms name the same user.
    let after_bang = rest.strip_prefix('!').unwrap_or(rest);
    let bang_len = rest.len() - after_bang.len();
    let digits: String = after_bang
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    if digits.is_empty() || !after_bang[digits.len()..].starts_with('>') {
        return None;
    }
    let id = digits.parse::<u64>().ok()?;
    // "<@" + optional "!" + digits + ">".
    let len = "<@".len() + bang_len + digits.len() + '>'.len_utf8();
    Some((UserId::new(id), len))
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
            TurnOutcome::Superseded => {
                // A newer inbound batch superseded this turn: normal operation, like `Deferred`. No
                // reply to post and no `turn_map` record — the successor's turn answers with
                // everything in context, and its reply reaches the channel through its own stream.
                tracing::info!("discord connector: turn superseded by a newer message batch");
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

#[cfg(test)]
mod tests {
    use super::*;

    fn memory_id(bits: u128) -> MemoryId {
        MemoryId(ulid::Ulid::from(bits))
    }

    #[test]
    fn splice_rewrites_both_mention_forms_of_a_projected_user() {
        let dave = UserId::new(123);
        let mem = memory_id(1);
        let map = HashMap::from([(dave, mem)]);
        let token = mem_ref::construct(mem);

        // Both the plain and the nickname form of the same user splice to the same token.
        assert_eq!(
            splice_mentions("hey <@123> around?", &map),
            format!("hey {token} around?")
        );
        assert_eq!(
            splice_mentions("hey <@!123> around?", &map),
            format!("hey {token} around?")
        );
    }

    #[test]
    fn splice_leaves_an_unprojected_mention_raw() {
        // The bot's own mention (and any user whose projection failed) is absent from the map, so its
        // raw form is preserved verbatim — addressing, not a reference.
        let map: HashMap<UserId, MemoryId> = HashMap::new();
        assert_eq!(splice_mentions("<@999> hello", &map), "<@999> hello");
        assert_eq!(splice_mentions("<@!999> hi", &map), "<@!999> hi");
    }

    #[test]
    fn splice_rewrites_only_projected_users_among_several_mentions() {
        let dave = UserId::new(123);
        let mem = memory_id(2);
        let map = HashMap::from([(dave, mem)]);
        let token = mem_ref::construct(mem);
        // Dave splices; Erin (unprojected) and the surrounding prose stay exactly as written.
        assert_eq!(
            splice_mentions("cc <@123> and <@456> please", &map),
            format!("cc {token} and <@456> please")
        );
    }

    #[test]
    fn splice_preserves_non_mention_and_multibyte_text() {
        let map: HashMap<UserId, MemoryId> = HashMap::new();
        // A lone `<`, an email-ish `@`, and multibyte prose must scan without panicking or corruption.
        for text in [
            "plain text",
            "a < b and c@d",
            "さっき <@ not a mention",
            "emoji 🎉 done",
        ] {
            assert_eq!(splice_mentions(text, &map), text);
        }
    }

    #[test]
    fn splice_leaves_a_mention_in_inline_code_raw() {
        let dave = UserId::new(123);
        let map = HashMap::from([(dave, memory_id(3))]);
        // A mention inside a backtick-delimited inline run is a literal code sample, not a reference.
        assert_eq!(
            splice_mentions("use `<@123>` to ping", &map),
            "use `<@123>` to ping"
        );
        // A double-backtick run (used so the content may itself contain a single backtick) closes only
        // on a matching double run, leaving the mention within it raw.
        assert_eq!(
            splice_mentions("run ``<@123>`` now", &map),
            "run ``<@123>`` now"
        );
    }

    #[test]
    fn splice_leaves_a_mention_in_a_fenced_block_raw() {
        let dave = UserId::new(123);
        let map = HashMap::from([(dave, memory_id(4))]);
        // A triple-backtick fenced block copies through untouched, mention and all.
        assert_eq!(
            splice_mentions("```\nping <@123> here\n```", &map),
            "```\nping <@123> here\n```"
        );
    }

    #[test]
    fn splice_rewrites_prose_but_not_code_in_a_mixed_message() {
        let dave = UserId::new(123);
        let mem = memory_id(5);
        let map = HashMap::from([(dave, mem)]);
        let token = mem_ref::construct(mem);
        // The prose mention splices; the identical mention inside the inline code stays a raw sample.
        assert_eq!(
            splice_mentions("cc <@123> — sample `<@123>`", &map),
            format!("cc {token} — sample `<@123>`")
        );
    }

    #[test]
    fn splice_treats_an_unclosed_backtick_run_as_literal_and_splices_past_it() {
        let dave = UserId::new(123);
        let mem = memory_id(6);
        let map = HashMap::from([(dave, mem)]);
        let token = mem_ref::construct(mem);
        // Discord renders an unclosed backtick literally, so it opens no span: the backtick is emitted
        // as-is and the mention beyond it still splices.
        assert_eq!(
            splice_mentions("oops `<@123> unclosed", &map),
            format!("oops `{token} unclosed")
        );
    }

    #[test]
    fn mention_at_rejects_malformed_forms() {
        // No id, a non-numeric id, and an unterminated mention are not mentions.
        assert_eq!(mention_at("<@>", 0), None);
        assert_eq!(mention_at("<@abc>", 0), None);
        assert_eq!(mention_at("<@123", 0), None);
        // A well-formed mention reports the right byte length (plain and nickname forms).
        assert_eq!(mention_at("<@123>", 0).map(|(_, len)| len), Some(6));
        assert_eq!(mention_at("<@!123>", 0).map(|(_, len)| len), Some(7));
    }

    #[test]
    fn observed_mention_identity_matches_the_sender_username_shape() {
        // The mention's username and display-name attributes carry the same keys and entry-text shape as
        // the sender's, so a later sender message updates in place rather than fighting the mention's
        // entries. A `User` default carries an empty global name, so display_name is cleared (`None`).
        let mut user = User::default();
        user.name = "dave1234".to_owned();
        let observed = observed_mention_identity(&user);
        assert_eq!(observed[0].key, "username");
        assert_eq!(observed[0].value.as_deref(), Some("dave1234"));
        assert_eq!(observed[0].entry_text, "Discord username: dave1234");
        assert_eq!(observed[1].key, "display_name");
        assert_eq!(observed[1].value, None);
    }
}
