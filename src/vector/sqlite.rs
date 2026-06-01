//! The real vector index: sqlite-vec's `vec0` virtual table with a text key and a cosine distance
//! metric. One fixed embedding dimensionality per index, established at creation.

use std::{path::Path, sync::Once};

use rusqlite::{Connection, OptionalExtension, ffi::sqlite3_auto_extension, params};
use sqlite_vec::sqlite3_vec_init;

use super::{ScoredHit, VectorError, VectorId, VectorIndex, VectorRecord};
use crate::ids::Seq;

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
        let mut statement = self
            .conn
            .prepare(
                "SELECT id, distance FROM vectors \
                 WHERE embedding MATCH ?1 AND k = ?2 ORDER BY distance",
            )
            .map_err(backend)?;
        let rows = statement
            .query_map(params![embedding, k as i64], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, f64>(1)?))
            })
            .map_err(backend)?;
        let mut hits = Vec::new();
        for row in rows {
            let (id, distance) = row.map_err(backend)?;
            // vec0 returns cosine *distance* in [0, 2]; convert back to similarity in [-1, 1].
            hits.push(ScoredHit {
                id: VectorId::new(id),
                score: 1.0 - distance as f32,
            });
        }
        Ok(hits)
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
