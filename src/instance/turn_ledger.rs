//! The per-conversation turn ledger: the in-epoch-order admission slot and the supersession signal a
//! conversation's turns share (spec §Concurrency → per-conversation supersession).
//!
//! Every inbound batch for a conversation passes through here before it runs. The ledger does two
//! things at once. It **serializes** a conversation's turns in strict arrival order, so two batches
//! for one room never drive the shared session VM concurrently and never run out of order. And it
//! **supersedes** an in-flight turn when a newer batch arrives: each arrival bumps a level-triggered
//! epoch counter and sends it on a [`tokio::sync::watch`] channel, which the generating turn watches
//! at its cooperative boundaries (via the [`Supersession`] handle it was admitted with) to decide
//! whether a newer batch has overtaken it.
//!
//! The epoch counter is level-triggered rather than edge-triggered on purpose. When batch 3 arrives
//! while batch 1 generates and batch 2 waits, the watch simply holds 3: batch 1's boundary check sees
//! `3 > 1` and yields, and when batch 2 finally takes the slot its very first check sees `3 > 2` and
//! yields without generating, so the newest batch answers once with everything in context. No
//! per-target bookkeeping is needed — the winner is always whoever holds the highest epoch.
//!
//! # In-epoch-order admission (the deli counter)
//!
//! Admission is ordered **by construction**, not by an async mutex whose queue order the scheduler
//! decides. Each arrival is stamped with a monotonic epoch under the map lock, and a second
//! [`tokio::sync::watch`] channel — `serving` — carries the single epoch currently entitled to the
//! slot. A ticket admits only when `serving` reaches its own epoch; otherwise it awaits the next
//! change. Because epochs are assigned and served in the same monotonic order, batch *N* always
//! admits, appends its inbound, and exits before batch *N+1* admits — regardless of which task's
//! settings read finished first or how the scheduler ordered them. There is no slot mutex whose
//! acquisition order could invert against epoch order.
//!
//! When a holder exits it advances `serving` past its own epoch, skipping any later epochs that
//! already finished ahead of their turn (a ticket dropped before its serving epoch came, recorded in
//! `finished`). An `outstanding` count tracks live holders — a live ticket or a live admission — and
//! the map entry is removed only when it reaches zero. This is the lifetime rule that makes the bug
//! class impossible: an entry can never be removed while any holder lives, so `next_epoch` never
//! resets under a live holder, no second slot generation is ever created for a conversation with a
//! turn still running, and a completing turn genuinely holds every lower epoch's messages in its
//! buffer (they admitted, appended, and exited before it could admit).
//!
//! The burst window is anchored on the ledger's arrival bookkeeping. Each arrival records its epoch
//! and its timestamp; the burst's origin — the arrival time the supersession window is measured from —
//! is the *oldest unanswered* arrival, `arrivals.first()`. That anchor is snapshotted under the map
//! lock **after** the admitting turn holds the slot, closing the TOCTOU gap: were the burst start read
//! before the slot were held, a batch admitted between the read and the slot acquisition could shift
//! the front out from under the snapshot. Holding the slot first pins the set of arrivals the winner
//! is answering, so the anchor it reads is the one its window is genuinely measured from.
//!
//! The arrival list is maintained by RAII prune rules so it never leaks (see the [`Drop`]
//! implementations below): a turn that runs to completion prunes its own arrival and every older one
//! (they are answered), re-anchoring the burst at the oldest remaining waiter; a turn superseded
//! mid-flight prunes nothing, leaving its unanswered messages to anchor the burst until the batch that
//! overtook it completes; and a ticket dropped before it ever admits (a caller error, a dropped
//! connection) prunes as a completion would. Every exit — completed, superseded, or dropped — still
//! advances `serving` and decrements `outstanding`; only the arrival prune is skipped for a superseded
//! exit. When the last holder leaves, the conversation's entry — its slot, watch channels, and
//! bookkeeping — is dropped from the map.

use std::{
    collections::{BTreeSet, HashMap},
    time::Duration,
};

use parking_lot::Mutex;
use tokio::sync::watch;

use crate::{agent::Supersession, ids::ConversationId, time::Timestamp};

