//! Addressing model: the cheap filter that decides whether to forward a message to the platform API.
//!
//! This runs before the agent's smart filter (the stay-silent terminal). It always drops bot messages
//! and messages in channels not on the allow-list; within an allowed channel it forwards either every
//! message or only ones addressing the bot, per [`ReplyMode`].

use crate::config::{BehaviorConfig, ReplyMode};

/// The addressing decision for an inbound message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AddressingDecision {
    /// Whether the bot should forward this message to the platform API.
    pub should_forward: bool,
    /// Whether this is a direct address (a DM or a mention) — the agent should always respond.
    pub is_direct: bool,
}

/// The message fields the addressing model needs, extracted so the function is testable without a
/// full serenity `Message`.
#[derive(Clone, Debug)]
pub struct MessageContext {
    pub author_is_bot: bool,
    pub guild_id: Option<u64>,
    pub channel_id: u64,
    pub mentions_bot: bool,
    /// Whether the message is a reply to the bot.
    pub replies_to_bot: bool,
}

/// Decide whether to forward a message.
///
/// Rules:
/// - **Ignore bot messages** — never forward.
/// - **DMs** (`guild_id.is_none()`) → always forward, `is_direct = true`.
/// - **Guild channels**: messages in channels not in `allowed_channels` are silently dropped. Within
///   an allowed channel, [`ReplyMode::All`] (the default) forwards every message and
///   [`ReplyMode::Addressed`] forwards only a mention or a reply to the bot. `is_direct` marks the
///   latter either way, so the agent still knows which messages spoke to it directly.
pub fn should_respond(msg: &MessageContext, config: &BehaviorConfig) -> AddressingDecision {
    // Never forward messages from bots.
    if msg.author_is_bot {
        return AddressingDecision {
            should_forward: false,
            is_direct: false,
        };
    }

    // DMs are always forwarded.
    if msg.guild_id.is_none() {
        return AddressingDecision {
            should_forward: true,
            is_direct: true,
        };
    }

    // Guild channel: check if it's in the allowed set (if the set is non-empty).
    if !config.allowed_channels.is_empty() {
        let channel_allowed = config
            .allowed_channels
            .iter()
            .any(|&id| id.get() == msg.channel_id);
        if !channel_allowed {
            return AddressingDecision {
                should_forward: false,
                is_direct: false,
            };
        }
    }

    // In an allowed guild channel: forward per the reply mode. A mention or reply-to-bot is always a
    // direct address, whether or not the mode also forwards the un-addressed chatter around it.
    let is_direct = msg.mentions_bot || msg.replies_to_bot;
    let should_forward = match config.reply_to {
        ReplyMode::All => true,
        ReplyMode::Addressed => is_direct,
    };
    AddressingDecision {
        should_forward,
        is_direct,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serenity::model::id::ChannelId;

    fn config_with_channels(channels: &[u64]) -> BehaviorConfig {
        BehaviorConfig {
            allowed_channels: channels.iter().map(|&c| ChannelId::new(c)).collect(),
            ..BehaviorConfig::default()
        }
    }

    fn config_addressed(channels: &[u64]) -> BehaviorConfig {
        BehaviorConfig {
            allowed_channels: channels.iter().map(|&c| ChannelId::new(c)).collect(),
            reply_to: ReplyMode::Addressed,
        }
    }

    #[test]
    fn addressing_ignores_bots() {
        let msg = MessageContext {
            author_is_bot: true,
            guild_id: Some(1),
            channel_id: 100,
            mentions_bot: true,
            replies_to_bot: false,
        };
        let config = config_with_channels(&[100]);
        let decision = should_respond(&msg, &config);
        assert!(!decision.should_forward);
    }

    #[test]
    fn addressing_dm_always() {
        let msg = MessageContext {
            author_is_bot: false,
            guild_id: None,
            channel_id: 200,
            mentions_bot: false,
            replies_to_bot: false,
        };
        let config = BehaviorConfig::default();
        let decision = should_respond(&msg, &config);
        assert!(decision.should_forward);
        assert!(decision.is_direct);
    }

    #[test]
    fn addressing_mention() {
        let msg = MessageContext {
            author_is_bot: false,
            guild_id: Some(1),
            channel_id: 100,
            mentions_bot: true,
            replies_to_bot: false,
        };
        let config = config_with_channels(&[100]);
        let decision = should_respond(&msg, &config);
        assert!(decision.should_forward);
        assert!(decision.is_direct);
    }

    #[test]
    fn addressing_reply_to_bot() {
        let msg = MessageContext {
            author_is_bot: false,
            guild_id: Some(1),
            channel_id: 100,
            mentions_bot: false,
            replies_to_bot: true,
        };
        let config = config_with_channels(&[100]);
        let decision = should_respond(&msg, &config);
        assert!(decision.should_forward);
        assert!(decision.is_direct);
    }

    #[test]
    fn addressing_disallowed_channel() {
        let msg = MessageContext {
            author_is_bot: false,
            guild_id: Some(1),
            channel_id: 999,
            mentions_bot: true,
            replies_to_bot: false,
        };
        let config = config_with_channels(&[100]);
        let decision = should_respond(&msg, &config);
        assert!(!decision.should_forward);
    }

    #[test]
    fn addressing_all_mode_forwards_unaddressed() {
        // The default reply mode forwards every message in an allowed channel; it just is not `direct`,
        // so the agent's own filter decides whether the un-addressed message warrants a reply.
        let msg = MessageContext {
            author_is_bot: false,
            guild_id: Some(1),
            channel_id: 100,
            mentions_bot: false,
            replies_to_bot: false,
        };
        let decision = should_respond(&msg, &config_with_channels(&[100]));
        assert!(decision.should_forward);
        assert!(!decision.is_direct);
    }

    #[test]
    fn addressing_addressed_mode_drops_unaddressed() {
        // Under `reply_to = "addressed"`, an un-addressed message in an allowed channel is dropped.
        let msg = MessageContext {
            author_is_bot: false,
            guild_id: Some(1),
            channel_id: 100,
            mentions_bot: false,
            replies_to_bot: false,
        };
        let decision = should_respond(&msg, &config_addressed(&[100]));
        assert!(!decision.should_forward);
    }

    #[test]
    fn addressing_addressed_mode_forwards_a_mention() {
        // ...but a mention is still forwarded, and marked direct.
        let msg = MessageContext {
            author_is_bot: false,
            guild_id: Some(1),
            channel_id: 100,
            mentions_bot: true,
            replies_to_bot: false,
        };
        let decision = should_respond(&msg, &config_addressed(&[100]));
        assert!(decision.should_forward);
        assert!(decision.is_direct);
    }
}
