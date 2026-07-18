//! Context sync: writes channel metadata + laconic guidance to the context memory on first contact
//! and when channel metadata changes.

use std::collections::{HashMap, HashSet};

use serenity::model::id::ChannelId;
use tokio::sync::Mutex;

use zuihitsu_platform_connector_api::{ContextEntry, PlatformClient};

use crate::error::Result;

/// The laconic guidance text for a Discord guild channel.
const CHANNEL_GUIDANCE: &str = "This is a Discord channel. Be laconic — one paragraph at most. \
    Use Discord markdown sparingly. Don't acknowledge every message.";

/// The laconic guidance text for a Discord DM.
const DM_GUIDANCE: &str = "This is a Discord DM. Be conversational but still concise.";

/// Tracks which channels have already had their context written, and the last-seen channel
/// name/topic for change detection.
#[derive(Default)]
pub struct ContextSync {
    seen: Mutex<HashSet<ChannelId>>,
    last_metadata: Mutex<HashMap<ChannelId, (String, String)>>,
}

/// Parameters for context setup, bundled to stay under the argument count threshold.
pub struct ContextParams<'a> {
    pub channel_id: ChannelId,
    pub guild_name: &'a str,
    pub channel_name: &'a str,
    pub topic: &'a str,
    pub is_dm: bool,
}

impl ContextSync {
    pub fn new() -> Self {
        ContextSync::default()
    }

    /// On first contact with a channel, write the context via `POST /platform/context`.
    /// Returns `true` if the context was written (first contact), `false` if already seen.
    pub async fn ensure_context(
        &self,
        client: &PlatformClient,
        locator: &zuihitsu_core::ids::ConversationLocator,
        params: &ContextParams<'_>,
    ) -> Result<bool> {
        // Check if we've already seen this channel.
        if self.seen.lock().await.contains(&params.channel_id) {
            return Ok(false);
        }

        // First contact: write the context.
        let metadata = if params.is_dm {
            DM_GUIDANCE.to_owned()
        } else {
            format!(
                "Channel: {} / {}. Topic: {}. {CHANNEL_GUIDANCE}",
                params.guild_name, params.channel_name, params.topic
            )
        };
        let entries = vec![ContextEntry { text: metadata }];
        client.write_context(locator, &entries).await?;

        // Mark as seen and record the metadata.
        self.seen.lock().await.insert(params.channel_id);
        self.last_metadata.lock().await.insert(
            params.channel_id,
            (params.channel_name.to_owned(), params.topic.to_owned()),
        );

        Ok(true)
    }

    /// On a channel update, re-write the context if the name or topic changed. Returns `true` if
    /// the context was updated.
    pub async fn update_context(
        &self,
        client: &PlatformClient,
        locator: &zuihitsu_core::ids::ConversationLocator,
        channel_id: ChannelId,
        guild_name: &str,
        channel_name: &str,
        topic: &str,
    ) -> Result<bool> {
        let key = (channel_name.to_owned(), topic.to_owned());

        // Check if the metadata changed.
        if self.last_metadata.lock().await.get(&channel_id) == Some(&key) {
            return Ok(false);
        }

        // Metadata changed: re-write the context.
        let metadata =
            format!("Channel: {guild_name} / {channel_name}. Topic: {topic}. {CHANNEL_GUIDANCE}");
        let entries = vec![ContextEntry { text: metadata }];
        client.write_context(locator, &entries).await?;

        // Update the recorded metadata.
        self.last_metadata.lock().await.insert(channel_id, key);

        Ok(true)
    }
}
