//! The step interpreter. [`execute`] drives a scenario's [`EvalStep`] script against a booted
//! [`RunContext`] and journals each step's event-log coverage into a [`StepRecord`]: the span of event
//! seqs the step appended and the log head after it. That journal is the recorded run's
//! scenario↔log correspondence — it lets a later phase restore the store up to a chosen step's
//! watermark and re-execute the rest, or render a run's events grouped by the step that produced them.

use zuihitsu::{Event, EventPayload, MemoryId, PersonId, Seq, TurnRole};

use crate::{
    context::{BurstDelivery, RunContext},
    error::EvalError,
    step::{EvalStep, OnMissing, StepText},
};

pub use zuihitsu_frontend_types::StepRecord;

/// Drive `steps` against `ctx`, journaling each step's event-log coverage. For each step the executor
/// notes the log head, performs the operation through the existing [`RunContext`] methods, then reads
/// the tail appended past the prior head — an incremental read, so a step's cost is its own events, not
/// a re-fold of the whole log.
pub async fn execute(steps: &[EvalStep], ctx: &RunContext) -> Result<Vec<StepRecord>, EvalError> {
    execute_from(steps, ctx, 0).await
}

/// As [`execute`], but the produced [`StepRecord`] indices start at `start_index` rather than zero —
/// the resume path drives `steps[K+1..]` against a restored context and numbers the continuation's
/// records from `K + 1`, so the merged journal's step indices stay contiguous with the recorded prefix.
/// The watermark still initializes to the context's current log head, which for a restored context is
/// the prefix's watermark, so the first continued step's span begins just past it.
pub async fn execute_from(
    steps: &[EvalStep],
    ctx: &RunContext,
    start_index: u32,
) -> Result<Vec<StepRecord>, EvalError> {
    let mut records = Vec::with_capacity(steps.len());
    // The last seq observed, initialized to the log head after genesis (or, on resume, the restored
    // prefix's head): the birth events predate the first step, so they sit outside the journal rather
    // than being attributed to step zero. `watermark.next()` reads the events strictly after it (the
    // first step's read starts just past that head).
    let mut watermark = head_seq(ctx)?;
    for (offset, step) in steps.iter().enumerate() {
        let index = start_index + offset as u32;
        let skipped = perform(step, ctx).await?;
        let appended = ctx.events_from(watermark.next())?;
        let first_seq = appended.first().map(|event| event.seq);
        let last_seq = appended.last().map(|event| event.seq);
        if let Some(seq) = last_seq {
            watermark = seq;
        }
        records.push(StepRecord {
            index,
            step: step.clone(),
            first_seq,
            last_seq,
            seq_after: watermark,
            skipped,
        });
    }
    Ok(records)
}

/// Perform one step against the agent, returning whether it was skipped (a no-op because a run-time
/// precondition was absent). Every step but a skipped [`EvalStep::ConfirmProposedMerge`] returns
/// `false`.
async fn perform(step: &EvalStep, ctx: &RunContext) -> Result<bool, EvalError> {
    match step {
        EvalStep::Turn(turn) => {
            let text = resolve_text(&turn.text, ctx)?;
            let sender = PersonId::new(&turn.platform, &turn.sender);
            let present: Vec<PersonId> = turn
                .present
                .iter()
                .map(|uid| PersonId::new(&turn.platform, uid))
                .collect();
            ctx.turn(&turn.platform, &turn.scope, &sender, &text, &present)
                .await?;
        }
        EvalStep::InterruptedTurn(burst) => {
            let first_text = resolve_text(&burst.first.text, ctx)?;
            let interrupt_text = resolve_text(&burst.interrupt.text, ctx)?;
            let first_sender = PersonId::new(&burst.platform, &burst.first.sender);
            let interrupt_sender = PersonId::new(&burst.platform, &burst.interrupt.sender);
            let present: Vec<PersonId> = burst
                .present
                .iter()
                .map(|uid| PersonId::new(&burst.platform, uid))
                .collect();
            let (first_outcome, interrupt_outcome) = ctx
                .interrupted_turn(
                    &burst.platform,
                    &burst.scope,
                    BurstDelivery {
                        sender: &first_sender,
                        text: &first_text,
                    },
                    BurstDelivery {
                        sender: &interrupt_sender,
                        text: &interrupt_text,
                    },
                    &present,
                )
                .await?;
            tracing::debug!(
                ?first_outcome,
                ?interrupt_outcome,
                "interrupted-turn burst completed"
            );
        }
        EvalStep::Imprint { text } => {
            ctx.imprint(text).await?;
        }
        EvalStep::Settle => ctx.settle().await?,
        EvalStep::Advance { millis } => ctx.advance(*millis),
        EvalStep::AdvancePastIdleGap => ctx.advance_past_idle_gap(),
        EvalStep::DescribeCatchUp => ctx.describe_catch_up().await?,
        EvalStep::LinkInferenceCatchUp => ctx.link_inference_catch_up().await?,
        EvalStep::MaintenanceCatchUp => ctx.maintenance_catch_up().await?,
        EvalStep::CheckpointSweep => {
            ctx.checkpoint_sweep().await?;
        }
        EvalStep::SeedEvents(events) => ctx.seed_events(events.clone())?,
        EvalStep::TuneSupersession { window_seconds } => ctx.tune_supersession(*window_seconds)?,
        EvalStep::TightenCompaction {
            token_budget,
            flush_min_turns,
        } => ctx.tighten_compaction(*token_budget, *flush_min_turns)?,
        EvalStep::ForceCompaction { platform, scope } => {
            ctx.force_compaction(platform, scope).await?;
        }
        EvalStep::TuneCheckpoint {
            min_delta_chars,
            cooldown_seconds,
            flush_on_open,
        } => ctx.tune_checkpoint(*min_delta_chars, *cooldown_seconds, *flush_on_open)?,
        EvalStep::ConfirmProposedMerge { on_missing } => {
            return confirm_proposed_merge(*on_missing, ctx);
        }
    }
    Ok(false)
}

