//! Tier0 query engine: resolve_symbol, get_file_outline, read_symbol. Compact projection,
//! callers/callees and inline staleness guard land in M2/M4.

use crate::db::{line_index, Db};
use crate::types::{Provenance, Resolution, SymbolKind};
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

/// Resolve a symbol argument (`sym:N`, `N`, or a name/name_path) to a symbol id.
pub fn resolve_arg(db: &Db, arg: &str) -> Result<i64> {
    if let Some(rest) = arg.strip_prefix("sym:") {
        if let Ok(id) = rest.parse::<i64>() {
            return Ok(id);
        }
    }
    if let Ok(id) = arg.parse::<i64>() {
        return Ok(id);
    }
    resolve(db, arg, 1)?
        .first()
        .map(|h| h.id)
        .ok_or_else(|| anyhow!("no symbol matches {arg:?}"))
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
    /// True if the file was reindexed inline because it had changed on disk.
    pub reindexed: bool,
}

struct SymRow {
    name_path: String,
    file: String,
    start_line: u32,
    end_line: u32,
    offsets: Vec<u8>,
    hash: Vec<u8>,
}

fn fetch_symbol(db: &Db, id: i64) -> Result<Option<SymRow>> {
    Ok(db
        .conn
        .query_row(
            "SELECT np.text, fp.text, s.start_line, s.end_line, li.offsets, f.content_hash
             FROM symbol s
             JOIN string_pool np ON np.id = s.name_path_sid
             JOIN file f         ON f.id  = s.file_id
             JOIN string_pool fp ON fp.id = f.path_sid
             JOIN line_index li  ON li.file_id = f.id
             WHERE s.id = ?1",
            [id],
            |r| {
                Ok(SymRow {
                    name_path: r.get(0)?,
                    file: r.get(1)?,
                    start_line: r.get(2)?,
                    end_line: r.get(3)?,
                    offsets: r.get(4)?,
                    hash: r.get(5)?,
                })
            },
        )
        .optional()?)
}

/// Staleness guard: validates the file's hash before serving. If the file changed on disk,
/// reindexes that file inline (ids stay stable via symbol_key) and serves fresh; if the symbol
/// no longer exists, returns a steering error instead of wrong code.
pub fn read_symbol(db: &mut Db, root: &Path, id: i64) -> Result<Code> {
    let mut row = fetch_symbol(db, id)?.ok_or_else(|| anyhow!("symbol {id} not found"))?;
    let mut bytes =
        std::fs::read(root.join(&row.file)).map_err(|e| anyhow!("read {}: {e}", row.file))?;

    let mut reindexed = false;
    if blake3::hash(&bytes).as_bytes()[..] != row.hash[..] {
        crate::index::reindex_file(db, root, &row.file)?;
        reindexed = true;
        row = fetch_symbol(db, id)?.ok_or_else(|| {
            anyhow!("symbol {id} no longer exists after reindex — re-resolve by name_path")
        })?;
        bytes =
            std::fs::read(root.join(&row.file)).map_err(|e| anyhow!("read {}: {e}", row.file))?;
    }

    let offsets = line_index::decode(&row.offsets);
    let (b0, b1) =
        line_index::byte_span(&offsets, bytes.len() as u64, row.start_line, row.end_line);
    let code = String::from_utf8_lossy(&bytes[b0 as usize..b1 as usize]).into_owned();
    Ok(Code {
        id,
        name_path: row.name_path,
        file: row.file,
        start_line: row.start_line,
        end_line: row.end_line,
        code,
        reindexed,
    })
}

/// A node reached by callers/callees traversal, with the edge's provenance/resolution.
#[derive(Debug, Clone)]
pub struct EdgeHit {
    pub id: i64,
    pub name_path: String,
    pub file: String,
    pub line: u32,
    pub kind: Option<SymbolKind>,
    pub depth: i64,
    pub provenance: Option<Provenance>,
    pub resolution: Option<Resolution>,
}

pub fn callees(db: &Db, root_id: i64, depth: i64, limit: i64) -> Result<Vec<EdgeHit>> {
    walk(db, root_id, depth, limit, true)
}

