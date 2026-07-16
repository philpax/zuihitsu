//! Turn-over-background priority at the shared model client.
//!
//! A participant's live turn and the off-hot-path synthesis passes (the describer, adjudicator,
//! link-inference pass, and the idle and checkpoint flush sweeps) share one [`ModelClient`]. Without
//! arbitration they contend on equal footing, so a busy session's describe backlog can crowd a
//! waiting conversation turn at the shared model. The [`ModelArbiter`] restores the intended
//! ordering: a waiting turn dispatches ahead of queued background work, and a background pass yields
//! the model to any pending turn.
//!
//! The arbiter hands out two typed handles over the same underlying client — [`ModelArbiter::turn`]
//! and [`ModelArbiter::background`] — both erased to `Arc<dyn ModelClient>`. The priority a call
//! carries is a property of which handle produced it, not an argument, so a background caller holding
//! a background handle cannot dispatch at turn priority by mistake. The serving host threads the turn
//! handle to the conversation path and a background handle to each worker.
//!
//! ## Invariants
//!
//! - **Ordering, not preemption.** An in-flight `generate` always runs to completion; the arbiter
//!   never cancels one. Priority governs only *dispatch* — whether a background call sends now or
//!   waits — so a background pass already talking to the model is never interrupted by a turn that
//!   arrives mid-call.
//! - **Priority is scoped to each model call, not the whole turn.** `turns_pending` counts the
//!   turn-priority `generate` calls that are queued or in flight, incremented for the span of one
//!   call and released when it returns. A turn doing Lua work between model calls holds no marker, so
//!   background work runs in that gap rather than waiting on a turn that is not actually at the model.
//!   This is also what keeps the arbiter composable with the `max_concurrent_streams` semaphore: a
//!   turn holding a stream permit but between model calls does not starve background.
//! - **Background yields to pending turns.** A background `generate` waits while `turns_pending` is
//!   non-zero, then dispatches; once dispatched it holds the model until it returns.
//! - **A dropped or errored turn releases its place.** The pending count is held by an RAII guard, so
//!   an early return, an error, or a cancelled turn future decrements it (and wakes waiters) on drop —
//!   background can never wedge behind a turn that went away.
//! - **Turns are transparent to each other.** The arbiter does not serialise turns; concurrent turns
//!   dispatch together (bounded only by the stream semaphore and the backend's own batching), so
//!   ordering among turns is FIFO exactly as the semaphore and backend deliver it.

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use async_trait::async_trait;
use tokio::sync::Notify;

use crate::{
    metrics::observe_background_model_deferral,
    model::{GenerateDelta, GenerateRequest, GenerateStream, ModelClient, ModelError},
};

/// Arbitrates one shared [`ModelClient`] between conversation turns and background passes, giving a
/// waiting turn priority over queued background work. Wrap the real client once at serving
/// construction, then hand [`turn`](ModelArbiter::turn) to the conversation path and
/// [`background`](ModelArbiter::background) to each worker. See the module docs for the invariants.
pub struct ModelArbiter {
    /// The underlying client every handle delegates to. The arbiter adds ordering above it and never
    /// changes a request or response.
    inner: Arc<dyn ModelClient>,
    /// The number of turn-priority `generate` calls currently queued or in flight. A background call
    /// waits while this is non-zero; it drops back to zero only when no turn is at the model.
    turns_pending: AtomicUsize,
    /// Notified when `turns_pending` falls to zero, waking the background calls waiting to dispatch.
    idle: Notify,
}

impl ModelArbiter {
    /// Wrap `inner` in a fresh arbiter with no turns pending. Returns an `Arc` so the two handles can
    /// share it.
    pub fn new(inner: Arc<dyn ModelClient>) -> Arc<ModelArbiter> {
        Arc::new(ModelArbiter {
            inner,
            turns_pending: AtomicUsize::new(0),
            idle: Notify::new(),
        })
    }

    /// A turn-priority handle: its `generate` marks a turn pending for the span of the call, so any
    /// background call yields to it. Cheap to mint (one `Arc` clone), so a caller may hold one per
    /// conversation.
    pub fn turn(self: &Arc<Self>) -> Arc<dyn ModelClient> {
        Arc::new(TurnModel {
            arbiter: self.clone(),
        })
    }

    /// A background handle: its `generate` waits until no turn is pending before dispatching. This is
    /// the only handle a background worker is given, so it cannot dispatch at turn priority by
    /// mistake.
    pub fn background(self: &Arc<Self>) -> Arc<dyn ModelClient> {
        Arc::new(BackgroundModel {
            arbiter: self.clone(),
        })
    }

