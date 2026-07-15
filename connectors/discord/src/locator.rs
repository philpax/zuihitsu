//! Locator mapping: Discord channel types to `ConversationLocator`.
//!
//! Maps Discord contexts to the `(platform, scope_path)` pair the zuihitsu platform API expects.
//! Pure functions, no side effects.

use smol_str::SmolStr;
use zuihitsu_core::ids::ConversationLocator;

/// The platform key for Discord — matches the `ConversationLocator` convention.
pub const DISCORD_PLATFORM: &str = "discord";

/// A Discord channel's identifying context, as an enum — the shape of the scope path depends on
/// which variant it is, so optional fields would be a smell.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChannelContext {
    /// A guild channel: `guild/{guild_id}/channel/{channel_id}`.
    Guild { guild_id: u64, channel_id: u64 },
    /// A thread within a guild channel: `guild/{guild_id}/channel/{channel_id}/thread/{thread_id}`. Used
    /// in tests; the bot doesn't construct it yet (thread messages arrive as guild messages with a
    /// thread parent), but the variant is part of the type's contract.
    #[allow(dead_code)]
    Thread {
        guild_id: u64,
        channel_id: u64,
        thread_id: u64,
    },
    /// A 1:1 or group DM: `dm/{channel_id}`.
    DirectMessage { channel_id: u64 },
}

impl ChannelContext {
    /// Build a `ConversationLocator` from this channel context.
    pub fn locator(&self) -> ConversationLocator {
        let scope_path = match self {
            ChannelContext::Guild {
                guild_id,
                channel_id,
            } => {
                format!("guild/{guild_id}/channel/{channel_id}")
            }
            ChannelContext::Thread {
                guild_id,
                channel_id,
                thread_id,
            } => format!("guild/{guild_id}/channel/{channel_id}/thread/{thread_id}"),
            ChannelContext::DirectMessage { channel_id } => format!("dm/{channel_id}"),
        };
        ConversationLocator::new(SmolStr::new(DISCORD_PLATFORM), SmolStr::new(scope_path))
    }

    /// Whether this is a DM (not a guild channel).
    pub fn is_dm(&self) -> bool {
        matches!(self, ChannelContext::DirectMessage { .. })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locator_guild_channel() {
        let ctx = ChannelContext::Guild {
            guild_id: 100,
            channel_id: 200,
        };
        let loc = ctx.locator();
        assert_eq!(loc.platform.as_str(), "discord");
        assert_eq!(loc.scope_path.as_str(), "guild/100/channel/200");
    }

    #[test]
    fn locator_thread() {
        let ctx = ChannelContext::Thread {
            guild_id: 100,
            channel_id: 200,
            thread_id: 300,
        };
        let loc = ctx.locator();
        assert_eq!(loc.scope_path.as_str(), "guild/100/channel/200/thread/300");
    }

    #[test]
    fn locator_dm() {
        let ctx = ChannelContext::DirectMessage { channel_id: 400 };
        let loc = ctx.locator();
        assert_eq!(loc.scope_path.as_str(), "dm/400");
    }

    #[test]
    fn locator_group_dm() {
        // A group DM arrives as guild_id = None, channel_id = the group DM's id.
        let ctx = ChannelContext::DirectMessage { channel_id: 500 };
        let loc = ctx.locator();
        assert_eq!(loc.scope_path.as_str(), "dm/500");
    }
}