pub fn callers(db: &Db, root_id: i64, depth: i64, limit: i64) -> Result<Vec<EdgeHit>> {
    walk(db, root_id, depth, limit, false)
}

fn walk(db: &Db, root_id: i64, depth: i64, limit: i64, forward: bool) -> Result<Vec<EdgeHit>> {
    let (from_col, to_col) = if forward {
        ("source_symbol_id", "target_symbol_id")
    } else {
        ("target_symbol_id", "source_symbol_id")
    };
    let sql = format!(
        "WITH RECURSIVE walk(sym, depth, prov, res, path) AS (
             SELECT ?1, 0, -1, -1, ',' || ?1 || ','
           UNION ALL
             SELECT e.{to_col}, w.depth + 1, e.provenance, e.resolution, w.path || e.{to_col} || ','
             FROM edge e JOIN walk w ON e.{from_col} = w.sym
             WHERE e.kind = 1 AND w.depth < ?2 AND instr(w.path, ',' || e.{to_col} || ',') = 0
         ),
         best AS (
             SELECT sym, depth, prov, res, ROW_NUMBER() OVER (PARTITION BY sym ORDER BY depth) rn
             FROM walk WHERE depth > 0
         )
         SELECT b.sym, np.text, fp.text, s.start_line, s.kind, b.depth, b.prov, b.res
         FROM best b
         JOIN symbol s       ON s.id  = b.sym
         JOIN string_pool np ON np.id = s.name_path_sid
         JOIN file f         ON f.id  = s.file_id
         JOIN string_pool fp ON fp.id = f.path_sid
         WHERE b.rn = 1
         ORDER BY b.depth, np.text
         LIMIT ?3"
    );
    let mut stmt = db.conn.prepare(&sql)?;
    let rows = stmt
        .query_map(rusqlite::params![root_id, depth, limit], |r| {
            Ok(EdgeHit {
                id: r.get(0)?,
                name_path: r.get(1)?,
                file: r.get(2)?,
                line: r.get(3)?,
                kind: SymbolKind::from_i64(r.get(4)?),
                depth: r.get(5)?,
                provenance: Provenance::from_i64(r.get(6)?),
                resolution: Resolution::from_i64(r.get(7)?),
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// A bounded subgraph around a root symbol, for export.
#[derive(Debug, Default)]
pub struct Subgraph {
    pub root: i64,
    pub nodes: Vec<(i64, String)>, // (id, name_path)
    pub edges: Vec<(i64, i64, Option<Provenance>, Option<Resolution>)>,
}

pub fn subgraph(db: &Db, root_id: i64, depth: i64, forward: bool) -> Result<Subgraph> {
    let walked = if forward {
        callees(db, root_id, depth, 1000)?
    } else {
        callers(db, root_id, depth, 1000)?
    };
    let mut ids: std::collections::HashSet<i64> = walked.iter().map(|h| h.id).collect();
    ids.insert(root_id);

    let mut nodes = Vec::new();
    let mut label = db.conn.prepare(
        "SELECT np.text FROM symbol s JOIN string_pool np ON np.id=s.name_path_sid WHERE s.id=?1",
    )?;
    for &id in &ids {
        if let Some(name) = label
            .query_row([id], |r| r.get::<_, String>(0))
            .optional()?
        {
            nodes.push((id, name));
        }
    }
    nodes.sort();

    let mut edges = Vec::new();
    let mut estmt = db
        .conn
        .prepare("SELECT target_symbol_id, provenance, resolution FROM edge WHERE source_symbol_id=?1 AND kind=1")?;
    for &src in &ids {
        let rows = estmt
            .query_map([src], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, i64>(2)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for (tgt, prov, res) in rows {
            if ids.contains(&tgt) {
                edges.push((
                    src,
                    tgt,
                    Provenance::from_i64(prov),
                    Resolution::from_i64(res),
                ));
            }
        }
    }
    edges.sort_by_key(|e| (e.0, e.1));
    Ok(Subgraph {
        root: root_id,
        nodes,
        edges,
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