/// Confirm the first merge proposed in the live log, resolving the proposed pair at execution time.
/// A proposal present is confirmed as an operator `same_as` merge; a proposal absent defers to
/// `on_missing` — [`OnMissing::Skip`] records the step skipped and continues (the no-proposal case a
/// hearsay scenario deliberately measures), [`OnMissing::Fail`] errors the run.
fn confirm_proposed_merge(on_missing: OnMissing, ctx: &RunContext) -> Result<bool, EvalError> {
    match proposed_merge(&ctx.events()?) {
        Some((from, to)) => {
            ctx.operator_merge(from, to)?;
            Ok(false)
        }
        None => match on_missing {
            OnMissing::Skip => Ok(true),
            OnMissing::Fail => Err(EvalError::Executor(
                "ConfirmProposedMerge found no merge proposal in the log".to_owned(),
            )),
        },
    }
}

/// Resolve a step's text against the live log. A [`StepText::WithTurnRef`] substitutes the `{turn}`
/// marker with the `[turn:<id>]` token of the first participant turn whose text is exactly `of_turn`;
/// an unresolvable reference is a clear executor error (the scenario referenced a moment its own
/// script never recorded).
fn resolve_text(text: &StepText, ctx: &RunContext) -> Result<String, EvalError> {
    match text {
        StepText::Literal(literal) => Ok(literal.clone()),
        StepText::WithTurnRef { template, of_turn } => {
            let turn_id = first_participant_turn_id(&ctx.events()?, of_turn).ok_or_else(|| {
                EvalError::Executor(format!(
                    "no participant turn whose text is {of_turn:?} to resolve a [turn:<id>] \
                     reference"
                ))
            })?;
            Ok(template.replace("{turn}", &format!("[turn:{turn_id}]")))
        }
    }
}

/// The log's current head seq — the seq of the last event recorded, or `Seq::ZERO` for an empty log.
fn head_seq(ctx: &RunContext) -> Result<Seq, EvalError> {
    Ok(ctx
        .events()?
        .last()
        .map(|event| event.seq)
        .unwrap_or(Seq::ZERO))
}

/// The `(from, to)` of the first merge proposed in the log, if any — the pair the operator confirms.
fn proposed_merge(events: &[Event]) -> Option<(MemoryId, MemoryId)> {
    events.iter().find_map(|event| match &event.payload {
        EventPayload::MergeProposed { from, to, .. } => Some((*from, *to)),
        _ => None,
    })
}