/// The per-conversation admission slots and supersession signals. All map state lives under one
/// `parking_lot::Mutex`, never held across an `.await`: the awaited handles (the `serving` and
/// `supersede` watch receivers) are cloned out under the lock and awaited outside it. Pure runtime
/// state — never logged; an agent restart drops it, and the next batch rebuilds a conversation's entry
/// on first contact.
pub(crate) struct TurnLedger {
    conversations: Mutex<HashMap<ConversationId, ConversationTurns>>,
}

/// One conversation's shared turn state: the deli-counter serving the slot in epoch order, the watch
/// carrying its latest arrival epoch, the next epoch to hand out, the count of live holders, the
/// epochs that exited ahead of their turn, and the arrivals not yet answered by a completed turn.
struct ConversationTurns {
    /// The epoch currently entitled to the slot. A ticket admits when this reaches its own epoch; on
    /// exit a holder advances it past its epoch (and past any consecutive already-`finished` epochs),
    /// so admission proceeds in strict arrival order by construction. Starts at the first epoch handed
    /// out (0 for a fresh entry).
    serving: watch::Sender<u64>,
    /// The conversation's latest arrival epoch. Bumped and sent on every arrival; the in-flight turn's
    /// [`Supersession`] reads the level to detect a newer batch.
    supersede: watch::Sender<u64>,
    /// The next epoch to assign — monotonic per conversation, so a later arrival always outranks an
    /// earlier one and admits strictly after it.
    next_epoch: u64,
    /// Live holders — a ticket not yet admitted or dropped, or an admission not yet dropped. Bumped in
    /// [`arrive`](TurnLedger::arrive), decremented by every exit. The entry is removed only when this
    /// reaches zero, so no holder ever outlives its conversation's slot.
    outstanding: usize,
    /// Epochs whose holder exited *before* `serving` reached them — a ticket dropped while an earlier
    /// epoch still ran. Drained as `serving` advances over the consecutive run at its front, so the
    /// counter never stalls waiting on a holder that has already left. Bounded by `outstanding`.
    finished: BTreeSet<u64>,
    /// Every batch that has arrived and not yet been answered by a completed turn, in arrival order.
    /// The front is the burst's origin; the prune rules keep it exact.
    arrivals: Vec<Arrival>,
}

/// One unanswered arrival: the epoch it was admitted under and when it arrived.
struct Arrival {
    epoch: u64,
    arrived_at: Timestamp,
}

impl TurnLedger {
    /// Construct an empty ledger.
    pub(crate) fn new() -> TurnLedger {
        TurnLedger {
            conversations: Mutex::new(HashMap::new()),
        }
    }

