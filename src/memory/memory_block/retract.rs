//! Retraction writes: withdrawing a fact rather than replacing it in place. Under a conversation turn a
//! retraction is per-attester — the speaker withdraws only its own account and the fact stands while
//! another teller attests it — while a maintenance pass or the console retracts the whole entry. The
//! console's `retract_attestation` is the operator-driven per-attester counterpart, and the withdrawal
//! note names only the remaining attesters visible to the present audience.

use crate::{
    event::{EventPayload, ProducedBy, Teller},
    ids::{EntryId, MemoryId},
    memory::{
        memory_block::{Authority, EntrySelector, MemoryBlock, MemoryError, Retraction},
        visibility::visible_attestations,
    },
};

impl MemoryBlock {
    /// Retract `entry` on `id`, recording `reason` — the agent withdraws a fact rather than replacing
    /// it in place (spec §Visibility → superseded entries are not live). Under a conversation turn
    /// (platform authority) this is **per-attester**: a fact is a set of tellers' accounts, so a
    /// speaker withdraws only its own attestation and the fact stands as long as another teller still
    /// attests it. Three shapes follow:
    ///
    /// - The speaker attests the entry **and other tellers do too** → an [`EventPayload::AttestationRetracted`]
    ///   for the speaker's account alone; the entry stays live on the rest, and the returned
    ///   [`Retraction::Withdrawn`] carries a note naming the remaining *visible* attesters.
    /// - The speaker is the entry's **sole teller** → a whole-entry `EntryRetracted`, as before.
    /// - The speaker **attests nothing** on it → a whole-entry retraction, still governed by
    ///   [`MemoryBlock::guard_foreign_confidence_supersede`]: permitted for a public/attributed fact,
    ///   refused for another participant's confidence.
    ///
    /// A maintenance pass (agent authority) and the console (operator authority) always retract the
    /// whole entry regardless of who else attests it — their reach is deliberate and must not be
    /// silently narrowed to a single account. `entry` must be a live entry of `id`'s `same_as` class,
    /// and `reason` must be non-empty (an unexplained retraction is unauditable). Guarded like
    /// `supersede`: platform authority may not retract a `self` entry. A model-driven caller (the
    /// link-cleanup maintenance pass) passes its `produced_by`; a mechanical or agent-authored
    /// retraction passes `None`. The last-attestation case routes through `EntryRetracted` here, so
    /// the read-your-writes fold ([`MemoryBlock::pending_superseded`]) sees the tombstone without
    /// having to reason over pending attestation-retractions.
    pub fn retract(
        &mut self,
        id: MemoryId,
        entry: impl Into<EntrySelector>,
        reason: &str,
        produced_by: Option<ProducedBy>,
    ) -> Result<Retraction, MemoryError> {
        // Recorded against the class primary when told through a platform-agnostic handle, matching where
        // `supersede` lands the tombstone — `live_class_entries` gathers the whole class either way, so
        // the redirect only attributes the event to the primary.
        let id = self.class_write_target(id)?;
        self.guard_self(id)?;
        self.guard_operator(id)?;
        let reason = reason.trim();
        if reason.is_empty() {
            return Err(MemoryError::RetractionReasonRequired);
        }
        // Resolve a by-id-or-prefix reference against the class, like `supersede`, before the live check.
        let entry = self.resolve_entry_ref(id, entry.into())?;
        let live = self.live_class_entries(id)?;
        let Some(target) = live.iter().find(|e| e.entry_id == entry) else {
            return Err(MemoryError::UnknownEntry(entry.0.to_string()));
        };
        // A conversation turn retracts per-attester; a maintenance pass or the console always retracts
        // the whole entry.
        if self.authority == Authority::Platform {
            let speaker = self.teller.clone();
            let mut speaker_accounts: Vec<Teller> = Vec::new();
            let mut others_attest = false;
            for (teller, _) in &target.attestations {
                if self.same_teller_class(teller, &speaker)? {
                    speaker_accounts.push(teller.clone());
                } else {
                    others_attest = true;
                }
            }
            if !speaker_accounts.is_empty() && others_attest {
                // The speaker stands among several tellers: withdraw only its account. Name the
                // remaining visible attesters before buffering, so the note tells the agent the fact
                // still stands and by whom — for the present audience, never leaking a hidden attester.
                let remaining = self.remaining_visible_attesters(entry, &speaker)?;
                self.touched.insert(id);
                for account in speaker_accounts {
                    self.buffer.push(EventPayload::attestation_retracted(
                        id,
                        entry,
                        account,
                        reason,
                        produced_by.clone(),
                    ));
                }
                return Ok(Retraction::Withdrawn {
                    note: withdrawal_note(entry, &remaining),
                });
            }
        }
        // Whole-entry retraction: the speaker is the sole teller, a non-attester on a public/attributed
        // fact, or any agent/operator write. The foreign-confidence guard still governs a platform
        // non-attester's reach over a confidence.
        self.guard_foreign_confidence_supersede(target)?;
        self.touched.insert(id);
        self.buffer.push(EventPayload::entry_retracted(
            id,
            entry,
            reason,
            produced_by,
        ));
        Ok(Retraction::Entry)
    }

