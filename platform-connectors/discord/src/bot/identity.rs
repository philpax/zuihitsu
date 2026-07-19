//! Identity and metadata projection helpers: what the connector observes about a sender, a
//! mentioned user, and a guild, plus the channel and guild metadata reads the message flow needs.

use serenity::{
    all::{GuildId, Message, User},
    prelude::Context,
};

use zuihitsu_platform_connector_api::LinkEndpoint;

use crate::{
    bot::BotState, error::Result, locator::guild_locator, projection_sync::ObservedAttribute,
};

/// The identity attributes to project for a message's sender: the account username and display name
/// (global to Discord), and the server nickname (per guild, keyed by guild id). An unset display name
/// or nickname is carried as `None`, so clearing it later retracts the prior projection rather than
/// leaving a stale handle. The nickname is only observed in a guild — a DM has none.
pub(super) fn observed_identity(msg: &Message, guild_name: &str) -> Vec<ObservedAttribute> {
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
pub(super) fn observed_mention_identity(user: &User) -> Vec<ObservedAttribute> {
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

/// Project a guild's server name onto its context memory, superseding it on a rename. A blank name (the
/// cache cold and the fetch failed) is skipped rather than recorded as an empty attribute.
pub(super) async fn sync_guild_name(
    state: &BotState,
    guild_id: u64,
    guild_name: &str,
) -> Result<()> {
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
pub(super) async fn guild_name(ctx: &Context, guild_id: GuildId) -> String {
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
pub(super) async fn channel_metadata(
    ctx: &Context,
    channel_id: serenity::all::ChannelId,
) -> (String, String) {
    match channel_id.to_channel(&ctx.http).await {
        Ok(serenity::all::Channel::Guild(gc)) => {
            (gc.name.clone(), gc.topic.clone().unwrap_or_default())
        }
        _ => (String::new(), String::new()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