    /// Register a newly arrived batch for `conversation` as of `now`, returning the ticket that admits
    /// it to a turn. Synchronous and immediate: it assigns the batch the next epoch, records the
    /// arrival, counts a new holder, and bumps the supersede watch **before returning** — so an
    /// in-flight turn is signalled the moment the batch lands, not after it has queued for a stream
    /// permit. The returned [`TurnTicket`] must then be [`admit`](TurnTicket::admit)ed to wait for its
    /// turn at the slot.
    pub(crate) fn arrive(&self, conversation: ConversationId, now: Timestamp) -> TurnTicket<'_> {
        let mut map = self.conversations.lock();
        let entry = map
            .entry(conversation)
            .or_insert_with(ConversationTurns::new);
        let epoch = entry.next_epoch;
        entry.next_epoch += 1;
        entry.outstanding += 1;
        entry.arrivals.push(Arrival {
            epoch,
            arrived_at: now,
        });
        // `send_replace` ignores the receiver count (there may be none between turns) and always
        // updates the level, so a later turn subscribing sees the newest epoch.
        entry.supersede.send_replace(epoch);
        let serving_rx = entry.serving.subscribe();
        let rx = entry.supersede.subscribe();
        TurnTicket {
            ledger: self,
            conversation,
            epoch,
            arrived_at: now,
            serving_rx,
            rx,
            defused: false,
        }
    }

    /// The shared exit protocol for every holder — a completed or superseded admission, or a ticket
    /// dropped before it admitted. Run under the map lock. It decrements the live-holder count,
    /// advances the deli counter past this epoch (draining any later epochs that already finished
    /// ahead of their turn), prunes this holder's arrival and every older one unless the turn was
    /// superseded, and removes the conversation's entry once no holder remains.
    fn exit(&self, conversation: ConversationId, epoch: u64, superseded: bool) {
        let mut map = self.conversations.lock();
        let Some(entry) = map.get_mut(&conversation) else {
            return;
        };
        entry.outstanding -= 1;

        // Advance the counter if this holder was the one being served, skipping the consecutive run of
        // epochs that already finished ahead of their turn; otherwise remember that this epoch is done
        // so the counter can skip it when it arrives.
        if *entry.serving.borrow() == epoch {
            let mut next = epoch + 1;
            while entry.finished.remove(&next) {
                next += 1;
            }
            entry.serving.send_replace(next);
        } else {
            entry.finished.insert(epoch);
        }

        // A superseded turn prunes nothing — its unanswered arrival still anchors the burst for the
        // winner. A completed (or errored, panicking, or never-admitted) exit answered or abandoned its
        // batch and every older one, so it prunes them and the burst re-anchors at the oldest waiter.
        if !superseded {
            entry.arrivals.retain(|arrival| arrival.epoch > epoch);
        }

        // Remove the entry only once every holder has exited. `arrivals` may still be non-empty here —
        // a superseded exit left its arrival, or a dropped ticket ahead in line abandoned messages —
        // but with no holder to answer them their burst is over; dropping them is correct, and the
        // passive catch-up contract (a later batch replays the buffer) covers the messages.
        if entry.outstanding == 0 {
            map.remove(&conversation);
        }
    }
}

impl ConversationTurns {
    fn new() -> ConversationTurns {
        let (serving, _serving_rx) = watch::channel(0);
        let (supersede, _rx) = watch::channel(0);
        ConversationTurns {
            serving,
            supersede,
            next_epoch: 0,
            outstanding: 0,
            finished: BTreeSet::new(),
            arrivals: Vec::new(),
        }
    }
}

/// A batch's claim on its conversation's next turn. Holding one means the batch has arrived (its
/// signal already fired) but has not yet taken the serialization slot. [`admit`](Self::admit) awaits
/// this ticket's turn at the deli counter and yields a [`TurnAdmission`]; dropping a ticket without
/// admitting — a caller error or a dropped connection before the turn ran — advances the counter past
/// it and prunes its arrival as a completion would, so a never-run batch neither stalls the line nor
/// anchors the burst forever.
pub(crate) struct TurnTicket<'a> {
    ledger: &'a TurnLedger,
    conversation: ConversationId,
    epoch: u64,
    arrived_at: Timestamp,
    /// The deli counter: awaited in [`admit`](Self::admit) until it reaches this ticket's epoch.
    serving_rx: watch::Receiver<u64>,
    /// The arrival-epoch watch, cloned into the [`Supersession`] handle the admission hands out.
    rx: watch::Receiver<u64>,
    /// Set once ownership has moved into a [`TurnAdmission`], so the ticket's own drop runs no exit —
    /// the admission now owns it. Ensures the exit protocol runs exactly once per holder.
    defused: bool,
}

