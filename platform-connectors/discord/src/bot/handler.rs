//! The bot event handler: the `EventHandler` implementation that wires Discord events to the
//! platform client.
//!
//! The main flow is `message`: check addressing, debounce, construct the locator, gather the
//! present set, inject `[turn:<id>]` if replying to a mapped message, call the platform API stream,
//! watch for reply progress to start the typing indicator, and post the outcome back to Discord.

use std::collections::HashMap;

use serenity::{
    all::{GuildChannel, GuildId, Member, Message, Ready, ResumedEvent, User, UserId},
    async_trait,
    prelude::*,
};

use zuihitsu_core::ids::{MemoryId, PersonId};
use zuihitsu_platform_connector_api::LinkEndpoint;

use crate::{
    addressing::{AddressingDecision, MessageContext, should_respond},
    bot::{
        BotStateKey,
        identity::{
            channel_metadata, guild_name, observed_identity, observed_mention_identity,
            sync_guild_name,
        },
        mentions::splice_mentions,
        process::process_message,
    },
    context_sync::ContextParams,
    locator::{ChannelContext, DISCORD_PLATFORM},
    pacing::PendingMessage,
};

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
        // mislead the subject guard). The bot's own mention resolves to the agent's reserved `self`
        // memory instead — never projected as a person, but referenced like any other mention. Each
        // projection caches the mentioned user's memory id, which the splice below reads to rewrite the
        // raw `<@id>` mention as a canonical `[mem:<id>]` token.
        let mut mention_memory_ids: HashMap<UserId, MemoryId> = HashMap::new();
        for user in &msg.mentions {
            if user.id == bot_id {
                // The agent must never be minted as a person, so the bot's own mention is not projected.
                // It still resolves to a reference — the agent's reserved `self` memory, fetched from the
                // server once per boot and cached. A failed self lookup degrades the mention to its raw
                // form rather than dropping the message, matching the failed-projection fallback below.
                match state
                    .self_memory
                    .get_or_try_init(|| async {
                        state
                            .platform
                            .self_memory()
                            .await
                            .map(|body| body.memory_id)
                    })
                    .await
                {
                    Ok(&self_id) => {
                        mention_memory_ids.insert(user.id, self_id);
                    }
                    Err(error) => {
                        tracing::warn!(
                            %error,
                            "discord connector: could not resolve the agent's self memory; leaving the bot mention raw"
                        );
                    }
                }
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
        // injection already folded into `text` above. The bot's own mention resolves to the agent's
        // reserved `self` memory (never projected as a person); only a failed projection or self lookup
        // leaves a mention in its raw form.
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