    /// Register a turn as pending and return the guard that releases it on drop.
    fn enter_turn(self: &Arc<Self>) -> TurnGuard {
        self.turns_pending.fetch_add(1, Ordering::AcqRel);
        TurnGuard {
            arbiter: self.clone(),
        }
    }

    /// Wait until no turn is pending. Registers for the wake **before** re-reading the count, so a
    /// turn draining between the check and the await is not a lost wakeup; the first iteration that
    /// actually waits records a deferral for observability.
    async fn yield_to_turns(&self) {
        let mut recorded = false;
        while self.turns_pending.load(Ordering::Acquire) > 0 {
            let notified = self.idle.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if self.turns_pending.load(Ordering::Acquire) == 0 {
                break;
            }
            if !recorded {
                recorded = true;
                observe_background_model_deferral();
            }
            notified.await;
        }
    }
}

/// The RAII marker a pending turn holds: decrements `turns_pending` on drop and, when it was the last
/// one, wakes the background calls waiting on `idle`. Dropping on an early return, an error, or a
/// cancelled future is what guarantees background never wedges behind a turn that went away.
struct TurnGuard {
    arbiter: Arc<ModelArbiter>,
}

impl Drop for TurnGuard {
    fn drop(&mut self) {
        if self.arbiter.turns_pending.fetch_sub(1, Ordering::AcqRel) == 1 {
            self.arbiter.idle.notify_waiters();
        }
    }
}

/// A turn-priority handle over the shared client (see [`ModelArbiter::turn`]).
struct TurnModel {
    arbiter: Arc<ModelArbiter>,
}

#[async_trait]
impl ModelClient for TurnModel {
    fn model_id(&self) -> &str {
        self.arbiter.inner.model_id()
    }

    /// The turn marker covers the stream's whole life, not just its creation: the guard moves into
    /// the yielded stream and drops when the stream does, so background yields until the turn's
    /// generation has fully finished (or the turn dropped it).
    async fn generate_stream(&self, request: &GenerateRequest) -> GenerateStream {
        let turn = self.arbiter.enter_turn();
        let inner = self.arbiter.inner.generate_stream(request).await;
        Box::pin(HoldWhileStreaming {
            inner,
            _guard: turn,
        })
    }
}

/// A background handle over the shared client (see [`ModelArbiter::background`]).
struct BackgroundModel {
    arbiter: Arc<ModelArbiter>,
}

#[async_trait]
impl ModelClient for BackgroundModel {
    fn model_id(&self) -> &str {
        self.arbiter.inner.model_id()
    }

    /// Background streaming yields before dispatch exactly as the unary call does; once dispatched
    /// the stream runs to completion (ordering, not preemption).
    async fn generate_stream(&self, request: &GenerateRequest) -> GenerateStream {
        self.arbiter.yield_to_turns().await;
        self.arbiter.inner.generate_stream(request).await
    }
}

/// A stream that keeps a [`TurnGuard`] alive for as long as it does, so the pending-turn marker
/// spans every delta of a streamed turn generation.
struct HoldWhileStreaming {
    inner: GenerateStream,
    _guard: TurnGuard,
}

impl futures_util::Stream for HoldWhileStreaming {
    type Item = Result<GenerateDelta, ModelError>;

    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        self.inner.as_mut().poll_next(cx)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use tokio::sync::{Notify, Semaphore, mpsc};

    use super::{GenerateStream, ModelArbiter};
    use crate::model::{
        Completion, GenerateRequest, GenerateResponse, ModelClient, ScriptedModel, Usage,
    };

    /// A fake whose every `generate` blocks on a shared gate until the test releases a permit, then
    /// reports *which* call completed — the request's `system` field carries the caller's tag, so the
    /// completion is bound to the actual call (a turn call versus a background call) rather than to
    /// the order permits happened to be granted. A test holds the model busy, admits calls one at a
    /// time with [`release`](GatedModel::release), and reads the completion order off the channel.
    struct GatedModel {
        gate: Arc<Semaphore>,
        completed: mpsc::UnboundedSender<String>,
    }

    impl GatedModel {
        fn new() -> (Arc<GatedModel>, mpsc::UnboundedReceiver<String>) {
            let (tx, rx) = mpsc::unbounded_channel();
            (
                Arc::new(GatedModel {
                    gate: Arc::new(Semaphore::new(0)),
                    completed: tx,
                }),
                rx,
            )
        }