impl<'a> TurnTicket<'a> {
    /// Await this ticket's turn at the deli counter, then admit its batch to a turn. Admission is in
    /// strict arrival order: the ticket resolves only once the counter serves its epoch, which happens
    /// when every earlier batch has exited. The returned [`TurnAdmission`] entitles the slot for its
    /// whole lifetime, so the next batch's admission waits behind it (the serialization guarantee).
    /// `window` bounds how long the admitted turn stays cancellable — a zero window leaves the turn
    /// uncancellable while keeping serialization on.
    ///
    /// Cancel-safe: dropping the returned future drops the ticket, whose [`Drop`] runs the exit
    /// protocol, so an abandoned admit never stalls the line.
    ///
    /// The burst origin is snapshotted **after** the slot is held, from the oldest arrival still
    /// outstanding: holding the slot first pins the arrival set the turn answers, so the origin the
    /// window is measured from cannot shift underneath the read.
    pub(crate) async fn admit(mut self, window: Duration) -> TurnAdmission<'a> {
        // Wait until the counter serves this ticket's epoch. `borrow_and_update` marks the current
        // value seen so `changed()` only wakes on a genuinely newer value — no lost wakeup, since the
        // entry (and thus the `serving` sender) outlives every holder, so `changed()` never errors.
        loop {
            if *self.serving_rx.borrow_and_update() == self.epoch {
                break;
            }
            self.serving_rx
                .changed()
                .await
                .expect("the serving sender outlives every holder of its conversation");
        }
        let burst_started_at = {
            let map = self.ledger.conversations.lock();
            map.get(&self.conversation)
                .and_then(|entry| entry.arrivals.first())
                .map(|arrival| arrival.arrived_at)
                // The ticket's own arrival is outstanding until it exits, so the front always exists;
                // fall back to this batch's own arrival if it somehow does not.
                .unwrap_or(self.arrived_at)
        };
        self.defused = true;
        TurnAdmission {
            ledger: self.ledger,
            conversation: self.conversation,
            epoch: self.epoch,
            rx: self.rx.clone(),
            burst_started_at,
            window,
            superseded: false,
        }
    }
}

impl Drop for TurnTicket<'_> {
    fn drop(&mut self) {
        // Only fires when the ticket was dropped before `admit` completed; a successful admit defuses
        // it and hands the exit to the `TurnAdmission`. A never-admitted ticket exits like a completion
        // — it advances the counter and prunes its arrival.
        if !self.defused {
            self.ledger.exit(self.conversation, self.epoch, false);
        }
    }
}

/// An admitted turn's hold on its conversation. It is entitled to the serialization slot for its whole
/// lifetime and carries the state a [`Supersession`] handle is built from. On drop it advances the
/// deli counter to the next epoch and re-anchors the burst unless it was superseded: a turn that
/// completed answered its batch and every older one, so it prunes them and the burst re-anchors at the
/// oldest waiter, whereas a superseded turn leaves its arrival in place so the batch that overtook it
/// still measures its window from the original origin.
pub(crate) struct TurnAdmission<'a> {
    ledger: &'a TurnLedger,
    conversation: ConversationId,
    epoch: u64,
    rx: watch::Receiver<u64>,
    burst_started_at: Timestamp,
    window: Duration,
    superseded: bool,
}

impl TurnAdmission<'_> {
    /// Build the cooperative-cancellation handle for this turn: a fresh view of the conversation's
    /// arrival-epoch watch, the epoch this turn was admitted under, and the window it stays cancellable
    /// for. Each call clones the receiver, so a turn threading the handle to several call sites hands
    /// each one its own view of the same signal.
    pub(crate) fn supersession(&self) -> Supersession {
        Supersession::new(
            self.rx.clone(),
            self.epoch,
            self.burst_started_at,
            self.window,
        )
    }

    /// Mark this turn as superseded, so its exit leaves the burst anchor untouched. Called when the
    /// turn returned [`crate::agent::TurnOutcome::Superseded`]: the batch that overtook it owns the
    /// burst now, and its unanswered messages must keep anchoring the window until that batch completes.
    /// The counter still advances and the holder count still decrements on exit — only the arrival prune
    /// is skipped.
    pub(crate) fn mark_superseded(&mut self) {
        self.superseded = true;
    }
}

