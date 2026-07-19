use std::collections::BTreeMap;

use rusqlite::params;

use crate::{
    db::{query_map_into, query_opt_into},
    graph::{GraphError, backend},
    ids::{MemoryId, MemoryName},
    time::{BEFORE_AFTER_EPSILON_MILLIS, OccurrenceBounds, TemporalRef, Timestamp},
    vocabulary::RelationName,
};

use crate::graph::{Graph, apply::OccurrenceColumns};

impl Graph {
    /// Denormalize an `occurred_at` reference into the values the `content_entries` occurrence
    /// columns store: the tagged JSON plus the `(sort, lo, hi)` millisecond bounds. A `BeforeAfter`
    /// resolves its anchor against the projection so far (`anchor_bounds`); every other variant is
    /// pure. Shared by the append and the `EntryTemporalResolved` arms so they denormalize identically.
    pub(super) fn occurrence_columns(
        &self,
        occurred_at: Option<&TemporalRef>,
    ) -> Result<OccurrenceColumns, GraphError> {
        let bounds = match occurred_at {
            Some(reference) => {
                let anchor = match reference {
                    TemporalRef::BeforeAfter { anchor, .. } => self.anchor_bounds(anchor)?,
                    _ => None,
                };
                reference.bounds(anchor, BEFORE_AFTER_EPSILON_MILLIS)
            }
            None => OccurrenceBounds::default(),
        };
        Ok(OccurrenceColumns {
            json: occurred_at
                .map(serde_json::to_string)
                .transpose()
                .map_err(GraphError::Serialize)?,
            sort: bounds.sort.map(|at| at.as_millisecond()),
            lo: bounds.lo.map(|at| at.as_millisecond()),
            hi: bounds.hi.map(|at| at.as_millisecond()),
        })
    }

    /// The representative bounds of a `BeforeAfter` anchor, by name, for occurrence denormalization
    /// (spec §Time). Resolved from the entries already projected, taking the anchor's earliest timed
    /// entry. Deliberately **not** filtered by soft delete: `MemoryDeleted` preserves contents, so a
    /// deleted anchor's occurrence stays resolvable (spec §Known limitations → `BeforeAfter`). `None`
    /// when the anchor name is unknown or has no timed entry — the caller then derives empty bounds.
    pub(super) fn anchor_bounds(
        &self,
        anchor: &MemoryName,
    ) -> Result<Option<OccurrenceBounds>, GraphError> {
        let stmt = self.conn.prepare(
            "SELECT e.occurred_sort, e.occurred_lo, e.occurred_hi
             FROM content_entries e JOIN memories m ON m.id = e.memory_id
             WHERE m.name = ?1 AND e.occurred_sort IS NOT NULL
             ORDER BY e.occurred_sort LIMIT 1",
        )?;
        query_opt_into(stmt, params![anchor.as_str()], |row| {
            let (sort, lo, hi): (Option<i64>, Option<i64>, Option<i64>) = row.try_into()?;
            Ok::<_, GraphError>(OccurrenceBounds {
                sort: sort.map(Timestamp::from_millis),
                lo: lo.map(Timestamp::from_millis),
                hi: hi.map(Timestamp::from_millis),
            })
        })
    }

