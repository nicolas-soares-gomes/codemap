//! Storage layer: open SQLite, apply pragmas, run migrations, low-level traversal.

use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension};
use std::path::Path;

pub mod line_index;
pub mod writer;

const MIGRATION_0001: &str = include_str!("../../migrations/0001_init.sql");

pub const SCHEMA_VERSION: i64 = 1;

pub struct Db {
    pub conn: Connection,
}

impl Db {
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path).with_context(|| format!("open db {}", path.display()))?;
        let mut db = Self { conn };
        db.apply_pragmas(true)?;
        db.migrate()?;
        Ok(db)
    }

    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        let mut db = Self { conn };
        db.apply_pragmas(false)?;
        db.migrate()?;
        Ok(db)
    }

    fn apply_pragmas(&self, on_disk: bool) -> Result<()> {
        let mut sql = String::from(
            "PRAGMA synchronous=NORMAL; \
             PRAGMA foreign_keys=ON; \
             PRAGMA cache_size=-65536; \
             PRAGMA temp_store=MEMORY; \
             PRAGMA busy_timeout=5000;",
        );
        if on_disk {
            sql.push_str(" PRAGMA journal_mode=WAL; PRAGMA mmap_size=268435456;");
        }
        self.conn.execute_batch(&sql).context("apply pragmas")?;
        Ok(())
    }

    fn migrate(&mut self) -> Result<()> {
        let current: i64 = self
            .conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap_or(0);
        if current < 1 {
            self.conn
                .execute_batch(MIGRATION_0001)
                .context("apply migration 0001")?;
            self.conn.execute(
                "INSERT OR REPLACE INTO meta(key,value) VALUES ('schema_version', ?1)",
                [SCHEMA_VERSION.to_string()],
            )?;
            self.conn
                .pragma_update(None, "user_version", SCHEMA_VERSION)?;
        }
        Ok(())
    }

    pub fn set_meta(&self, key: &str, value: &str) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO meta(key,value) VALUES (?1,?2)",
            rusqlite::params![key, value],
        )?;
        Ok(())
    }

    pub fn get_meta(&self, key: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row("SELECT value FROM meta WHERE key=?1", [key], |r| r.get(0))
            .optional()?)
    }

    /// Forward traversal (callees): cycle-safe recursive CTE with depth cap and total limit.
    pub fn callees(
        &self,
        root: i64,
        kind: i64,
        max_depth: i64,
        limit: i64,
    ) -> Result<Vec<(i64, i64)>> {
        self.traverse(root, kind, max_depth, limit, Direction::Forward)
    }

    /// Reverse traversal (callers).
    pub fn callers(
        &self,
        root: i64,
        kind: i64,
        max_depth: i64,
        limit: i64,
    ) -> Result<Vec<(i64, i64)>> {
        self.traverse(root, kind, max_depth, limit, Direction::Reverse)
    }

    fn traverse(
        &self,
        root: i64,
        kind: i64,
        max_depth: i64,
        limit: i64,
        dir: Direction,
    ) -> Result<Vec<(i64, i64)>> {
        let (from_col, to_col) = match dir {
            Direction::Forward => ("source_symbol_id", "target_symbol_id"),
            Direction::Reverse => ("target_symbol_id", "source_symbol_id"),
        };
        let sql = format!(
            "WITH RECURSIVE walk(sym, depth, path) AS (
                 SELECT ?1, 0, ',' || ?1 || ','
               UNION ALL
                 SELECT e.{to_col}, w.depth + 1, w.path || e.{to_col} || ','
                 FROM edge e JOIN walk w ON e.{from_col} = w.sym
                 WHERE e.kind = ?2
                   AND w.depth < ?3
                   AND instr(w.path, ',' || e.{to_col} || ',') = 0
             )
             SELECT sym, MIN(depth) AS depth FROM walk WHERE depth > 0
             GROUP BY sym ORDER BY depth, sym LIMIT ?4"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt
            .query_map(rusqlite::params![root, kind, max_depth, limit], |r| {
                Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }
}

enum Direction {
    Forward,
    Reverse,
}
