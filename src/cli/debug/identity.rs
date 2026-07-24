//! The `designate-primary` and `merge` identity commands: two more of the deliberate write exceptions
//! in the otherwise read-only `debug` namespace. `designate-primary` pins which member of a `same_as`
//! class renders as its canonical identity (releasing any incumbent in the same batch); `merge` binds
//! two memories into one identity with an operator-asserted `same_as`. Each appends one forward,
//! operator-sourced batch to the log, so the change is auditable and itself revertible — history is
//! never rewritten. Both open the event log read-write, so the agent must be stopped first: the open
//! takes the single-writer log lock and fails while a running agent holds it.
//!
//! `merge` writes a raw `same_as` [`EventPayload::LinkCreated`] under [`EventSource::Operator`],
//! bypassing the block layer's merge-proposal guards deliberately: an operator's `same_as` assertion is
//! always within the operator's authority. The free-merge guard constrains an agent's writes
//! ([`Authority::Agent`](zuihitsu::Authority)), not the operator's.
//!
//! Both share [`resolve_memory`]: an exact memory name or a unique id prefix is resolved against a graph
//! freshly materialised from the log, erroring when the prefix is ambiguous (listing the candidates) or
//! matches nothing live.

use zuihitsu::{
    Clock, EventPayload, EventSource, Graph, LinkPosture, LinkSource, MemoryId, MemoryName,
    MemoryView, RelationName, Store, SystemClock, Visibility, config::EnvConfig,
};

use crate::cli::{
    debug::correction::{materialize, open_store},
    error::CliError,
};

/// Designate `target` the primary of its `same_as` class: append a batch that releases every *other*
/// currently-designated member of the class, then designates `target`. Releasing the incumbent in the
/// same batch matters — the recompute breaks a tie among designated members by earliest ULID, so
/// designating without releasing would silently keep an earlier-ULID incumbent. Designating a lone
/// memory (one in no `same_as` class) is legal and harmless; it is reported as such. With `release`
/// set, the command instead only withdraws `target`'s own designation, designating nothing new.
pub(crate) fn designate_primary(
    config: &EnvConfig,
    target: &str,
    release: bool,
) -> Result<(), CliError> {
    let mut store = open_store(config, CliError::DesignatePrimary)?;
    let graph = materialize(&store, CliError::DesignatePrimary)?;
    let memory = resolve_memory(&graph, target).map_err(CliError::DesignatePrimary)?;

    let members = graph.class_members(memory.id).map_err(|source| {
        CliError::DesignatePrimary(format!("could not read the identity class: {source}"))
    })?;
    let lone = members.len() <= 1;

    if release {
        if !graph.is_primary_designated(memory.id).map_err(|source| {
            CliError::DesignatePrimary(format!("could not read the designation: {source}"))
        })? {
            tracing::info!(
                "{} ({}) is not the designated primary; nothing to release",
                memory.name.as_str(),
                memory.id.0,
            );
            return Ok(());
        }
        store
            .append(
                SystemClock.now(),
                EventSource::Operator,
                vec![EventPayload::class_primary_designated(memory.id, false)],
            )
            .map_err(|source| {
                CliError::DesignatePrimary(format!("could not append the release: {source}"))
            })?;
        tracing::info!(
            "released the primary designation on {} ({}); the class falls back to its earliest-ULID \
             member on the next fold",
            memory.name.as_str(),
            memory.id.0,
        );
        return Ok(());
    }

    // Release every other currently-designated member, then designate the target — one batch, so the
    // recompute never sees the target and an incumbent both designated.
    let mut events = Vec::new();
    let mut released: Vec<MemoryId> = Vec::new();
    for &member in &members {
        if member == memory.id {
            continue;
        }
        if graph.is_primary_designated(member).map_err(|source| {
            CliError::DesignatePrimary(format!("could not read a member's designation: {source}"))
        })? {
            events.push(EventPayload::class_primary_designated(member, false));
            released.push(member);
        }
    }

    let already = graph.is_primary_designated(memory.id).map_err(|source| {
        CliError::DesignatePrimary(format!("could not read the designation: {source}"))
    })?;
    if already && events.is_empty() {
        tracing::info!(
            "{} ({}) is already the sole designated primary; nothing to do",
            memory.name.as_str(),
            memory.id.0,
        );
        return Ok(());
    }

    events.push(EventPayload::class_primary_designated(memory.id, true));
    store
        .append(SystemClock.now(), EventSource::Operator, events)
        .map_err(|source| {
            CliError::DesignatePrimary(format!("could not append the designation: {source}"))
        })?;

    if !released.is_empty() {
        tracing::info!(
            "released the prior designation on: {}",
            member_lines(&graph, &released).join(", "),
        );
    }
    tracing::info!(
        "designated {} ({}) the primary of its identity class{}. Its relationships render under this \
         handle on the next fold.",
        memory.name.as_str(),
        memory.id.0,
        if lone {
            " (a lone memory — legal and harmless, though it has no other members)"
        } else {
            ""
        },
    );
    Ok(())
}