    /// Resolve a link (asserted under either label) to its stored canonical direction:
    /// `(from_id, to_id, canonical_relation)`. A relation matched by its inverse swaps endpoints;
    /// a symmetric relation orders endpoints so `(a, b)` and `(b, a)` collapse to one edge. An
    /// unregistered relation is stored as given (the Lua layer enforces registration in Stage 4). Shared
    /// with [`Graph::link_between`], which canonicalizes a lookup the same way the fold canonicalizes a
    /// write, so a redundant re-link finds the one row it would collide with.
    pub(crate) fn canonical_edge(
        &self,
        from: MemoryId,
        to: MemoryId,
        relation: &RelationName,
    ) -> Result<(String, String, String), GraphError> {
        let from = from.0.to_string();
        let to = to.0.to_string();
        let label = relation.as_str();

        let stmt = self
            .conn
            .prepare("SELECT name, symmetric FROM relations WHERE name = ?1 OR inverse = ?1")?;
        let resolved = query_opt_into(stmt, params![label], |row| {
            Ok::<(String, i64), GraphError>(row.try_into()?)
        })?;

        Ok(match resolved {
            None => (from, to, label.to_owned()),
            Some((canonical, symmetric)) if symmetric != 0 => {
                let (lo, hi) = if from <= to { (from, to) } else { (to, from) };
                (lo, hi, canonical)
            }
            Some((canonical, _)) if label == canonical => (from, to, canonical),
            Some((canonical, _)) => (to, from, canonical),
        })
    }

    /// Recompute the denormalized `class_id` on every memory by union-find over the `same_as` edges,
    /// setting each class's id to its primary stub. Run on every `same_as` link change and on every
    /// primary designation: a merge unions two classes, an unmerge re-splits the component, and a whole
    /// recompute is correct for all three without a local patch (trivial at personal-agent class sizes).
    ///
    /// The primary is the component's **earliest member by ULID** (ULIDs sort chronologically), unless
    /// the operator has designated one — then it is the earliest-ULID **designated** member instead, so
    /// the operator's canonical stub wins over a throwaway that merely predates it. Both rules are
    /// order-independent: the whole component is examined, so the merge order that assembled it does not
    /// bear on the outcome. A component with two designations falls back to the earliest-ULID designated
    /// stub; a designation on a stub in another component has no bearing here, since only this
    /// component's members are ranked.
    pub(super) fn recompute_classes(&self) -> Result<(), GraphError> {
        // `(id, designated)` for every memory — the flag decides the primary, so it rides the read.
        let members: Vec<(String, bool)> = query_map_into(
            self.conn
                .prepare("SELECT id, designated_primary FROM memories")?,
            [],
            |row| {
                let (id, designated): (String, i64) = row.try_into()?;
                Ok::<_, GraphError>((id, designated != 0))
            },
        )?;
        let edges: Vec<(String, String)> = query_map_into(
            self.conn
                .prepare("SELECT from_id, to_id FROM links WHERE relation = ?1")?,
            params![RelationName::SameAs.as_str()],
            |row| Ok::<(String, String), GraphError>(row.try_into()?),
        )?;

        let mut parent: BTreeMap<String, String> = members
            .iter()
            .map(|(id, _)| (id.clone(), id.clone()))
            .collect();
        for (a, b) in &edges {
            let (ra, rb) = (find(&parent, a), find(&parent, b));
            if ra != rb {
                parent.insert(ra, rb);
            }
        }

        // Rank each member by `(is_not_designated, id)` and keep the minimum per component: a designated
        // member (rank `false`) beats an underived one (rank `true`), and ULID order breaks the tie —
        // yielding the earliest-ULID designated member, or the earliest member overall when none is.
        let mut primary: BTreeMap<String, (bool, &String)> = BTreeMap::new();
        for (id, designated) in &members {
            let root = find(&parent, id);
            let candidate = (!designated, id);
            primary
                .entry(root)
                .and_modify(|best| {
                    if candidate < *best {
                        *best = candidate;
                    }
                })
                .or_insert(candidate);
        }
        for (id, _) in &members {
            self.conn
                .execute(
                    "UPDATE memories SET class_id = ?1 WHERE id = ?2",
                    params![primary[&find(&parent, id)].1, id],
                )
                .map_err(backend)?;
        }
        Ok(())
    }
}

/// Union-find root of `x`, following parent pointers (no path compression — classes are tiny).
fn find(parent: &BTreeMap<String, String>, x: &str) -> String {
    let mut cur = x.to_owned();
    while let Some(next) = parent.get(&cur) {
        if *next == cur {
            break;
        }
        cur = next.clone();
    }
    cur
}
