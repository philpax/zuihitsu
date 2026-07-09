//! Block effects, guards, and content buffering: transaction, visibility guards, teller labels,
//! and the push/handle methods.

use std::collections::BTreeSet;

use crate::{
    event::{EventPayload, Teller, Visibility},
    ids::MemoryId,
    time::TemporalRef,
};

use super::{Authority, BlockEffects, EntryId, MemoryBlock, MemoryError};

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

    /// Consume the block for commit: the buffered events, the touched lock set, and any abort reason.
    pub fn into_effects(self) -> BlockEffects {
        BlockEffects {
            events: self.buffer,
            touched: self.touched.into_iter().collect(),
            aborted: self.aborted,
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

    /// The live entry ids of `id`'s `same_as` class: committed-live (the graph already excludes
    /// committed supersessions) plus this block's pending appends, minus what it has superseded —
    /// the set [`MemoryBlock::supersede`] validates its arguments against.
    pub(super) fn live_class_entry_ids(
        &self,
        id: MemoryId,
    ) -> Result<BTreeSet<EntryId>, MemoryError> {
        let (members, committed) = {
            let graph = self.engine.graph.lock();
            (graph.class_members(id)?, graph.class_entries(id)?)
        };
        let members: BTreeSet<MemoryId> = members.into_iter().chain([id]).collect();
        let pending_superseded = self.pending_superseded();
        let mut ids: BTreeSet<EntryId> = committed
            .into_iter()
            .map(|entry| entry.entry_id)
            .filter(|entry_id| !pending_superseded.contains(entry_id))
            .collect();
        for entry in self.pending_entries(&members, &pending_superseded) {
            ids.insert(entry.entry_id);
        }
        Ok(ids)
    }

    /// Buffer a content entry and touch its memory, returning the minted entry id (so a write can be
    /// handed back to the agent as an addressable entry — see [`MemoryBlock::append`]).
    pub(super) fn push_content(
        &mut self,
        id: MemoryId,
        text: String,
        told_by: Teller,
        visibility: Visibility,
        occurred_at: Option<TemporalRef>,
    ) -> EntryId {
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
        entry_id
    }
}
