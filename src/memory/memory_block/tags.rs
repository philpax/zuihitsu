//! Tag operations: creating, describing, applying, and removing tags, plus vocabulary reads.

use crate::{
    event::EventPayload,
    graph::{GraphError, TagVocabularyEntry},
    ids::MemoryId,
    memory::memory_block::{MemoryBlock, MemoryError, suggest::most_similar},
    vocabulary::TagName,
};

impl MemoryBlock {
    /// Create a tag with a one-line purpose. A tag's description is set only at creation; applying it
    /// never mutates it (spec §Tag operations). A name already in the vocabulary is a teachable error.
    pub fn create_tag(&mut self, name: TagName, description: &str) -> Result<(), MemoryError> {
        if self.tag_exists(&name)? {
            let similar = self.similar_tags(&name)?;
            return Err(MemoryError::TagExists { name, similar });
        }
        self.buffer
            .push(EventPayload::tag_created(name, description));
        Ok(())
    }

    /// Change an existing tag's one-line purpose. The tag must already exist — re-describing an
    /// unknown tag is a teachable error (create it first).
    pub fn describe_tag(&mut self, name: TagName, description: &str) -> Result<(), MemoryError> {
        if !self.tag_exists(&name)? {
            return Err(MemoryError::UnknownTag(name));
        }
        self.buffer.push(EventPayload::tag_description_changed(
            name,
            description.to_owned(),
        ));
        Ok(())
    }

    /// Apply a tag to a memory. The tag must be in the vocabulary (`tags.create` first) — applying an
    /// unknown tag is a teachable error, since a tag is a shared, described vocabulary rather than an
    /// ad-hoc label. Tagging is idempotent at the projection (`INSERT OR IGNORE`).
    pub fn tag(&mut self, id: MemoryId, tag: TagName) -> Result<(), MemoryError> {
        self.guard_self(id)?;
        if !self.tag_exists(&tag)? {
            return Err(MemoryError::UnknownTag(tag));
        }
        self.touched.insert(id);
        self.buffer
            .push(EventPayload::tag_applied_to_memory(id, tag));
        Ok(())
    }

    /// Remove a tag from a memory. Idempotent — removing a tag the memory does not carry is a no-op
    /// at the projection — so it needs no vocabulary check. A platform turn may not remove
    /// `#confidential` ([`MemoryBlock::guard_confidential_untag`]): weakening a room's confidentiality
    /// retroactively re-marks every aside told under it, so it stays operator-only.
    pub fn untag(&mut self, id: MemoryId, tag: TagName) -> Result<(), MemoryError> {
        self.guard_self(id)?;
        self.guard_confidential_untag(&tag)?;
        self.touched.insert(id);
        self.buffer
            .push(EventPayload::tag_removed_from_memory(id, tag));
        Ok(())
    }

    /// The whole tag vocabulary (committed), for `tags.list`. A plain read of the projection; this
    /// block's pending tag creations are not yet reflected, like every other committed read.
    pub fn all_tags(&self) -> Result<Vec<TagVocabularyEntry>, MemoryError> {
        Ok(self.engine.graph.lock().all_tags()?)
    }

    /// Whether `name` is a created tag — checking this block's pending `TagCreated`s (read-your-writes)
    /// before the committed vocabulary, so a tag created and applied within the same block is
    /// recognized.
    pub(super) fn tag_exists(&self, name: &TagName) -> Result<bool, MemoryError> {
        let pending = self.buffer.iter().any(|event| {
            matches!(event, EventPayload::TagCreated { name: created, .. } if created == name)
        });
        if pending {
            return Ok(true);
        }
        Ok(self
            .engine
            .graph
            .lock()
            .tag_description(name.as_str())?
            .is_some())
    }

    /// The near-matching existing tags, closest first — the suggestions a tag collision surfaces so
    /// the agent applies a near-duplicate it may have meant rather than minting another. Tags share no
    /// namespace, so the whole committed vocabulary is the candidate set — and unlike memory names
    /// it is deliberately not sliced by first character: the vocabulary is a small, curated set (one
    /// row per described tag, nothing per-memory), so the whole fetch is already the cheap path.
    fn similar_tags(&self, attempted: &TagName) -> Result<Vec<TagName>, GraphError> {
        let candidates = self
            .engine
            .graph
            .lock()
            .all_tags()?
            .into_iter()
            .map(|entry| (entry.name.as_str().to_owned(), entry.name))
            .collect();
        Ok(most_similar(attempted.as_str(), candidates))
    }
}