    /// Withdraw one named teller's attestation from `entry` under operator authority — the console's
    /// per-attester counterpart to [`MemoryBlock::retract`]. Buffers an
    /// [`EventPayload::AttestationRetracted`] for every live attestation of `teller`'s class on the
    /// entry; if that leaves no live attestation, the fold tombstones the entry exactly as a
    /// whole-entry retraction does (see the `AttestationRetracted` apply). `entry` must be a live entry
    /// of `id`'s class, `reason` non-empty, and `teller` must actually attest the entry
    /// ([`MemoryError::UnknownAttestation`] otherwise). Operator-only in practice — it emits under
    /// whatever authority the block carries, and the console builds an operator block.
    pub fn retract_attestation(
        &mut self,
        id: MemoryId,
        entry: impl Into<EntrySelector>,
        teller: Teller,
        reason: &str,
        produced_by: Option<ProducedBy>,
    ) -> Result<(), MemoryError> {
        let id = self.class_write_target(id)?;
        self.guard_self(id)?;
        self.guard_operator(id)?;
        let reason = reason.trim();
        if reason.is_empty() {
            return Err(MemoryError::RetractionReasonRequired);
        }
        let entry = self.resolve_entry_ref(id, entry.into())?;
        let live = self.live_class_entries(id)?;
        let Some(target) = live.iter().find(|e| e.entry_id == entry) else {
            return Err(MemoryError::UnknownEntry(entry.0.to_string()));
        };
        // Withdraw every live attestation of the named teller's class — a merged identity's account is
        // the same teller's, keyed on the exact stored teller value so the fold's `(entry, teller)`
        // row is matched.
        let mut accounts: Vec<Teller> = Vec::new();
        for (held, _) in &target.attestations {
            if self.same_teller_class(held, &teller)? {
                accounts.push(held.clone());
            }
        }
        if accounts.is_empty() {
            return Err(MemoryError::UnknownAttestation);
        }
        self.touched.insert(id);
        for account in accounts {
            self.buffer.push(EventPayload::attestation_retracted(
                id,
                entry,
                account,
                reason,
                produced_by.clone(),
            ));
        }
        Ok(())
    }

    /// The readable labels of the tellers still standing behind `entry`, other than `speaker`, that are
    /// visible to the present audience — the names a withdrawal note may safely surface. Reads the
    /// committed attestation set through the chip-rule predicate ([`visible_attestations`]), so a
    /// hidden attester (a confidence whose teller is absent, or blocked by the subject rule) is never
    /// named, and drops the withdrawing speaker's own class. A pending same-block attestation is not
    /// reflected (matching the deferred read-your-writes for attestation reads); the committed set is
    /// the note's basis, which suffices since the fact being withdrawn was already committed.
    fn remaining_visible_attesters(
        &self,
        entry: EntryId,
        speaker: &Teller,
    ) -> Result<Vec<String>, MemoryError> {
        let committed = { self.engine.graph.lock().entry_by_id(entry)? };
        let Some((memory, view)) = committed else {
            return Ok(Vec::new());
        };
        let visible_tellers: Vec<Teller> = {
            let graph = self.engine.graph.lock();
            let class_of = |mid| graph.class_id(mid).map(|class| class.unwrap_or(mid));
            visible_attestations(&view, &memory, &self.present_set, &class_of)?
                .into_iter()
                .map(|attestation| attestation.teller.clone())
                .collect()
        };
        let mut labels = Vec::new();
        for teller in visible_tellers {
            if !self.same_teller_class(&teller, speaker)? {
                labels.push(self.teller_label(&teller));
            }
        }
        Ok(labels)
    }
}

/// The note a per-attester withdrawal hands back: the speaker's account is gone but the fact stands,
/// attested by the remaining tellers visible to the present audience. Names them when any are visible;
/// when the survivors are all hidden from this audience, it says the fact still stands without naming
/// anyone, so a hidden attester's existence is never leaked through the note.
fn withdrawal_note(entry: EntryId, remaining: &[String]) -> String {
    let ulid = entry.0;
    if remaining.is_empty() {
        return format!(
            "withdrew your account of entry {ulid}; the fact still stands, attested by others."
        );
    }
    let who = remaining.join(", ");
    format!("withdrew your account of entry {ulid} — the fact still stands, attested by {who}.")
}
