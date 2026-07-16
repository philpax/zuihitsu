//! Block effects, guards, and content buffering: transaction, visibility guards, teller labels,
//! and the push/handle methods.

use std::collections::BTreeSet;

use crate::{
    event::{EventPayload, Teller, Visibility},
    ids::MemoryId,
    time::TemporalRef,
    vocabulary::TagName,
};

use crate::memory::memory_block::{Authority, BlockEffects, EntryId, MemoryBlock, MemoryError};

/// A live entry of a memory's `same_as` class, reduced to the fields the supersede guards read: its
/// id, who told it, and its visibility posture. Assembled from the committed class entries and this
/// block's pending appends by [`MemoryBlock::live_class_entries`], so the guards read a corrected
/// entry within the same block (read-your-writes) rather than only the committed graph.
pub(super) struct LiveEntry {
    pub entry_id: EntryId,
    pub told_by: Teller,
    pub visibility: Visibility,
}

impl MemoryBlock {
    /// A handle to the shared backends and the present set this block runs under — the inputs the
    /// async `memory.search` needs (the embedder, vector index, graph, clock, settings, and visibility
    /// set). Returned together so the Lua layer can embed and search without holding the block lock.
    pub fn retrieval_handle(&self) -> (std::sync::Arc<crate::engine::Engine>, Vec<MemoryId>) {
        (self.engine.clone(), self.present_set.clone())
    }

    /// The backends and present set the `convo.turn` link resolver reads under — the running engine
    /// (for the event store and graph) and who is present in this conversation, so the resolver can
    /// apply the audience rule: a turn resolves iff everyone present here was in that moment's audience
    /// (spec §Transcripts). Returned together so the Lua layer can resolve without holding the block
    /// lock.
    pub fn turn_resolution_handle(&self) -> (std::sync::Arc<crate::engine::Engine>, Vec<MemoryId>) {
        (self.engine.clone(), self.present_set.clone())
    }

    /// The current conversation's context memory, or `None` — touches it so it enters the lock set.
    pub fn current_context(&mut self) -> Option<MemoryId> {
        if let Some(id) = self.context_memory {
            self.touched.insert(id);
            Some(id)
        } else {
            None
        }
    }

    /// Discard everything this block buffered and end it, recording `reason` as the terminal cause.
    pub fn abort(&mut self, reason: Option<String>) {
        self.aborted = Some(reason.unwrap_or_default());
    }

    /// Signal that the turn should end silently, committing this block's buffered writes. The
    /// `turn.skip(reason)` function sets this and raises a `RuntimeError` to stop execution, mirroring
    /// `abort`'s mechanism — but unlike an abort, the buffer is committed, not discarded.
    pub fn skip(&mut self, reason: Option<String>) {
        self.skip = Some(reason.unwrap_or_default());
    }

    /// Consume the block for commit: the buffered events, the touched lock set, and any abort or
    /// skip reason.
    pub fn into_effects(self) -> BlockEffects {
        BlockEffects {
            events: self.buffer,
            touched: self.touched.into_iter().collect(),
            aborted: self.aborted,
            skip: self.skip,
        }
    }

    /// Drain the block's effects without consuming it. The block now lives behind a shared
    /// `Arc<Mutex<…>>` (so the Lua functions can hold `'static` handles to it), which cannot be
    /// `try_unwrap`ped while those function references survive in the VM, so the caller reclaims the
    /// effects through the lock instead. Leaves the block empty.
    pub fn take_effects(&mut self) -> BlockEffects {
        BlockEffects {
            events: std::mem::take(&mut self.buffer),
            touched: std::mem::take(&mut self.touched).into_iter().collect(),
            aborted: self.aborted.take(),
            skip: self.skip.take(),
        }
    }

