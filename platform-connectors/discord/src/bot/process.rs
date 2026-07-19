//! Driving a debounced batch through the platform stream: send it, watch progress to start and
//! refresh the typing indicator, and post the outcome back to Discord.

use std::sync::Arc;

use parking_lot::Mutex as SyncMutex;
use serenity::prelude::Context;

use zuihitsu_core::{
    ids::{ConversationLocator, PersonId, TurnId},
    progress::{ProgressKind, TurnProgress},
};
use zuihitsu_platform_connector_api::{PlatformMessage, StreamOutcome, TurnOutcome};

use crate::{bot::BotState, locator::DISCORD_PLATFORM, pacing::PendingMessage};

/// Process a debounced batch: send it to the platform, watch progress, post the outcome.
pub(super) async fn process_message(
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
