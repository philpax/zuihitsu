//! Guild links: asserts the structural `part_of` links that place a channel and its members in a
//! guild, retracting a member's placement when they leave.
//!
//! A guild is a first-class scope: a channel or a member is tied to it by a `part_of` link (the
//! server's name is projected onto the guild's `context/*` memory separately, by the projection sync).
//! Guild membership is durable — asserted when a member is first seen, retracted when they leave —
//! whereas channel presence stays ephemeral (the per-channel present set). The platform link endpoint
//! is idempotent, so the in-memory sets here are a chattiness guard, not a correctness one.

use std::collections::HashSet;

use tokio::sync::Mutex;

use zuihitsu_core::{
    ids::{ConversationLocator, PersonId},
    vocabulary::RelationName,
};
use zuihitsu_platform_connector_api::{LinkEndpoint, PlatformClient};

use crate::{error::Result, locator::guild_locator};

/// Tracks which channels and members have already had their `part_of` guild link asserted, so it
/// lands once rather than on every message.
#[derive(Default)]
pub struct GuildSync {
    /// Channel ids whose `part_of` guild link has been asserted.
    linked_channels: Mutex<HashSet<u64>>,
    /// `(guild_id, user_id)` pairs whose member `part_of` guild link has been asserted.
    linked_members: Mutex<HashSet<(u64, u64)>>,
}

impl GuildSync {
    pub fn new() -> Self {
        GuildSync::default()
    }

    /// Assert that a channel is `part_of` its guild, once per channel.
    pub async fn link_channel(
        &self,
        client: &PlatformClient,
        guild_id: u64,
        channel_locator: &ConversationLocator,
        channel_id: u64,
    ) -> Result<()> {
        if !self.linked_channels.lock().await.insert(channel_id) {
            return Ok(());
        }
        client
            .link(
                &LinkEndpoint::Context(channel_locator.clone()),
                &LinkEndpoint::Context(guild_locator(guild_id)),
                RelationName::PartOf.as_str(),
                false,
            )
            .await?;
        Ok(())
    }

    /// Assert that a member is `part_of` a guild, once per `(guild, member)`.
    pub async fn link_member(
        &self,
        client: &PlatformClient,
        guild_id: u64,
        person: &PersonId,
        user_id: u64,
    ) -> Result<()> {
        if !self.linked_members.lock().await.insert((guild_id, user_id)) {
            return Ok(());
        }
        client
            .link(
                &LinkEndpoint::Participant(person.clone()),
                &LinkEndpoint::Context(guild_locator(guild_id)),
                RelationName::PartOf.as_str(),
                false,
            )
            .await?;
        Ok(())
    }

    /// Retract a member's `part_of` guild link when they leave, so departed members do not linger as
    /// current members. A retract naming an unknown member is a server-side no-op.
    pub async fn unlink_member(
        &self,
        client: &PlatformClient,
        guild_id: u64,
        person: &PersonId,
        user_id: u64,
    ) -> Result<()> {
        self.linked_members
            .lock()
            .await
            .remove(&(guild_id, user_id));
        client
            .link(
                &LinkEndpoint::Participant(person.clone()),
                &LinkEndpoint::Context(guild_locator(guild_id)),
                RelationName::PartOf.as_str(),
                true,
            )
            .await?;
        Ok(())
    }
}