    /// Run a compound operation as a transaction: if `body` returns `Err`, discard every event it
    /// buffered so a failure partway through a multi-event operation leaves no orphaned writes, then
    /// propagate the error. The touched set is left intact — reads within the operation genuinely
    /// touched those memories, and a rolled-back write's target was still interacted with. A
    /// single-event operation needs no transaction: its one check-then-buffer is already atomic,
    /// since the check precedes the (infallible) buffer push. Used by [`MemoryBlock::revise`] and
    /// [`MemoryBlock::create_with_opts`].
    pub(super) fn transaction<R>(
        &mut self,
        body: impl FnOnce(&mut Self) -> Result<R, MemoryError>,
    ) -> Result<R, MemoryError> {
        let savepoint = self.buffer.len();
        match body(self) {
            Ok(value) => Ok(value),
            Err(error) => {
                self.buffer.truncate(savepoint);
                Err(error)
            }
        }
    }

    /// Reject a platform-authority write that touches `self`. The console (operator authority)
    /// is the only path permitted to edit `self`, so the self model cannot be forged from a
    /// conversation (spec §Imprint interview). `create("self")` needs no guard — it is already blocked
    /// by `NameExists`, since `self` is seeded at genesis.
    pub(super) fn guard_self(&self, id: MemoryId) -> Result<(), MemoryError> {
        if self.authority == Authority::Platform && Some(id) == self.self_id {
            return Err(MemoryError::SelfWriteForbidden);
        }
        Ok(())
    }

    /// Reject a content write to the `person/operator` anchor (under any authority). The anchor holds
    /// no content of its own — facts about the operator belong on their real `person/<name>` profile,
    /// which is merged into it — so it stays a pure merge target. The merge (`same_as`) and `created_by`
    /// links to it are not content, so they are unaffected.
    pub(super) fn guard_operator(&self, id: MemoryId) -> Result<(), MemoryError> {
        if Some(id) == self.operator_id {
            return Err(MemoryError::OperatorWriteForbidden);
        }
        Ok(())
    }

    /// Reject a platform-authority turn removing the `#confidential` tag from any memory. The
    /// teller-private marker resolves a room's `#confidential` flag at read time, so removing the tag
    /// retroactively weakens the disclosure-judgment signal on every historical aside told under it — a
    /// broadcast, retroactive change with no legitimate platform-turn use, mirroring the `self`-write
    /// rationale (spec §Trust model). Adding the tag stays ungated: adding is conservative, over-hedging
    /// at worst, and the agent sets it from conversational cues. Operator authority (the console) passes.
    pub(super) fn guard_confidential_untag(&self, tag: &TagName) -> Result<(), MemoryError> {
        if self.authority == Authority::Platform && *tag == TagName::Confidential {
            return Err(MemoryError::ConfidentialUntagForbidden);
        }
        Ok(())
    }

    /// Reject a platform-authority turn superseding another participant's confidence. Superseding drops
    /// an entry from every live surface, so a platform turn suppressing what a *different* participant
    /// confided would let one person retract another's entrusted fact. The gate fires only when the
    /// superseded entry is both non-public (a `PrivateToTeller`/`Exclude` confidence — public and
    /// attributed entries surface to anyone, so consolidating them is routine) and told by a participant
    /// who is not the current speaker's identity. An entry told by the agent or genesis is never gated,
    /// and operator authority (the console) passes. The teachable error deliberately does not name the
    /// foreign teller — who confided the fact is itself part of the confidence.
    pub(super) fn guard_foreign_confidence_supersede(
        &self,
        entry: &LiveEntry,
    ) -> Result<(), MemoryError> {
        if self.authority != Authority::Platform {
            return Ok(());
        }
        // Public and attributed entries surface to anyone; only a confidence is protected.
        if !matches!(
            entry.visibility,
            Visibility::PrivateToTeller | Visibility::Exclude(_)
        ) {
            return Ok(());
        }
        // Only a participant-told confidence can be foreign; agent- and genesis-told entries are not gated.
        let Teller::Participant(teller) = &entry.told_by else {
            return Ok(());
        };
        // The confidence is the current speaker's own (or a merged identity of theirs) iff the speaker is
        // a participant in the same `same_as` class. An agent or bootstrap speaker is never that teller.
        if let Teller::Participant(speaker) = &self.teller
            && self.same_participant_class(*speaker, *teller)?
        {
            return Ok(());
        }
        Err(MemoryError::ForeignConfidenceSupersedeForbidden)
    }