        /// Admit one blocked (or next-arriving) `generate` to complete.
        fn release(&self) {
            self.gate.add_permits(1);
        }
    }

    #[async_trait::async_trait]
    impl ModelClient for GatedModel {
        fn model_id(&self) -> &str {
            "gated-model"
        }

        async fn generate_stream(&self, request: &GenerateRequest) -> GenerateStream {
            let permit = self.gate.acquire().await.expect("the gate is never closed");
            permit.forget();
            let _ = self.completed.send(request.system.clone());
            super::super::stream_response(Ok(GenerateResponse {
                completion: Completion::Reply("ok".to_owned()),
                usage: Usage::default(),
                reasoning: None,
                finish_reason: None,
            }))
        }
    }

    /// The property `HoldWhileStreaming` exists for: a turn's stream holds the pending-turn marker
    /// from creation until the stream drops, so background yields the whole time the turn's
    /// generation is open — even while the caller has not yet polled it.
    #[tokio::test]
    async fn a_turn_stream_held_open_blocks_background_until_dropped() {
        let inner = Arc::new(ScriptedModel::new([
            Completion::Reply("turn".to_owned()),
            Completion::Reply("background".to_owned()),
        ]));
        let arbiter = ModelArbiter::new(inner);
        // Create (and hold, undrained) the turn's stream: the marker arms here.
        let turn_model = arbiter.turn();
        let turn_stream = turn_model.generate_stream(&req("turn")).await;

        let background_model = arbiter.background();
        let background =
            tokio::spawn(async move { background_model.generate(&req("background")).await });
        // Give background every chance to (incorrectly) dispatch.
        for _ in 0..20 {
            tokio::task::yield_now().await;
        }
        assert!(
            !background.is_finished(),
            "background dispatched while a turn stream was open"
        );

        drop(turn_stream);
        let response = background.await.expect("joins").expect("generates");
        assert!(matches!(
            response.completion,
            Completion::Reply(reply) if reply == "background"
        ));
    }

    /// A request tagged so the gated model reports it by name on completion.
    fn req(tag: &str) -> GenerateRequest {
        GenerateRequest {
            system: tag.to_owned(),
            ..GenerateRequest::default()
        }
    }

    /// Yield the runtime enough times for spawned tasks to reach their next suspension point (the
    /// gate or the arbiter's wait). One `yield_now` is not always enough when a task must traverse
    /// several awaits, so drain a handful.
    async fn settle() {
        for _ in 0..16 {
            tokio::task::yield_now().await;
        }
    }