/// The id of the first participant `ConversationTurn` whose text is `text` — the earlier moment a later
/// reference points back to. Read from the run's own log so the scenario references the exact turn id
/// the agent will resolve, rather than a fabricated one.
fn first_participant_turn_id(events: &[Event], text: &str) -> Option<String> {
    events.iter().find_map(|event| match &event.payload {
        EventPayload::ConversationTurn {
            turn_id,
            role: TurnRole::Participant,
            text: turn_text,
            ..
        } if turn_text == text => Some(turn_id.0.to_string()),
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use zuihitsu::{
        Completion, EventPayload, InstanceFeatures, MemoryId, MemoryName, ScriptedModel,
        TEST_PLATFORM, TurnRole,
    };

    use super::{execute, head_seq};
    use crate::{
        analysis,
        context::{RunContext, RunDeps},
        error::EvalError,
        step::{BurstMessage, EvalStep, InterruptedTurn, OnMissing, StepText, Turn},
    };

    /// Boot a fresh, retrieval-free agent whose turns reply from `model`. The in-memory backends make
    /// the run a pure function of the log, exactly as the harness drives it.
    async fn booted(model: ScriptedModel) -> RunContext {
        let deps = RunDeps {
            model: Arc::new(model),
            embedder: None,
            dimensions: 0,
            web: crate::fetch_fixture::web_fetcher(),
        };
        RunContext::new(
            &deps,
            InstanceFeatures::default(),
            &crate::context::default_seed(),
        )
        .await
        .expect("a fresh agent boots")
    }

    /// A lone `MemoryContentAppended`-free write the executor can attribute a single seq to.
    fn one_memory(name: &str) -> Vec<EventPayload> {
        vec![EventPayload::memory_created(
            MemoryId::generate(),
            MemoryName::new(name),
        )]
    }

    #[tokio::test]
    async fn the_journal_tiles_the_steps_events_with_monotone_watermarks() {
        let ctx = booted(ScriptedModel::new([
            Completion::Reply("Recorded.".to_owned()),
            Completion::Reply("Recorded that too.".to_owned()),
        ]))
        .await;
        // The genesis events predate the first step, so they sit below the journal.
        let genesis_head = head_seq(&ctx).expect("a genesis head");
        let steps = vec![
            Turn::new(TEST_PLATFORM, "team", "dave", "A first fact to keep.").into(),
            Turn::new(TEST_PLATFORM, "team", "erin", "A second, unrelated fact.").into(),
            EvalStep::Advance { millis: 1_000 },
            EvalStep::SeedEvents(one_memory("person/extra")),
        ];
        let journal = execute(&steps, &ctx).await.expect("the steps execute");
        assert_eq!(journal.len(), 4);

        // Nothing was skipped, and the watermark never decreases across the journal.
        assert!(journal.iter().all(|record| !record.skipped));
        assert!(
            journal
                .windows(2)
                .all(|pair| pair[0].seq_after <= pair[1].seq_after)
        );

        // The advance appended no events: an empty span, its watermark carried from the prior step.
        let advance = &journal[2];
        assert!(advance.first_seq.is_none() && advance.last_seq.is_none());
        assert_eq!(advance.seq_after, journal[1].seq_after);

        // Every event a step appended is covered by exactly one span, contiguous from just past the
        // genesis head to the final watermark, with no gaps or overlaps.
        let mut covered: Vec<u64> = Vec::new();
        for record in &journal {
            if let (Some(first), Some(last)) = (record.first_seq, record.last_seq) {
                assert!(first <= last);
                covered.extend(first.0..=last.0);
            }
        }
        let mut deduped = covered.clone();
        deduped.sort_unstable();
        deduped.dedup();
        assert_eq!(deduped.len(), covered.len(), "no seq is covered twice");
        let final_head = head_seq(&ctx).expect("a final head");
        let expected: Vec<u64> = ((genesis_head.0 + 1)..=final_head.0).collect();
        assert_eq!(deduped, expected, "the spans tile the steps' events");

        // The last watermark is the log head.
        assert_eq!(journal.last().unwrap().seq_after, final_head);
    }

    #[tokio::test]
    async fn an_interrupted_turn_burst_journals_one_tiled_span() {
        // The two burst texts, held so the participant-turn assertions match exactly what was delivered.
        const FIRST: &str = "Pull together everything we've settled for Saturday.";
        const CORRECTION: &str = "Scratch the 7pm — moved the booking to 8:30.";

        // ScriptedModel replies instantly, so turn A usually wins the phase-1 race and completes before
        // the interrupt is delivered — the completed-early path. Two completions: one per turn.
        let ctx = booted(ScriptedModel::new([
            Completion::Reply("Here is the summary.".to_owned()),
            Completion::Reply("Updated: the table is now 8:30.".to_owned()),
        ]))
        .await;
        let genesis_head = head_seq(&ctx).expect("a genesis head");

        let steps = vec![
            EvalStep::TuneSupersession {
                window_seconds: 600,
            },
            InterruptedTurn::new(
                TEST_PLATFORM,
                "dinner",
                BurstMessage::new("noor", FIRST),
                BurstMessage::new("noor", CORRECTION),
            )
            .into(),
        ];
        let journal = execute(&steps, &ctx).await.expect("the burst executes");
        assert_eq!(journal.len(), 2);

        // The burst is one journal entry: the whole concurrent burst — both turns' interleaved events —
        // reads as a single serial step, ending at the log head.
        let burst = &journal[1];
        let (first, last) = (
            burst.first_seq.expect("the burst appended events"),
            burst.last_seq.expect("the burst appended events"),
        );
        assert!(first <= last);
        let final_head = head_seq(&ctx).expect("a final head");
        assert_eq!(last, final_head, "the burst span reaches the log head");
        assert_eq!(burst.seq_after, final_head);

        // The two steps' spans tile contiguously from just past genesis to the head, with no gap or
        // overlap: the tune step's settings-change event, then the burst's events.
        let mut covered: Vec<u64> = Vec::new();
        for record in &journal {
            if let (Some(f), Some(l)) = (record.first_seq, record.last_seq) {
                covered.extend(f.0..=l.0);
            }
        }
        let expected: Vec<u64> = ((genesis_head.0 + 1)..=final_head.0).collect();
        assert_eq!(covered, expected, "the spans tile the steps' events");

        // Both burst messages landed durably as participant turns, whichever way the race fell.
        let events = ctx.events().expect("the log");
        assert!(analysis::participant_turn_recorded(&events, FIRST));
        assert!(analysis::participant_turn_recorded(&events, CORRECTION));
    }

    #[tokio::test]
    async fn confirm_proposed_merge_skips_a_missing_proposal_and_continues() {
        let ctx = booted(ScriptedModel::new([])).await;
        let steps = vec![
            EvalStep::ConfirmProposedMerge {
                on_missing: OnMissing::Skip,
            },
            EvalStep::SeedEvents(one_memory("person/after")),
        ];
        let journal = execute(&steps, &ctx)
            .await
            .expect("the run continues past a skip");

        // The confirm step is journaled as skipped, having performed nothing.
        assert!(journal[0].skipped);
        assert!(journal[0].first_seq.is_none() && journal[0].last_seq.is_none());
        // The following step still runs — the skip is not an abort.
        assert!(!journal[1].skipped);
        assert!(journal[1].first_seq.is_some());
    }

    #[tokio::test]
    async fn confirm_proposed_merge_fails_a_required_but_missing_proposal() {
        let ctx = booted(ScriptedModel::new([])).await;
        let steps = vec![EvalStep::ConfirmProposedMerge {
            on_missing: OnMissing::Fail,
        }];
        let error = execute(&steps, &ctx)
            .await
            .expect_err("a required proposal that is absent fails the run");
        assert!(matches!(error, EvalError::Executor(_)), "got {error:?}");
    }

    #[tokio::test]
    async fn with_turn_ref_resolves_the_reference_to_the_recorded_turn_token() {
        const ANCHOR: &str = "We ship on the 14th and Priya owns the release.";
        let ctx = booted(ScriptedModel::new([
            Completion::Reply("Understood.".to_owned()),
            Completion::Reply("Here is what you said.".to_owned()),
        ]))
        .await;
        let steps = vec![
            Turn::new(TEST_PLATFORM, "room", "sarah", ANCHOR).into(),
            Turn::new(
                TEST_PLATFORM,
                "room",
                "sarah",
                StepText::with_turn_ref("Reminder: {turn}", ANCHOR),
            )
            .into(),
        ];
        execute(&steps, &ctx).await.expect("the steps execute");

        let events = ctx.events().expect("the log");
        let anchor_id = events
            .iter()
            .find_map(|event| match &event.payload {
                EventPayload::ConversationTurn {
                    turn_id,
                    role: TurnRole::Participant,
                    text,
                    ..
                } if text.as_str() == ANCHOR => Some(turn_id.0.to_string()),
                _ => None,
            })
            .expect("the anchor turn is recorded");
        let expected = format!("Reminder: [turn:{anchor_id}]");
        let resolved = events.iter().any(|event| {
            matches!(
                &event.payload,
                EventPayload::ConversationTurn { role: TurnRole::Participant, text, .. }
                    if text.as_str() == expected
            )
        });
        assert!(
            resolved,
            "the referencing turn should be routed as {expected:?}"
        );
    }

    #[tokio::test]
    async fn with_turn_ref_errors_on_an_unresolvable_reference() {
        let ctx = booted(ScriptedModel::new([])).await;
        let steps = vec![
            Turn::new(
                TEST_PLATFORM,
                "room",
                "sarah",
                StepText::with_turn_ref("Reminder: {turn}", "a moment never recorded"),
            )
            .into(),
        ];
        let error = execute(&steps, &ctx)
            .await
            .expect_err("an unresolvable turn reference fails the run");
        assert!(matches!(error, EvalError::Executor(_)), "got {error:?}");
    }
}