    /// Whether two participant memories resolve to the same `same_as` class — so a confidence told by a
    /// merged identity of the current speaker counts as the speaker's own. Falls back to the memory's
    /// own id when it is unmerged (its own class), matching the read-time visibility predicate's
    /// identity model.
    fn same_participant_class(&self, a: MemoryId, b: MemoryId) -> Result<bool, MemoryError> {
        if a == b {
            return Ok(true);
        }
        let graph = self.engine.graph.lock();
        let class_a = graph.class_id(a)?.unwrap_or(a);
        let class_b = graph.class_id(b)?.unwrap_or(b);
        Ok(class_a == class_b)
    }

    /// A readable label for who an entry is attributed to: the participant's canonical handle, `you`
    /// for the agent's own observations, or `genesis` for seeded content.
    pub(super) fn teller_label(&self, teller: &Teller) -> String {
        match teller {
            Teller::Participant(id) => self
                .resolve_name(*id)
                .ok()
                .flatten()
                .map(|name| name.as_str().to_owned())
                .unwrap_or_else(|| "someone".to_owned()),
            Teller::Agent => "you".to_owned(),
            Teller::Bootstrap => "genesis".to_owned(),
        }
    }

    /// The live entries of `id`'s `same_as` class: committed-live (the graph already excludes
    /// committed supersessions) plus this block's pending appends, minus what it has superseded —
    /// the set [`MemoryBlock::supersede`] validates its arguments against, carrying each entry's teller
    /// and visibility so the same pass feeds [`MemoryBlock::guard_foreign_confidence_supersede`]
    /// without a second read.
    pub(super) fn live_class_entries(&self, id: MemoryId) -> Result<Vec<LiveEntry>, MemoryError> {
        let (members, committed) = {
            let graph = self.engine.graph.lock();
            (graph.class_members(id)?, graph.class_entries(id)?)
        };
        let members: BTreeSet<MemoryId> = members.into_iter().chain([id]).collect();
        let pending_superseded = self.pending_superseded();
        let mut entries: Vec<LiveEntry> = committed
            .into_iter()
            .filter(|entry| !pending_superseded.contains(&entry.entry_id))
            .map(|entry| LiveEntry {
                entry_id: entry.entry_id,
                told_by: entry.told_by,
                visibility: entry.visibility,
            })
            .collect();
        for event in &self.buffer {
            if let EventPayload::MemoryContentAppended {
                id: entry_memory,
                entry_id,
                told_by,
                visibility,
                ..
            } = event
                && members.contains(entry_memory)
                && !pending_superseded.contains(entry_id)
            {
                entries.push(LiveEntry {
                    entry_id: *entry_id,
                    told_by: told_by.clone(),
                    visibility: visibility.clone(),
                });
            }
        }
        Ok(entries)
    }

    /// Buffer a content entry and touch its memory, returning the minted entry id (so a write can be
    /// handed back to the agent as an addressable entry — see [`MemoryBlock::append`]). Rejects text
    /// exceeding `max_entry_chars` before buffering anything, surfacing a teachable error that guides
    /// the agent to summarize rather than paste source content.
    pub(super) fn push_content(
        &mut self,
        id: MemoryId,
        text: String,
        told_by: Teller,
        visibility: Visibility,
        occurred_at: Option<TemporalRef>,
    ) -> Result<EntryId, MemoryError> {
        let length = text.chars().count();
        if length > self.max_entry_chars {
            return Err(MemoryError::ContentTooLong {
                length,
                limit: self.max_entry_chars,
            });
        }
        let entry_id = EntryId::generate();
        self.touched.insert(id);
        self.buffer.push(EventPayload::MemoryContentAppended {
            id,
            entry_id,
            asserted_at: self.engine.clock.now(),
            occurred_at,
            text,
            told_by,
            told_in: self.told_in.clone(),
            visibility,
        });
        Ok(entry_id)
    }
}
