//! The cooperative-cancellation handle a turn holds while it generates (spec §Concurrency →
//! per-conversation supersession).
//!
//! A [`Supersession`] is the turn-side view of the conversation's arrival epoch: the ledger bumps a
//! [`tokio::sync::watch`] channel every time a new inbound batch arrives, and the in-flight turn
//! checks the level at each cooperative boundary — between steps, between tool calls, and mid-stream
//! — to decide whether a newer batch has superseded it. The check is level-triggered on the epoch
//! counter rather than edge-triggered, so a batch that arrives while an earlier one is still queued
//! is still observed by whichever turn eventually holds the slot (the TOCTOU case the ledger relies
//! on). Cancellation is always cooperative: a running block is never interrupted, only the gaps
//! between them and the gaps between streamed fragments.
//!
//! A `window` caps repeated supersession so continuous chatter still gets an answer. It is measured
//! from `burst_started_at` — the arrival of the burst's first unanswered message, maintained by the
//! ledger — and once the turn is past it the handle latches disarmed permanently: the window cannot
//! be re-entered while this turn holds the slot, so the turn runs to completion and a later batch
//! queues behind it.

use std::time::Duration;

use tokio::sync::watch;

use crate::{clock::Clock, time::Timestamp};

/// A turn's view of its conversation's supersession signal: the shared arrival epoch, the epoch this
/// turn was admitted at, and the window that bounds how long it stays cancellable.
///
/// Cloneable so the same signal can be threaded to several call sites of one turn (the step loop
/// passes a fresh borrow to each `generate`); a clone shares the underlying watch channel, so every
/// copy observes the same arrivals.
#[derive(Clone)]
pub struct Supersession {
    /// The conversation's latest arrival epoch, bumped by the ledger on every new inbound batch.
    rx: watch::Receiver<u64>,
    /// The epoch this turn was admitted at. A watched value strictly greater than this means a newer
    /// batch has arrived since — the level the boundary checks read.
    epoch: u64,
    /// When the burst's first unanswered message arrived, the origin the window is measured from.
    burst_started_at: Timestamp,
    /// How long after `burst_started_at` this turn stays cancellable.
    window: Duration,
    /// Whether the window is still open. Latched to `false` the first time a check finds the turn
    /// past the window, so the window is never re-entered once left (a later `now` cannot un-elapse
    /// it while the turn holds its slot).
    armed: bool,
}

impl Supersession {
    /// Build a handle from the ledger's per-conversation state: the arrival-epoch `rx`, the `epoch`
    /// this turn was admitted at, the `burst_started_at` the window is measured from, and the
    /// `window` itself. `pub(crate)` because only the instance-side turn ledger constructs one.
    ///
    /// A zero window disables supersession entirely (spec §Concurrency → per-conversation
    /// supersession): the handle is born disarmed, so [`superseded`](Self::superseded) always returns
    /// `false` and [`wait`](Self::wait) pends forever. This is what makes `supersede_window_seconds =
    /// 0` mean "serialization stays on, but no turn is ever cancelled" — without it, a batch arriving
    /// in the same instant the burst began (`elapsed == 0`, not yet past the zero window) would still
    /// supersede, which the setting's contract forbids.
    pub(crate) fn new(
        rx: watch::Receiver<u64>,
        epoch: u64,
        burst_started_at: Timestamp,
        window: Duration,
    ) -> Supersession {
        Supersession {
            rx,
            epoch,
            burst_started_at,
            window,
            armed: !window.is_zero(),
        }
    }

    /// Whether a newer inbound batch has superseded this turn as of `now`: a level check that returns
    /// `true` only while the handle is armed, `now` is still within the window, and the watched epoch
    /// has advanced past this turn's. The first time `now` is found past the window the handle latches
    /// disarmed, so every later check returns `false` — the turn holds its slot to completion.
    pub fn superseded(&mut self, now: Timestamp) -> bool {
        if !self.armed {
            return false;
        }
        let elapsed = now
            .as_millis()
            .saturating_sub(self.burst_started_at.as_millis());
        if elapsed > self.window.as_millis() as i64 {
            self.armed = false;
            return false;
        }
        *self.rx.borrow() > self.epoch
    }