impl Drop for TurnAdmission<'_> {
    fn drop(&mut self) {
        // A superseded turn prunes nothing — its arrival still anchors the burst for the winner. A
        // completed (or errored, or panicking) turn answered its batch and every older one, so it
        // prunes them and re-anchors the burst at the oldest remaining waiter. Either way the exit
        // advances the deli counter to the next epoch, so the next batch admits.
        self.ledger
            .exit(self.conversation, self.epoch, self.superseded);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tokio::time::timeout;

    fn conversation() -> ConversationId {
        ConversationId::generate()
    }

    fn stamp(millis: i64) -> Timestamp {
        Timestamp::from_millis(millis)
    }

    const WINDOW: Duration = Duration::from_secs(60);

    #[tokio::test]
    async fn an_arrival_bumps_the_supersede_watch() {
        let ledger = TurnLedger::new();
        let convo = conversation();

        let first = ledger.arrive(convo, stamp(0));
        let admission = first.admit(WINDOW).await;
        let mut sup = admission.supersession();
        // The admitting turn is not superseded by its own arrival.
        assert!(!sup.superseded(stamp(0)));

        // A second arrival bumps the watch, and the in-flight turn now observes the newer level.
        let _second = ledger.arrive(convo, stamp(1_000));
        assert!(sup.superseded(stamp(1_000)));
    }

    #[tokio::test]
    async fn a_second_admission_waits_until_the_first_drops() {
        let ledger = TurnLedger::new();
        let convo = conversation();

        let first = ledger.arrive(convo, stamp(0));
        let second = ledger.arrive(convo, stamp(1_000));

        let first_admission = first.admit(WINDOW).await;

        // The second admission cannot complete while the first holds the slot.
        let mut second_fut = std::pin::pin!(second.admit(WINDOW));
        assert!(
            timeout(Duration::from_millis(50), &mut second_fut)
                .await
                .is_err(),
            "the second admission must block on the slot the first holds",
        );

        // Release the first; the second then admits promptly.
        drop(first_admission);
        let second_admission = timeout(Duration::from_secs(1), &mut second_fut)
            .await
            .expect("the second admission proceeds once the slot frees");
        // The first completed, so the burst re-anchors at the second arrival.
        assert_eq!(second_admission.burst_started_at, stamp(1_000));
    }

    #[tokio::test]
    async fn a_superseded_exit_keeps_the_burst_anchor() {
        let ledger = TurnLedger::new();
        let convo = conversation();

        let first = ledger.arrive(convo, stamp(0));
        let second = ledger.arrive(convo, stamp(1_000));

        let mut first_admission = first.admit(WINDOW).await;
        first_admission.mark_superseded();
        drop(first_admission);

        // The superseded turn pruned nothing, so the second turn still measures its window from the
        // burst's original origin — the first, unanswered arrival.
        let second_admission = second.admit(WINDOW).await;
        assert_eq!(second_admission.burst_started_at, stamp(0));
    }

    #[tokio::test]
    async fn a_completed_exit_reanchors_the_burst() {
        let ledger = TurnLedger::new();
        let convo = conversation();

        let first = ledger.arrive(convo, stamp(0));
        let second = ledger.arrive(convo, stamp(1_000));

        // The first completes normally (no supersede mark), pruning its arrival.
        drop(first.admit(WINDOW).await);

        let second_admission = second.admit(WINDOW).await;
        assert_eq!(second_admission.burst_started_at, stamp(1_000));
    }

    #[tokio::test]
    async fn a_ticket_dropped_before_admit_prunes() {
        let ledger = TurnLedger::new();
        let convo = conversation();

        let first = ledger.arrive(convo, stamp(0));
        let second = ledger.arrive(convo, stamp(1_000));
        // Drop the first ticket without admitting — a caller error or a dropped connection.
        drop(first);

        // The second turn re-anchors at its own arrival: the abandoned first no longer anchors, and the
        // counter advanced past the dropped epoch so the second admits at all.
        let second_admission = second.admit(WINDOW).await;
        assert_eq!(second_admission.burst_started_at, stamp(1_000));
    }

    #[tokio::test]
    async fn the_entry_is_removed_once_idle() {
        let ledger = TurnLedger::new();
        let convo = conversation();

        let ticket = ledger.arrive(convo, stamp(0));
        assert!(ledger.conversations.lock().contains_key(&convo));
        drop(ticket.admit(WINDOW).await);

        // With no holders outstanding, the conversation's entry is gone from the map.
        assert!(!ledger.conversations.lock().contains_key(&convo));
    }

    #[tokio::test]
    async fn admission_follows_arrival_order_even_when_the_later_batch_admits_first() {
        let ledger = TurnLedger::new();
        let convo = conversation();

        // A arrives before B, so A holds the earlier epoch.
        let a = ledger.arrive(convo, stamp(0));
        let b = ledger.arrive(convo, stamp(1_000));

        // B tries to admit first — the very inversion the old slot mutex allowed. With the deli
        // counter it must NOT resolve while A (the earlier epoch) has not exited.
        let mut b_fut = std::pin::pin!(b.admit(WINDOW));
        assert!(
            timeout(Duration::from_millis(50), &mut b_fut)
                .await
                .is_err(),
            "B must not admit ahead of the earlier-epoch A, whichever task reached admit first",
        );

        // A admits (it is served first), runs, and exits.
        let a_admission = a.admit(WINDOW).await;
        drop(a_admission);

        // Only now does B admit — strictly after A, by construction.
        let b_admission = timeout(Duration::from_secs(1), &mut b_fut)
            .await
            .expect("B admits once A has exited");
        // A completed, so the burst re-anchored at B's arrival.
        assert_eq!(b_admission.burst_started_at, stamp(1_000));
    }

    #[tokio::test]
    async fn a_dropped_ticket_ahead_in_line_is_skipped() {
        let ledger = TurnLedger::new();
        let convo = conversation();

        let a = ledger.arrive(convo, stamp(0));
        let b = ledger.arrive(convo, stamp(1_000));

        // A holds the served epoch but is dropped without ever admitting. The counter must advance past
        // it so B is not stuck waiting on a holder that has left.
        drop(a);

        let b_admission = timeout(Duration::from_secs(1), b.admit(WINDOW))
            .await
            .expect("B admits promptly once the dropped A advances the counter");
        assert_eq!(b_admission.burst_started_at, stamp(1_000));
    }

    #[tokio::test]
    async fn a_dropped_ticket_behind_the_holder_is_skipped_on_advance() {
        let ledger = TurnLedger::new();
        let convo = conversation();

        let a = ledger.arrive(convo, stamp(0));
        let b = ledger.arrive(convo, stamp(1_000));
        let c = ledger.arrive(convo, stamp(2_000));

        // A is served and admits.
        let a_admission = a.admit(WINDOW).await;

        // C drops before its turn — it lands in `finished`, ahead of the counter.
        drop(c);

        // A exits; the counter advances to B (C is behind it, not consecutive yet).
        drop(a_admission);
        let b_admission = timeout(Duration::from_secs(1), b.admit(WINDOW))
            .await
            .expect("B admits once A exits");

        // B exits; the counter would advance to C's epoch, but C already finished, so it is skipped and
        // the last holder's exit removes the entry.
        drop(b_admission);
        assert!(
            !ledger.conversations.lock().contains_key(&convo),
            "with C already finished and B exited, no holder remains and the entry is removed",
        );
    }

    #[tokio::test]
    async fn the_entry_survives_until_the_last_holder_exits() {
        let ledger = TurnLedger::new();
        let convo = conversation();

        let a = ledger.arrive(convo, stamp(0));
        let b = ledger.arrive(convo, stamp(1_000));

        // A admits, completes, and exits — but B is still outstanding, so the entry must survive.
        drop(a.admit(WINDOW).await);
        assert!(
            ledger.conversations.lock().contains_key(&convo),
            "the entry must outlive A while B still holds a ticket",
        );

        // B admits against the SAME entry — same supersede watch, so a subsequent arrival is observed.
        let b_admission = b.admit(WINDOW).await;
        let mut b_sup = b_admission.supersession();
        assert!(!b_sup.superseded(stamp(1_000)));

        // A fresh arrival C bumps the shared watch B is subscribed to.
        let c = ledger.arrive(convo, stamp(2_000));
        assert!(
            b_sup.superseded(stamp(2_000)),
            "B's supersession sees C's arrival on the same watch it was admitted with",
        );

        // The entry is removed only once every holder — B and C — has exited.
        drop(b_admission);
        assert!(
            ledger.conversations.lock().contains_key(&convo),
            "C is still outstanding, so the entry survives B's exit",
        );
        drop(c);
        assert!(
            !ledger.conversations.lock().contains_key(&convo),
            "the last holder's exit removes the entry",
        );
    }
}