/// Merge `a` and `b` into one identity: append a single operator-asserted `same_as`
/// [`EventPayload::LinkCreated`] (public, no teller) binding them. Refuses a cross-namespace merge and
/// reports a no-op when the two already share an identity class. Reports the resulting class — its
/// members and which member is now the designated primary, or that none is, suggesting
/// `debug designate-primary`.
pub(crate) fn merge(config: &EnvConfig, a_target: &str, b_target: &str) -> Result<(), CliError> {
    let mut store = open_store(config, CliError::Merge)?;
    let graph = materialize(&store, CliError::Merge)?;
    let a = resolve_memory(&graph, a_target).map_err(CliError::Merge)?;
    let b = resolve_memory(&graph, b_target).map_err(CliError::Merge)?;

    if a.id == b.id {
        return Err(CliError::Merge(format!(
            "{} resolves to the same memory as {}; nothing to merge",
            a_target, b_target,
        )));
    }

    // Both must sit in the same namespace: a `same_as` binds one identity, never a person to a place.
    let ns_a = a.name.namespaced().map(|n| n.namespace);
    let ns_b = b.name.namespaced().map(|n| n.namespace);
    if ns_a != ns_b {
        return Err(CliError::Merge(format!(
            "refusing a cross-namespace merge: {} and {} are in different namespaces",
            a.name.as_str(),
            b.name.as_str(),
        )));
    }

    let class_a = graph.class_id(a.id).map_err(|source| {
        CliError::Merge(format!(
            "could not read {}'s class: {source}",
            a.name.as_str()
        ))
    })?;
    let class_b = graph.class_id(b.id).map_err(|source| {
        CliError::Merge(format!(
            "could not read {}'s class: {source}",
            b.name.as_str()
        ))
    })?;
    if class_a.is_some() && class_a == class_b {
        tracing::info!(
            "{} ({}) and {} ({}) are already in the same identity class; nothing to merge",
            a.name.as_str(),
            a.id.0,
            b.name.as_str(),
            b.id.0,
        );
        return Ok(());
    }

    store
        .append(
            SystemClock.now(),
            EventSource::Operator,
            vec![EventPayload::link_created(
                a.id,
                b.id,
                RelationName::SameAs,
                LinkPosture {
                    source: LinkSource::Operator,
                    told_by: None,
                    told_in: None,
                    visibility: Visibility::Public,
                },
            )],
        )
        .map_err(|source| {
            CliError::Merge(format!("could not append the same_as link: {source}"))
        })?;

    // Re-materialise so the resulting class reflects the append, then report it.
    let after = materialize(&store, CliError::Merge)?;
    let members = after
        .class_members(a.id)
        .map_err(|source| CliError::Merge(format!("could not read the merged class: {source}")))?;
    let designated: Vec<MemoryId> = members
        .iter()
        .copied()
        .filter(|member| after.is_primary_designated(*member).unwrap_or(false))
        .collect();

    tracing::info!(
        "merged {} ({}) and {} ({}) into one identity; the class now holds: {}",
        a.name.as_str(),
        a.id.0,
        b.name.as_str(),
        b.id.0,
        member_lines(&after, &members).join(", "),
    );
    match designated.as_slice() {
        [primary] => {
            if let Some(view) = after.memory_by_id(*primary).ok().flatten() {
                tracing::info!(
                    "the designated primary is {} ({})",
                    view.name.as_str(),
                    primary.0,
                );
            }
        }
        [] => tracing::info!(
            "no member is designated primary; the class renders under its earliest-ULID member. Pin \
             one with `debug designate-primary --memory <name>` if that is not the intended identity."
        ),
        _ => tracing::warn!(
            "several members are designated primary: {}. Pin exactly one with \
             `debug designate-primary --memory <name>`.",
            member_lines(&after, &designated).join(", "),
        ),
    }
    Ok(())
}

/// Resolve `target` — an exact memory name (e.g. `person/rowan`) or a unique id prefix — to exactly one
/// live memory. An exact name wins first; failing that, a full id or unique id prefix resolves. An
/// ambiguous prefix is an error listing the candidates; a prefix matching nothing live is an error too.
/// The error is a bare message the caller wraps in its own command context.
fn resolve_memory(graph: &Graph, target: &str) -> Result<MemoryView, String> {
    if let Some(memory) = graph
        .memory_by_name(MemoryName::new(target))
        .map_err(|source| format!("could not look up the name: {source}"))?
    {
        return Ok(memory);
    }
    let candidates = graph
        .memory_ids_with_prefix(target)
        .map_err(|source| format!("could not resolve the id prefix: {source}"))?;
    match candidates.as_slice() {
        [] => Err(format!(
            "no live memory found with name or id prefix {target:?}"
        )),
        [id] => graph
            .memory_by_id(*id)
            .map_err(|source| format!("could not read the memory: {source}"))?
            .ok_or_else(|| format!("memory {} is not live", id.0)),
        many => Err(format!(
            "ambiguous id prefix {target:?} matches {} memories: {}",
            many.len(),
            member_lines(graph, many).join(", "),
        )),
    }
}

