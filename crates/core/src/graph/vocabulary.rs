//! Vocabulary reads: tags and relations.

use crate::{
    db::{query_map_into, query_opt_into},
    graph::{
        Graph, GraphError, RelationView, TagVocabularyEntry, backend, entries::parse_cardinality,
    },
    vocabulary::{RelationName, TagName},
};
use rusqlite::{OptionalExtension, params};

impl Graph {
    /// A tag's description, or `None` if the tag was never created.
    pub fn tag_description(&self, name: &str) -> Result<Option<String>, GraphError> {
        self.conn
            .query_row(
                "SELECT description FROM tags WHERE name = ?1",
                params![name],
                |r| r.get(0),
            )
            .optional()
            .map_err(backend)
    }

    /// The whole tag vocabulary: every created tag with its one-line purpose and how many live
    /// memories carry it, ordered by name. Backs `tags.list` and the system prompt's tag-vocabulary
    /// block. The count joins only undeleted memories, so a tag applied solely to soft-deleted
    /// memories reads as zero uses, consistent with every other agent-facing read.
    pub fn all_tags(&self) -> Result<Vec<TagVocabularyEntry>, GraphError> {
        let stmt = self.conn.prepare(
            "SELECT t.name, t.description, COUNT(m.id) AS count
             FROM tags t
             LEFT JOIN memory_tags mt ON mt.tag = t.name
             LEFT JOIN memories m ON m.id = mt.memory_id AND m.deleted = 0
             GROUP BY t.name, t.description
             ORDER BY t.name",
        )?;
        query_map_into(stmt, [], |row| {
            let name: String = row.get("name")?;
            let description: String = row.get("description")?;
            let count: i64 = row.get("count")?;
            Ok(TagVocabularyEntry {
                name: TagName::new(&name),
                description,
                count: count as usize,
            })
        })
    }

    /// A registered relation by either of its labels (canonical or inverse), or `None`. Resolving the
    /// inverse label too is what lets a relation be used under either name (spec §Data model: one
    /// relation, two labels) — both at `links.get` and when validating a `mem:link` asserted under the
    /// inverse label.
    pub fn relation(&self, name: &str) -> Result<Option<RelationView>, GraphError> {
        let stmt = self.conn.prepare(
            "SELECT name, inverse, from_card, to_card, symmetric, reflexive, description
             FROM relations WHERE name = ?1 OR inverse = ?1",
        )?;
        query_opt_into(stmt, params![name], |row| {
            let name: String = row.get("name")?;
            let inverse: String = row.get("inverse")?;
            let from_card: String = row.get("from_card")?;
            let to_card: String = row.get("to_card")?;
            let symmetric: i64 = row.get("symmetric")?;
            let reflexive: i64 = row.get("reflexive")?;
            let description: String = row.get("description")?;
            Ok(RelationView {
                name: RelationName::new(&name),
                inverse: RelationName::new(&inverse),
                from_card: parse_cardinality(&from_card)?,
                to_card: parse_cardinality(&to_card)?,
                symmetric: symmetric != 0,
                reflexive: reflexive != 0,
                description,
            })
        })
    }

    /// Every registered relation, ordered by canonical name. Backs `links.list` and the system
    /// prompt's relation-registry block.
    pub fn all_relations(&self) -> Result<Vec<RelationView>, GraphError> {
        let stmt = self.conn.prepare(
            "SELECT name, inverse, from_card, to_card, symmetric, reflexive, description
             FROM relations ORDER BY name",
        )?;
        query_map_into(stmt, [], |row| {
            let name: String = row.get("name")?;
            let inverse: String = row.get("inverse")?;
            let from_card: String = row.get("from_card")?;
            let to_card: String = row.get("to_card")?;
            let symmetric: i64 = row.get("symmetric")?;
            let reflexive: i64 = row.get("reflexive")?;
            let description: String = row.get("description")?;
            Ok(RelationView {
                name: RelationName::new(&name),
                inverse: RelationName::new(&inverse),
                from_card: parse_cardinality(&from_card)?,
                to_card: parse_cardinality(&to_card)?,
                symmetric: symmetric != 0,
                reflexive: reflexive != 0,
                description,
            })
        })
    }
}