    #[tokio::test]
    async fn a_background_call_queued_while_a_turn_waits_dispatches_after_the_turn() {
        let (model, mut done) = GatedModel::new();
        let arbiter = ModelArbiter::new(model.clone());
        let turn = arbiter.turn();
        let background = arbiter.background();

        // The turn is submitted first and waits at the gate (holding the model); the background call
        // is submitted while it waits.
        let t = tokio::spawn(async move { turn.generate(&req("turn")).await });
        settle().await;
        let b = tokio::spawn(async move { background.generate(&req("background")).await });
        settle().await;

        // One permit admits the turn (the background call is still yielding, not at the gate), so the
        // turn completes first.
        model.release();
        assert_eq!(done.recv().await.as_deref(), Some("turn"));
        t.await.unwrap().unwrap();
        // With the turn drained, the background call proceeds on the next permit.
        model.release();
        assert_eq!(done.recv().await.as_deref(), Some("background"));
        b.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn a_turn_arriving_while_background_is_in_flight_waits_only_for_that_call() {
        let (model, mut done) = GatedModel::new();
        let arbiter = ModelArbiter::new(model.clone());
        let background = arbiter.background();
        let background2 = arbiter.background();
        let turn = arbiter.turn();

        // Background call 1 is in flight (at the gate). A second background call and a turn arrive
        // behind it.
        let b1 = tokio::spawn(async move { background.generate(&req("bg1")).await });
        settle().await;
        let t = tokio::spawn(async move { turn.generate(&req("turn")).await });
        settle().await;
        let b2 = tokio::spawn(async move { background2.generate(&req("bg2")).await });
        settle().await;

        // Release bg1: it completes. The turn is now queued at the gate (it does not yield); bg2 is
        // still yielding because a turn is pending.
        model.release();
        assert_eq!(done.recv().await.as_deref(), Some("bg1"));
        b1.await.unwrap().unwrap();

        // The next permit admits the turn ahead of the queued background call.
        model.release();
        assert_eq!(done.recv().await.as_deref(), Some("turn"));
        t.await.unwrap().unwrap();

        // Only once the turn drains does bg2 dispatch.
        model.release();
        assert_eq!(done.recv().await.as_deref(), Some("bg2"));
        b2.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn background_proceeds_when_no_turn_is_waiting() {
        let (model, mut done) = GatedModel::new();
        let arbiter = ModelArbiter::new(model.clone());
        let background = arbiter.background();

        let b = tokio::spawn(async move { background.generate(&req("background")).await });
        settle().await;
        // No turn ever registers, so the background call is already at the gate; one permit completes
        // it with no needless serialisation.
        model.release();
        assert_eq!(done.recv().await.as_deref(), Some("background"));
        b.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn multiple_turns_keep_their_order_among_themselves() {
        // The arbiter is transparent to turns — it does not reorder them — so turns queued at a
        // single-slot model dispatch in the order they arrived (the model's own fair queue).
        let (model, mut done) = GatedModel::new();
        let arbiter = ModelArbiter::new(model.clone());

        let mut handles = Vec::new();
        for tag in ["t1", "t2", "t3"] {
            let turn = arbiter.turn();
            let request = req(tag);
            handles.push(tokio::spawn(async move { turn.generate(&request).await }));
            settle().await;
        }
        for expected in ["t1", "t2", "t3"] {
            model.release();
            assert_eq!(done.recv().await.as_deref(), Some(expected));
        }
        for handle in handles {
            handle.await.unwrap().unwrap();
        }
    }

    #[tokio::test]
    async fn a_dropped_turn_releases_its_place() {
        let (model, mut done) = GatedModel::new();
        let arbiter = ModelArbiter::new(model.clone());
        let background = arbiter.background();

        // A turn registers and then its future is dropped before ever completing (a cancelled turn).
        // The `enter_turn` guard must decrement on drop, or the background call wedges forever.
        let turn = arbiter.turn();
        let started = Arc::new(Notify::new());
        let started2 = started.clone();
        let t = tokio::spawn(async move {
            let request = req("turn");
            let fut = turn.generate(&request);
            started2.notify_one();
            fut.await
        });
        started.notified().await;
        settle().await;
        t.abort();
        let _ = t.await;
        settle().await;

        // With the turn gone, the background call must proceed.
        let b = tokio::spawn(async move { background.generate(&req("background")).await });
        settle().await;
        model.release();
        assert_eq!(done.recv().await.as_deref(), Some("background"));
        b.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn interleaved_turns_and_background_complete_without_deadlock() {
        // A stress interleave: many turns and background calls in flight at once against a
        // single-slot model. Every call must complete (no deadlock), and no background call may
        // complete while a turn is still queued and unserved.
        let (model, mut done) = GatedModel::new();
        let arbiter = ModelArbiter::new(model.clone());

        let turns = 12;
        let backgrounds = 12;
        let mut handles = Vec::new();

        // Register every turn first and settle, so all turns are queued at the gate (and pending)
        // before any background call runs its yield — otherwise a background scheduled ahead of the
        // turns could slip through on an empty pending count, which is correct behaviour but not the
        // contention this test means to exercise.
        for _ in 0..turns {
            let turn = arbiter.turn();
            handles.push(tokio::spawn(
                async move { turn.generate(&req("turn")).await },
            ));
        }
        settle().await;
        for _ in 0..backgrounds {
            let background = arbiter.background();
            handles.push(tokio::spawn(async move {
                background.generate(&req("bg")).await
            }));
        }
        settle().await;

        // Drain the model one slot at a time: no background call may complete until every turn has,
        // because a background yields while any turn is pending.
        let mut turns_done = 0;
        let mut backgrounds_done = 0;
        for _ in 0..(turns + backgrounds) {
            model.release();
            let tag = done.recv().await.expect("a call completes per permit");
            match tag.as_str() {
                "turn" => {
                    assert_eq!(
                        backgrounds_done, 0,
                        "a background call completed while turns were still queued"
                    );
                    turns_done += 1;
                }
                "bg" => backgrounds_done += 1,
                other => panic!("unexpected tag {other}"),
            }
        }
        assert_eq!(turns_done, turns);
        assert_eq!(backgrounds_done, backgrounds);
        for handle in handles {
            handle.await.unwrap().unwrap();
        }
    }
}
