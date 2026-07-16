//! The real vector index: sqlite-vec's `vec0` virtual table with a text key and a cosine distance
//! metric. One fixed embedding dimensionality per index, established at creation.

use std::{path::Path, sync::Once};

use rusqlite::{Connection, OptionalExtension, ffi::sqlite3_auto_extension, params};
use smol_str::SmolStr;
use sqlite_vec::sqlite3_vec_init;

use crate::{
    db::query_map_into,
    ids::Seq,
    vector::{ScoredHit, VectorError, VectorId, VectorIndex, VectorRecord},
};

pub struct SqliteVectorIndex {
    conn: Connection,
    dimensions: usize,
}

impl SqliteVectorIndex {
    /// Open an ephemeral in-memory index — what the fast-lane tests use.
    pub fn open_in_memory(dimensions: usize) -> Result<SqliteVectorIndex, VectorError> {
        register_sqlite_vec();
        let conn = Connection::open_in_memory().map_err(backend)?;
        SqliteVectorIndex::init(conn, dimensions)
    }

    /// Open (creating if absent) a file-backed index at `path`.
    pub fn open(
        path: impl AsRef<Path>,
        dimensions: usize,
    ) -> Result<SqliteVectorIndex, VectorError> {
        register_sqlite_vec();
        let conn = Connection::open(path).map_err(backend)?;
        SqliteVectorIndex::init(conn, dimensions)
    }

    fn init(conn: Connection, dimensions: usize) -> Result<SqliteVectorIndex, VectorError> {
        conn.execute_batch(&format!(
            "CREATE VIRTUAL TABLE IF NOT EXISTS vectors USING vec0(\
                 id TEXT PRIMARY KEY, embedding float[{dimensions}] distance_metric=cosine, \
                 +model_id TEXT);\
             CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value INTEGER NOT NULL);"
        ))
        .map_err(backend)?;
        Ok(SqliteVectorIndex { conn, dimensions })
    }

    fn require_dimensions(&self, vector: &[f32]) -> Result<(), VectorError> {
        if vector.len() != self.dimensions {
            return Err(VectorError::Dimension {
                expected: self.dimensions,
                found: vector.len(),
            });
        }
        Ok(())
    }
}

impl VectorIndex for SqliteVectorIndex {
    fn upsert(&mut self, record: VectorRecord) -> Result<(), VectorError> {
        self.require_dimensions(&record.embedding)?;
        let embedding = serde_json::to_string(&record.embedding).map_err(backend)?;
        // vec0 has no in-place update, so replace is delete-then-insert, atomic in one transaction.
        let tx = self.conn.transaction().map_err(backend)?;
        tx.execute(
            "DELETE FROM vectors WHERE id = ?1",
            params![record.id.0.as_str()],
        )
        .map_err(backend)?;
        tx.execute(
            "INSERT INTO vectors (id, embedding, model_id) VALUES (?1, ?2, ?3)",
            params![record.id.0.as_str(), embedding, record.model_id.as_str()],
        )
        .map_err(backend)?;
        tx.commit().map_err(backend)?;
        Ok(())
    }

    fn remove(&mut self, id: &VectorId) -> Result<(), VectorError> {
        self.conn
            .execute("DELETE FROM vectors WHERE id = ?1", params![id.0.as_str()])
            .map_err(backend)?;
        Ok(())
    }

    fn len(&self) -> Result<usize, VectorError> {
        let count: i64 = self
            .conn
            .query_row("SELECT count(*) FROM vectors", [], |row| row.get(0))
            .map_err(backend)?;
        Ok(count as usize)
    }

    fn search(&self, query: &[f32], k: usize) -> Result<Vec<ScoredHit>, VectorError> {
        self.require_dimensions(query)?;
        if k == 0 {
            return Ok(Vec::new());
        }
        let embedding = serde_json::to_string(query).map_err(backend)?;
        let statement = self.conn.prepare(
            "SELECT id, distance FROM vectors \
             WHERE embedding MATCH ?1 AND k = ?2 ORDER BY distance",
        )?;
        query_map_into(statement, params![embedding, k as i64], |row| {
            let (id, distance): (String, f64) = row.try_into()?;
            // vec0 returns cosine *distance* in [0, 2]; convert back to similarity in [-1, 1].
            Ok(ScoredHit {
                id: VectorId::new(id),
                score: 1.0 - distance as f32,
            })
        })
    }

    fn cursor(&self) -> Result<Seq, VectorError> {
        let value: Option<i64> = self
            .conn
            .query_row("SELECT value FROM meta WHERE key = 'cursor'", [], |row| {
                row.get(0)
            })
            .optional()
            .map_err(backend)?;
        Ok(Seq(value.unwrap_or(0) as u64))
    }

    fn set_cursor(&mut self, seq: Seq) -> Result<(), VectorError> {
        self.conn
            .execute(
                "INSERT INTO meta (key, value) VALUES ('cursor', ?1) \
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![seq.0 as i64],
            )
            .map_err(backend)?;
        Ok(())
    }

    fn model_id(&self) -> Result<Option<SmolStr>, VectorError> {
        let model_id: Option<String> = self
            .conn
            .query_row("SELECT model_id FROM vectors LIMIT 1", [], |row| row.get(0))
            .optional()
            .map_err(backend)?;
        Ok(model_id.map(SmolStr::from))
    }

    fn clear(&mut self) -> Result<(), VectorError> {
        let tx = self.conn.transaction().map_err(backend)?;
        tx.execute("DELETE FROM vectors", []).map_err(backend)?;
        tx.execute("DELETE FROM meta WHERE key = 'cursor'", [])
            .map_err(backend)?;
        tx.commit().map_err(backend)?;
        Ok(())
    }
}

