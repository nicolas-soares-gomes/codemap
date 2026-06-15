//! Tier0 query engine: resolve_symbol, get_file_outline, read_symbol. Compact projection,
//! callers/callees and inline staleness guard land in M2/M4.

use crate::db::{line_index, Db};
use crate::types::SymbolKind;
use anyhow::{anyhow, Result};
use rusqlite::OptionalExtension;
use std::path::Path;

/// Compact navigation row (no code).
#[derive(Debug, Clone)]
pub struct Hit {
    pub id: i64,
    pub name_path: String,
    pub file: String,
    pub line: u32,
    pub kind: Option<SymbolKind>,
}

pub fn resolve(db: &Db, query: &str, limit: i64) -> Result<Vec<Hit>> {
    let mut stmt = db.conn.prepare(
        "SELECT s.id, np.text, fp.text, s.start_line, s.kind
         FROM symbol s
         JOIN string_pool n  ON n.id  = s.name_sid
         JOIN string_pool np ON np.id = s.name_path_sid
         JOIN file f         ON f.id  = s.file_id
         JOIN string_pool fp ON fp.id = f.path_sid
         WHERE n.text = ?1 OR np.text = ?1 OR n.text LIKE '%' || ?1 || '%'
         ORDER BY (n.text = ?1 OR np.text = ?1) DESC, length(n.text) ASC
         LIMIT ?2",
    )?;
    let hits = stmt
        .query_map(rusqlite::params![query, limit], row_to_hit)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(hits)
}

pub fn outline(db: &Db, file: &str) -> Result<Vec<Hit>> {
    let mut stmt = db.conn.prepare(
        "SELECT s.id, np.text, fp.text, s.start_line, s.kind
         FROM symbol s
         JOIN string_pool np ON np.id = s.name_path_sid
         JOIN file f         ON f.id  = s.file_id
         JOIN string_pool fp ON fp.id = f.path_sid
         WHERE fp.text = ?1
         ORDER BY s.start_line",
    )?;
    let hits = stmt
        .query_map([file], row_to_hit)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(hits)
}

/// One symbol's code: reads only the range's byte span from disk, via line_index.
#[derive(Debug, Clone)]
pub struct Code {
    pub id: i64,
    pub name_path: String,
    pub file: String,
    pub start_line: u32,
    pub end_line: u32,
    pub code: String,
}

pub fn read_symbol(db: &Db, root: &Path, id: i64) -> Result<Code> {
    let row = db
        .conn
        .query_row(
            "SELECT np.text, fp.text, s.start_line, s.end_line, li.offsets
             FROM symbol s
             JOIN string_pool np ON np.id = s.name_path_sid
             JOIN file f         ON f.id  = s.file_id
             JOIN string_pool fp ON fp.id = f.path_sid
             JOIN line_index li  ON li.file_id = f.id
             WHERE s.id = ?1",
            [id],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, u32>(2)?,
                    r.get::<_, u32>(3)?,
                    r.get::<_, Vec<u8>>(4)?,
                ))
            },
        )
        .optional()?
        .ok_or_else(|| anyhow!("symbol {id} not found"))?;
    let (name_path, file, start_line, end_line, blob) = row;
    let bytes = std::fs::read(root.join(&file))
        .map_err(|e| anyhow!("read {file}: {e} (index may be stale — staleness guard lands in M4)"))?;
    let offsets = line_index::decode(&blob);
    let (b0, b1) = line_index::byte_span(&offsets, bytes.len() as u64, start_line, end_line);
    let code = String::from_utf8_lossy(&bytes[b0 as usize..b1 as usize]).into_owned();
    Ok(Code {
        id,
        name_path,
        file,
        start_line,
        end_line,
        code,
    })
}

fn row_to_hit(r: &rusqlite::Row) -> rusqlite::Result<Hit> {
    Ok(Hit {
        id: r.get(0)?,
        name_path: r.get(1)?,
        file: r.get(2)?,
        line: r.get(3)?,
        kind: SymbolKind::from_i64(r.get(4)?),
    })
}
