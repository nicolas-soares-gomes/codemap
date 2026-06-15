//! Low-level write ops, run inside an indexer transaction. They take `&Connection` so
//! they work with both `Connection` and `Transaction`.

use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};

pub fn intern(conn: &Connection, text: &str) -> Result<i64> {
    conn.execute("INSERT OR IGNORE INTO string_pool(text) VALUES (?1)", [text])?;
    Ok(conn.query_row("SELECT id FROM string_pool WHERE text=?1", [text], |r| r.get(0))?)
}

/// Remove a file and its symbols. Manually syncs contentless FTS5: emits 'delete' with the
/// OLD values before deleting the file (CASCADE does not clean the FTS index).
pub fn prune_file(conn: &Connection, path: &str) -> Result<()> {
    let fid: Option<i64> = conn
        .query_row(
            "SELECT f.id FROM file f JOIN string_pool s ON s.id=f.path_sid WHERE s.text=?1",
            [path],
            |r| r.get(0),
        )
        .optional()?;
    if let Some(fid) = fid {
        {
            let mut stmt = conn.prepare(
                "SELECT s.id, n.text, np.text FROM symbol s
                 JOIN string_pool n  ON n.id  = s.name_sid
                 JOIN string_pool np ON np.id = s.name_path_sid
                 WHERE s.file_id = ?1",
            )?;
            let rows: Vec<(i64, String, String)> = stmt
                .query_map([fid], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
                .collect::<rusqlite::Result<_>>()?;
            let mut del = conn.prepare(
                "INSERT INTO symbol_fts(symbol_fts,rowid,name,name_path) VALUES('delete',?1,?2,?3)",
            )?;
            for (id, n, np) in rows {
                del.execute(params![id, n, np])?;
            }
        }
        conn.execute("DELETE FROM file WHERE id=?1", [fid])?;
    }
    Ok(())
}