/// Register sqlite-vec as an auto-extension so every connection opened afterwards loads it. The
/// transmute is the crate's documented bridge to rusqlite's auto-extension hook.
fn register_sqlite_vec() {
    static REGISTER: Once = Once::new();
    REGISTER.call_once(|| unsafe {
        #[allow(clippy::missing_transmute_annotations)]
        sqlite3_auto_extension(Some(std::mem::transmute(sqlite3_vec_init as *const ())));
    });
}

fn backend(error: impl std::fmt::Display) -> VectorError {
    VectorError::Backend(error.to_string())
}

#[cfg(test)]
mod tests {
    //! The sqlite-vec backend: nearest-neighbour ranking, upsert-replaces semantics, and the
    //! dimension-mismatch guard. Uses the deterministic fake embedder so no model is needed.
    use crate::{
        ids::Seq,
        model::embed::{Embedder, FakeEmbedder},
        vector::{SqliteVectorIndex, VectorError, VectorId, VectorIndex, VectorRecord},
    };

    const DIMS: usize = 32;

    async fn vector(embedder: &FakeEmbedder, text: &str) -> Vec<f32> {
        embedder.embed(&[text.to_owned()]).await.unwrap().remove(0)
    }

    async fn record(embedder: &FakeEmbedder, id: &str, text: &str) -> VectorRecord {
        VectorRecord {
            id: VectorId::new(id),
            embedding: vector(embedder, text).await,
            model_id: embedder.model_id().into(),
        }
    }

    #[tokio::test]
    async fn ranks_nearest_first_and_replaces_on_reinsert() {
        let embedder = FakeEmbedder::new(DIMS);
        let mut index = SqliteVectorIndex::open_in_memory(DIMS).unwrap();
        assert!(index.is_empty().unwrap());

        for text in ["climbing gym", "sourdough bread", "tax return"] {
            index.upsert(record(&embedder, text, text).await).unwrap();
        }
        assert_eq!(index.len().unwrap(), 3);

        let query = vector(&embedder, "climbing gym").await;
        let hits = index.search(&query, 2).unwrap();
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].id, VectorId::new("climbing gym"));
        // The exact match is cosine similarity ~1 (within f32 round-trip error through the JSON binding).
        assert!((hits[0].score - 1.0).abs() < 1e-3);
        assert!(hits[0].score >= hits[1].score);

        // Re-inserting an existing id replaces rather than duplicates.
        index
            .upsert(record(&embedder, "climbing gym", "climbing gym").await)
            .unwrap();
        assert_eq!(index.len().unwrap(), 3);

        index.remove(&VectorId::new("climbing gym")).unwrap();
        assert_eq!(index.len().unwrap(), 2);
        assert!(
            index
                .search(&query, 5)
                .unwrap()
                .iter()
                .all(|hit| hit.id != VectorId::new("climbing gym"))
        );
    }

    #[tokio::test]
    async fn rejects_a_wrong_dimensioned_embedding() {
        let mut index = SqliteVectorIndex::open_in_memory(DIMS).unwrap();
        let wrong = vec![0.0_f32; DIMS + 1];
        let record = VectorRecord {
            id: VectorId::new("x"),
            embedding: wrong.clone(),
            model_id: "fake-embedder".into(),
        };
        assert!(matches!(
            index.upsert(record),
            Err(VectorError::Dimension {
                expected: DIMS,
                found,
            }) if found == DIMS + 1
        ));
        assert!(matches!(
            index.search(&wrong, 3),
            Err(VectorError::Dimension { .. })
        ));
    }

    #[tokio::test]
    async fn model_id_reads_back_the_stored_model_and_clear_resets_the_index() {
        // The boot-time embedding-swap detection turns on these two against the live vec0 backend: the
        // model the stored vectors carry, and clearing them for a re-embed (spec §Storage → vector store).
        let embedder = FakeEmbedder::new(DIMS);
        let mut index = SqliteVectorIndex::open_in_memory(DIMS).unwrap();

        // An empty index identifies no model and sits at the start of the log.
        assert_eq!(index.model_id().unwrap(), None);
        assert_eq!(index.cursor().unwrap(), Seq::ZERO);

        // Populated under a model, with the cursor advanced: `model_id` reads the tag back off the vec0
        // partition column, and the cursor persists in the meta table.
        index
            .upsert(VectorRecord {
                id: VectorId::new("entry:a"),
                embedding: vector(&embedder, "a").await,
                model_id: "jina-v5".into(),
            })
            .unwrap();
        index.set_cursor(Seq(7)).unwrap();
        assert_eq!(index.model_id().unwrap().as_deref(), Some("jina-v5"));
        assert_eq!(index.cursor().unwrap(), Seq(7));

        // `clear` drops every vector and resets the cursor, so a rebuild re-embeds the whole log.
        index.clear().unwrap();
        assert!(index.is_empty().unwrap());
        assert_eq!(index.model_id().unwrap(), None);
        assert_eq!(index.cursor().unwrap(), Seq::ZERO);

        // And the cleared index repopulates cleanly under a new model.
        index
            .upsert(VectorRecord {
                id: VectorId::new("entry:b"),
                embedding: vector(&embedder, "b").await,
                model_id: "bge-small".into(),
            })
            .unwrap();
        assert_eq!(index.model_id().unwrap().as_deref(), Some("bge-small"));
    }
}