    /// Resolve when this turn is superseded, for use as a branch of a `tokio::select!` against the
    /// model stream. Pends forever once the handle is disarmed (past the window, or the ledger's
    /// sender dropped), so a turn that can no longer be cancelled simply never wins the select.
    /// Cancel-safe: the underlying [`watch::Receiver::changed`] is cancel-safe, and the level is
    /// re-read from the shared channel on each poll, so dropping the future mid-wait loses nothing.
    pub async fn wait(&mut self, clock: &dyn Clock) {
        loop {
            if !self.armed {
                std::future::pending::<()>().await;
            }
            if self.superseded(clock.now()) {
                return;
            }
            // Wait for the next arrival, then re-check the level against the current clock — a bare
            // change is not necessarily a supersession (the window may have since elapsed).
            if self.rx.changed().await.is_err() {
                // The ledger dropped its sender: no further arrival can come, so this turn can no
                // longer be superseded.
                std::future::pending::<()>().await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Supersession;
    use crate::time::Timestamp;
    use std::time::Duration;
    use tokio::sync::watch;

    fn stamp(millis: i64) -> Timestamp {
        Timestamp::from_millis(millis)
    }

    #[test]
    fn a_newer_epoch_within_the_window_supersedes() {
        let (tx, rx) = watch::channel(0u64);
        let mut sup = Supersession::new(rx, 0, stamp(0), Duration::from_secs(60));
        // No newer batch yet.
        assert!(!sup.superseded(stamp(1_000)));
        // A newer batch arrives; within the window it supersedes.
        tx.send(1).unwrap();
        assert!(sup.superseded(stamp(2_000)));
    }

    #[test]
    fn the_admitting_turns_own_epoch_does_not_supersede_it() {
        // Batch 2 admitted at epoch 1 must not read its own arrival as a supersession.
        let (_tx, rx) = watch::channel(1u64);
        let mut sup = Supersession::new(rx, 1, stamp(0), Duration::from_secs(60));
        assert!(!sup.superseded(stamp(1_000)));
    }

    #[test]
    fn an_already_pending_newer_epoch_is_observed() {
        // The TOCTOU case: batch 3 is already on the watch when batch 2 (epoch 1) takes the slot.
        let (_tx, rx) = watch::channel(3u64);
        let mut sup = Supersession::new(rx, 1, stamp(0), Duration::from_secs(60));
        assert!(sup.superseded(stamp(1_000)));
    }

    #[test]
    fn a_zero_window_is_born_disarmed_and_never_supersedes() {
        // `supersede_window_seconds = 0` disables supersession: even a newer batch arriving in the
        // same instant the burst began (`elapsed == 0`) must not cancel the turn.
        let (tx, rx) = watch::channel(0u64);
        let mut sup = Supersession::new(rx, 0, stamp(0), Duration::ZERO);
        tx.send(1).unwrap();
        assert!(!sup.superseded(stamp(0)));
        assert!(!sup.superseded(stamp(1_000)));
    }

    #[test]
    fn beyond_the_window_latches_disarmed_permanently() {
        let (tx, rx) = watch::channel(0u64);
        let mut sup = Supersession::new(rx, 0, stamp(0), Duration::from_secs(60));
        // A check past the window disarms the handle, even though no newer batch has arrived yet.
        assert!(!sup.superseded(stamp(61_000)));
        // A newer batch now arrives, and the clock rewinds back inside the window — the handle stays
        // disarmed regardless, so the turn keeps its slot.
        tx.send(1).unwrap();
        assert!(!sup.superseded(stamp(1_000)));
    }
}
