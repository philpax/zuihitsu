//! Attribute projection and context writes: recording a connector's channel metadata onto a scope's
//! context memory, and projecting participant handles onto their `person/*` stubs.

use crate::{
    event::{EventSource, Teller},
    ids::ConversationLocator,
    instance::{
        ContextEntry, InstanceError,
        platform::{
            LinkNode, ParticipantAttribute, Platform, ProjectOutcome, retract_if_live,
            supersede_if_live,
        },
    },
    memory::{
        identity::resolve_or_mint_context,
        memory_block::{AppendOptions, Authority, MemoryBlock, VisibilityChoice},
    },
};

impl Platform<'_> {
    /// Write context entries to a conversation's context memory under platform authority. A
    /// connector (e.g. the Discord bot) uses this to write channel metadata and laconic guidance on
    /// first contact, posting structured data rather than interpolating untrusted strings into code.
    ///
    /// The context memory is resolved (or minted) by name from the locator's scope — independent of any
    /// conversation, so a connector can establish context for a scope that has no messages of its own (a
    /// guild), and can establish a room's context before its first participant message. A room's first
    /// message reuses the same memory by name. Each entry is appended as `Public` under the agent's
    /// teller. The `max_entry_chars` guard is bypassed (passed as `usize::MAX`): platform-authority
    /// context writes are blessed, like self-memories, and not subject to the agent's entry length limit.
    pub fn write_context(
        &self,
        locator: &ConversationLocator,
        platform: &str,
        entries: &[ContextEntry],
    ) -> Result<(), InstanceError> {
        if entries.is_empty() {
            return Ok(());
        }
        // Resolve (or mint) the scope's context memory by name — no conversation, so this works for a
        // guild as well as a room. One graph guard spans the resolve, the mint, and the materialize
        // (as in `ensure_conversation`), so a concurrent first contact for the same scope cannot mint a
        // second context memory whose duplicate name would wedge every later fold. Graph before store,
        // per the lock-ordering rule; the store is locked transiently within the span.
        let engine = &self.server.engine;
        let context_memory = {
            let mut graph = engine.graph.lock();
            let id = resolve_or_mint_context(
                engine.store.lock().as_mut(),
                engine.clock.as_ref(),
                &graph,
                locator,
            )?;
            graph.materialize_from(engine.store.lock().as_ref())?;
            id
        };

        // Connector-authored: these entries commit under `EventSource::PlatformConnector` and belong on
        // the exact stub the connector holds ids against, so they must not follow an agent write's
        // redirect to the class primary (see `MemoryBlock::class_write_target`).
        let mut block = MemoryBlock::new(
            engine.clone(),
            Teller::Agent,
            Authority::Platform,
            None,
            None,
            Vec::new(),
            usize::MAX,
        )?
        .authored_by_connector();
        for entry in entries {
            let opts = AppendOptions {
                visibility: Some(VisibilityChoice::Public),
                ..AppendOptions::default()
            };
            block
                .append(context_memory, &entry.text, opts)
                .map_err(InstanceError::Memory)?;
        }
        let now = engine.clock.now();
        engine.store.lock().append(
            now,
            EventSource::PlatformConnector(platform.to_owned()),
            block.into_effects().events,
        )?;
        engine
            .graph
            .lock()
            .materialize_from(engine.store.lock().as_ref())?;
        Ok(())
    }

    /// Project platform attributes onto a scoped memory as ordinary `Public` entries: a participant's
    /// current handles onto their `person/*` stub, or a guild's name onto its `context/*` memory. Each
    /// attribute either records a new value, superseding the entry a prior projection returned for it, or
    /// clears a value no longer set, retracting that entry. The connector holds the entry ids, so the
    /// server keys nothing itself.
    ///
    /// The target is resolved (or minted), so a projection lands even on first contact. Returns the
    /// memory id it landed on and the new entry id per attribute, in request order: `Some` for a
    /// recorded value, `None` for a cleared or absent one. The memory id is returned even when no
    /// attribute changed (an empty `attributes`), so a connector can learn a subject's memory id to
    /// reference it without recording anything. A supersede or retract target the agent has since
    /// dropped is a no-op — the fresh append still stands — so a projection never fails on a target that
    /// moved underneath it.
    pub fn project(
        &self,
        target: &LinkNode,
        platform: &str,
        attributes: &[ParticipantAttribute],
    ) -> Result<ProjectOutcome, InstanceError> {
        let engine = &self.server.engine;
        // Resolve (or mint) the target memory, the same path a message or a link takes. It is resolved
        // even with no attributes, so the caller always learns the subject's memory id.
        // `resolve_or_mint_node` already materializes the mint under its lock.
        let memory = self.resolve_or_mint_node(target)?;
        if attributes.is_empty() {
            return Ok(ProjectOutcome {
                memory_id: memory,
                entries: Vec::new(),
            });
        }

        // No conversation to attribute to — a projection is about the subject, not a room.
        // Connector-authored: these entries commit under `EventSource::PlatformConnector` and belong on
        // the exact stub the connector holds ids against, so they must not follow an agent write's
        // redirect to the class primary (see `MemoryBlock::class_write_target`).
        let mut block = MemoryBlock::new(
            engine.clone(),
            Teller::Agent,
            Authority::Platform,
            None,
            None,
            Vec::new(),
            usize::MAX,
        )?
        .authored_by_connector();
        let mut results = Vec::with_capacity(attributes.len());
        for attribute in attributes {
            match &attribute.text {
                Some(text) => {
                    let opts = AppendOptions {
                        visibility: Some(VisibilityChoice::Public),
                        ..AppendOptions::default()
                    };
                    let new = block
                        .append(memory, text, opts)
                        .map_err(InstanceError::Memory)?;
                    if let Some(old) = attribute.supersedes {
                        supersede_if_live(&mut block, memory, old, new)?;
                    }
                    results.push(Some(new));
                }
                None => {
                    if let Some(old) = attribute.supersedes {
                        retract_if_live(&mut block, memory, old)?;
                    }
                    results.push(None);
                }
            }
        }
        let now = engine.clock.now();
        engine.store.lock().append(
            now,
            EventSource::PlatformConnector(platform.to_owned()),
            block.into_effects().events,
        )?;
        engine
            .graph
            .lock()
            .materialize_from(engine.store.lock().as_ref())?;
        Ok(ProjectOutcome {
            memory_id: memory,
            entries: results,
        })
    }
}