/// Render each id as `name (id)` for a candidate or class listing, falling back to the bare id when the
/// memory does not resolve (a race with a concurrent delete, which the reader tolerates rather than
/// erroring the whole report).
fn member_lines(graph: &Graph, ids: &[MemoryId]) -> Vec<String> {
    ids.iter()
        .map(|id| match graph.memory_by_id(*id).ok().flatten() {
            Some(view) => format!("{} ({})", view.name.as_str(), id.0),
            None => id.0.to_string(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    //! In-memory, like the other `debug` command tests: they exercise the resolution and event-shape
    //! logic over a folded log rather than opening a real store. Here that is [`resolve_memory`] and the
    //! append batches `designate-primary` and `merge` build. The disk plumbing (opening the log under
    //! the lock, appending) is the same thin shell the sibling commands leave untested; the
    //! `ClassPrimaryDesignated` and `LinkCreated` folds are covered by the graph's own tests.
    use super::resolve_memory;
    use zuihitsu::{
        Cardinality, Clock, EventPayload, EventSource, Graph, LinkPosture, LinkSource, MemoryId,
        MemoryName, MemoryStore, Namespace, RelationName, Store, SystemClock, Visibility,
    };

    /// A graph materialized from an in-memory log of `payloads`, appended under `EventSource::Operator`.
    fn graph_of(payloads: Vec<EventPayload>) -> Graph {
        let mut store = MemoryStore::new();
        for payload in payloads {
            store
                .append(SystemClock.now(), EventSource::Operator, vec![payload])
                .unwrap();
        }
        let mut graph = Graph::open_in_memory().unwrap();
        graph.materialize_from(&store).unwrap();
        graph
    }

    /// The `same_as` relation registration every identity fold needs in the log.
    fn same_as_registered() -> EventPayload {
        EventPayload::LinkTypeRegistered {
            name: RelationName::SameAs,
            inverse: RelationName::SameAs,
            from_card: Cardinality::Many,
            to_card: Cardinality::Many,
            symmetric: true,
            reflexive: false,
            description: String::new(),
        }
    }

    #[test]
    fn a_memory_resolves_by_exact_name_and_by_unique_id_prefix() {
        let id = MemoryId::generate();
        let graph = graph_of(vec![EventPayload::memory_created(
            id,
            Namespace::Person.with_name("rowan"),
        )]);
        assert_eq!(resolve_memory(&graph, "person/rowan").unwrap().id, id);
        assert_eq!(resolve_memory(&graph, &id.0.to_string()).unwrap().id, id);
        // A prefix of the id resolves the same memory, in either casing.
        let prefix = &id.0.to_string()[..10];
        assert_eq!(resolve_memory(&graph, prefix).unwrap().id, id);
        assert_eq!(
            resolve_memory(&graph, &prefix.to_lowercase()).unwrap().id,
            id
        );
    }

    #[test]
    fn an_unknown_target_is_an_error() {
        let graph = graph_of(vec![EventPayload::memory_created(
            MemoryId::generate(),
            Namespace::Person.with_name("rowan"),
        )]);
        let error = resolve_memory(&graph, "person/nobody").unwrap_err();
        assert!(error.contains("no live memory found"), "got: {error}");
    }

    #[test]
    fn an_ambiguous_id_prefix_lists_the_candidates() {
        let a = MemoryId::generate();
        let b = MemoryId::generate();
        let graph = graph_of(vec![
            EventPayload::memory_created(a, Namespace::Person.with_name("rowan")),
            EventPayload::memory_created(b, Namespace::Person.with_name("erin")),
        ]);
        // The empty string is the guaranteed-common prefix that matches both ids.
        let error = resolve_memory(&graph, "").unwrap_err();
        assert!(error.contains("ambiguous id prefix"), "got: {error}");
        assert!(error.contains(&a.0.to_string()), "got: {error}");
        assert!(error.contains(&b.0.to_string()), "got: {error}");
    }

    /// The batch `designate-primary` builds, computed here directly (the command's core, minus the disk
    /// shell): release every other currently-designated member, then designate the target — releases
    /// first, so the recompute never sees two designations at once.
    fn designate_batch(graph: &Graph, target: MemoryId) -> Vec<EventPayload> {
        let members = graph.class_members(target).unwrap();
        let mut events = Vec::new();
        for member in &members {
            if *member != target && graph.is_primary_designated(*member).unwrap() {
                events.push(EventPayload::class_primary_designated(*member, false));
            }
        }
        events.push(EventPayload::class_primary_designated(target, true));
        events
    }

    #[test]
    fn designate_releases_the_incumbent_before_designating_the_target() {
        // A class of two bare members with `person/operator` (an imprint artifact) currently designated.
        // Designating the other must release the artifact first, in the same batch.
        let artifact = MemoryId::generate();
        let real = MemoryId::generate();
        let graph = graph_of(vec![
            same_as_registered(),
            EventPayload::memory_created(artifact, Namespace::Person.with_name("operator")),
            EventPayload::memory_created(real, Namespace::Person.with_name("rowan")),
            EventPayload::link_created(
                artifact,
                real,
                RelationName::SameAs,
                LinkPosture {
                    source: LinkSource::Operator,
                    told_by: None,
                    told_in: None,
                    visibility: Visibility::Public,
                },
            ),
            EventPayload::class_primary_designated(artifact, true),
        ]);

        let events = designate_batch(&graph, real);
        assert_eq!(
            events,
            vec![
                EventPayload::class_primary_designated(artifact, false),
                EventPayload::class_primary_designated(real, true),
            ],
            "the incumbent is released before the target is designated, in one batch"
        );
    }

    #[test]
    fn designate_a_lone_memory_is_a_single_designation() {
        // A lone memory has no other members, so the batch is just its own designation.
        let lone = MemoryId::generate();
        let graph = graph_of(vec![EventPayload::memory_created(
            lone,
            Namespace::Person.with_name("rowan"),
        )]);
        assert_eq!(
            designate_batch(&graph, lone),
            vec![EventPayload::class_primary_designated(lone, true)],
        );
    }

    #[test]
    fn merge_binds_two_memories_into_the_same_class() {
        // Two lone person profiles, in distinct classes, merge into one on a `same_as` append. The link
        // is appended to the same store the memories live in, so the re-fold sees the whole history.
        let a = MemoryId::generate();
        let b = MemoryId::generate();
        let mut store = MemoryStore::new();
        store
            .append(
                SystemClock.now(),
                EventSource::Operator,
                vec![
                    same_as_registered(),
                    EventPayload::memory_created(a, Namespace::Person.with_name("rowan")),
                    EventPayload::memory_created(b, Namespace::Person.with_name("rowan@discord")),
                ],
            )
            .unwrap();
        let mut graph = Graph::open_in_memory().unwrap();
        graph.materialize_from(&store).unwrap();
        assert_ne!(
            graph.class_id(a).unwrap(),
            graph.class_id(b).unwrap(),
            "the two start in distinct classes"
        );

        // The command's one append: an operator-sourced, public, tellerless `same_as`.
        store
            .append(
                SystemClock.now(),
                EventSource::Operator,
                vec![EventPayload::link_created(
                    a,
                    b,
                    RelationName::SameAs,
                    LinkPosture {
                        source: LinkSource::Operator,
                        told_by: None,
                        told_in: None,
                        visibility: Visibility::Public,
                    },
                )],
            )
            .unwrap();
        graph.materialize_from(&store).unwrap();

        let members = graph.class_members(a).unwrap();
        assert!(
            members.contains(&a) && members.contains(&b),
            "both memories share one class after the merge: {members:?}"
        );
    }

    #[test]
    fn two_members_of_one_class_are_a_merge_no_op() {
        // Already bound by `same_as`, the two share a class id — the command detects this and appends
        // nothing.
        let a = MemoryId::generate();
        let b = MemoryId::generate();
        let graph = graph_of(vec![
            same_as_registered(),
            EventPayload::memory_created(a, Namespace::Person.with_name("rowan")),
            EventPayload::memory_created(b, Namespace::Person.with_name("rowan@discord")),
            EventPayload::link_created(
                a,
                b,
                RelationName::SameAs,
                LinkPosture {
                    source: LinkSource::Operator,
                    told_by: None,
                    told_in: None,
                    visibility: Visibility::Public,
                },
            ),
        ]);
        let class_a = graph.class_id(a).unwrap();
        let class_b = graph.class_id(b).unwrap();
        assert!(
            class_a.is_some() && class_a == class_b,
            "both already share a class, so the merge is a no-op"
        );
    }

    #[test]
    fn a_cross_namespace_merge_is_refused() {
        // A person and a place are in different namespaces; the resolver's callers refuse the bind. The
        // namespace comparison is the guard, checked here on the resolved handles.
        let person = MemoryName::new("person/rowan");
        let place = MemoryName::new("place/office");
        assert_ne!(
            person.namespaced().map(|n| n.namespace),
            place.namespaced().map(|n| n.namespace),
            "a person and a place are in different namespaces, so the merge is refused"
        );
    }
}
