//! Ingest a SCIP index produced by an external indexer (e.g. `rust-analyzer scip`,
//! `scip-typescript`). codemap never generates it — it only reads the `.scip` file.
//!
//! SCIP records occurrences, not a call graph, so call edges are derived by intersecting
//! reference occurrences with the enclosing-symbol ranges from the tree-sitter pass. Derived
//! edges are provenance=scip, resolution=resolved; for covered files they replace the
//! tree-sitter (ambiguous) call edges.

use crate::db::{writer, Db};
use crate::types::{EdgeKind, Provenance, Resolution, SymbolKind};
use anyhow::{Context, Result};
use protobuf::Message;
use rusqlite::{params, OptionalExtension};
use std::collections::{HashMap, HashSet};
use std::path::Path;

const ROLE_DEFINITION: i32 = 1; // SymbolRole::Definition bit

#[derive(Debug, Default)]
pub struct ScipStats {
    pub documents: usize,
    pub covered_files: usize,
    pub total_files: usize,
    pub edges: u64,
}

impl ScipStats {
    /// Percentage of indexed files the SCIP index covers (0 if none indexed).
    pub fn coverage_pct(&self) -> u32 {
        (self.covered_files * 100)
            .checked_div(self.total_files)
            .unwrap_or(0) as u32
    }
}

pub fn ingest(db: &mut Db, scip_path: &Path) -> Result<ScipStats> {
    let bytes =
        std::fs::read(scip_path).with_context(|| format!("read {}", scip_path.display()))?;
    let index = ::scip::types::Index::parse_from_bytes(&bytes).context("parse SCIP index")?;

    let mut stats = ScipStats::default();
    let tx = db.conn.transaction()?;
    // Idempotency: drop previously-derived SCIP edges.
    tx.execute(
        "DELETE FROM edge WHERE provenance=?1",
        [Provenance::Scip.as_i64()],
    )?;

    // Pass A: map each SCIP symbol string to our symbol id via (file, definition line).
    let mut sym_map: HashMap<String, (i64, i64)> = HashMap::new(); // scip_symbol -> (id, kind)
    let mut covered: HashSet<i64> = HashSet::new();
    for doc in &index.documents {
        stats.documents += 1;
        let Some(file_id) = file_id_by_path(&tx, &doc.relative_path)? else {
            continue;
        };
        covered.insert(file_id);
        let by_sel = symbols_by_sel_line(&tx, file_id)?;
        for occ in &doc.occurrences {
            if occ.symbol_roles & ROLE_DEFINITION == 0 {
                continue;
            }
            let Some(&line0) = occ.range.first() else {
                continue;
            };
            if let Some(&(sid, kind)) = by_sel.get(&((line0 as u32) + 1)) {
                let sym_sid = writer::intern(&tx, &occ.symbol)?;
                tx.execute(
                    "UPDATE symbol SET scip_sym_sid=?2 WHERE id=?1",
                    params![sid, sym_sid],
                )?;
                sym_map.insert(occ.symbol.clone(), (sid, kind));
            }
        }
    }

    // For covered files, SCIP is authoritative: drop the syntactic tree-sitter call edges.
    for &fid in &covered {
        tx.execute(
            "DELETE FROM edge WHERE kind=?1 AND provenance!=?2
             AND source_symbol_id IN (SELECT id FROM symbol WHERE file_id=?3)",
            params![EdgeKind::Calls.as_i64(), Provenance::Scip.as_i64(), fid],
        )?;
    }

    // Pass B: reference occurrences to a function/method become resolved call edges.
    for doc in &index.documents {
        let Some(file_id) = file_id_by_path(&tx, &doc.relative_path)? else {
            continue;
        };
        let callables = callables_in_file(&tx, file_id)?;
        for occ in &doc.occurrences {
            if occ.symbol_roles & ROLE_DEFINITION != 0 {
                continue;
            }
            let Some(&line0) = occ.range.first() else {
                continue;
            };
            let Some(&(callee, kind)) = sym_map.get(&occ.symbol) else {
                continue;
            };
            if kind != SymbolKind::Function.as_i64() && kind != SymbolKind::Method.as_i64() {
                continue; // only function/method references are "calls"
            }
            let Some(caller) = innermost_callable(&callables, (line0 as u32) + 1) else {
                continue;
            };
            stats.edges += tx.execute(
                "INSERT OR IGNORE INTO edge(source_symbol_id,target_symbol_id,kind,provenance,resolution)
                 VALUES (?1,?2,?3,?4,?5)",
                params![caller, callee, EdgeKind::Calls.as_i64(), Provenance::Scip.as_i64(), Resolution::Resolved.as_i64()],
            )? as u64;
        }
    }

    stats.covered_files = covered.len();
    stats.total_files =
        tx.query_row("SELECT count(*) FROM file", [], |r| r.get::<_, i64>(0))? as usize;
    tx.commit()?;
    db.set_meta("scip_at", &scip_path.to_string_lossy())?;
    db.set_meta(
        "scip_coverage",
        &format!(
            "{}/{} ({}%)",
            stats.covered_files,
            stats.total_files,
            stats.coverage_pct()
        ),
    )?;
    Ok(stats)
}

fn file_id_by_path(conn: &rusqlite::Connection, path: &str) -> Result<Option<i64>> {
    Ok(conn
        .query_row(
            "SELECT f.id FROM file f JOIN string_pool s ON s.id=f.path_sid WHERE s.text=?1",
            [path],
            |r| r.get(0),
        )
        .optional()?)
}

fn symbols_by_sel_line(
    conn: &rusqlite::Connection,
    file_id: i64,
) -> Result<HashMap<u32, (i64, i64)>> {
    let mut stmt = conn.prepare("SELECT sel_line, id, kind FROM symbol WHERE file_id=?1")?;
    let rows = stmt
        .query_map([file_id], |r| {
            Ok((
                r.get::<_, u32>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, i64>(2)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows.into_iter().map(|(l, id, k)| (l, (id, k))).collect())
}

fn callables_in_file(
    conn: &rusqlite::Connection,
    file_id: i64,
) -> Result<Vec<(i64, u32, u32, i64)>> {
    let mut stmt =
        conn.prepare("SELECT id,start_line,end_line,kind FROM symbol WHERE file_id=?1")?;
    let rows = stmt
        .query_map([file_id], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

fn innermost_callable(callables: &[(i64, u32, u32, i64)], line: u32) -> Option<i64> {
    callables
        .iter()
        .filter(|(_, s, e, k)| {
            *s <= line
                && line <= *e
                && (*k == SymbolKind::Function.as_i64() || *k == SymbolKind::Method.as_i64())
        })
        .min_by_key(|(_, s, e, _)| e - s)
        .map(|(id, _, _, _)| *id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ::scip::types::{Document, Index, Occurrence};
    use rusqlite::params;
    use std::io::Write;

    fn intern(db: &Db, s: &str) -> i64 {
        db.conn
            .execute("INSERT OR IGNORE INTO string_pool(text) VALUES (?1)", [s])
            .unwrap();
        db.conn
            .query_row("SELECT id FROM string_pool WHERE text=?1", [s], |r| {
                r.get(0)
            })
            .unwrap()
    }

    fn add_symbol(db: &Db, id: i64, file_id: i64, name: &str, kind: SymbolKind, line: u32) {
        let n = intern(db, name);
        let key = blake3::hash(format!("{file_id}:{name}:{id}").as_bytes());
        db.conn
            .execute(
                "INSERT INTO symbol(id,symbol_key,file_id,name_sid,name_path_sid,kind,
                                    start_line,start_col,end_line,end_col,sel_line,sel_col)
                 VALUES (?1,?2,?3,?4,?4,?5,?6,0,?6,0,?6,0)",
                params![id, key.as_bytes().to_vec(), file_id, n, kind.as_i64(), line],
            )
            .unwrap();
    }

    fn occ(symbol: &str, line: i32, role: i32) -> Occurrence {
        let mut o = Occurrence::new();
        o.range = vec![line, 3, 9];
        o.symbol = symbol.into();
        o.symbol_roles = role;
        o
    }

    #[test]
    fn ingest_derives_resolved_call_edge() {
        let mut db = Db::open_in_memory().unwrap();
        let path_sid = intern(&db, "src/g.rs");
        db.conn
            .execute(
                "INSERT INTO file(id,path_sid,lang,size,mtime_ns,content_hash,line_count,indexed_at,tier)
                 VALUES (1,?1,4,0,0,X'00',3,0,0)",
                [path_sid],
            )
            .unwrap();
        // callee on line 1, caller on line 2 (range covers the call on line 2).
        add_symbol(&db, 10, 1, "callee", SymbolKind::Function, 1);
        add_symbol(&db, 11, 1, "caller", SymbolKind::Function, 2);

        let mut doc = Document::new();
        doc.relative_path = "src/g.rs".into();
        doc.occurrences = vec![
            occ("rust::callee#", 0, ROLE_DEFINITION),
            occ("rust::caller#", 1, ROLE_DEFINITION),
            occ("rust::callee#", 1, 0), // reference to callee inside caller
        ];
        let mut idx = Index::new();
        idx.documents = vec![doc];

        let mut f = tempfile::NamedTempFile::new().unwrap();
        f.write_all(&idx.write_to_bytes().unwrap()).unwrap();

        let stats = ingest(&mut db, f.path()).unwrap();
        assert_eq!(stats.covered_files, 1);
        assert!(stats.edges >= 1);

        let (prov, res): (i64, i64) = db
            .conn
            .query_row(
                "SELECT provenance, resolution FROM edge WHERE source_symbol_id=11 AND target_symbol_id=10 AND kind=1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(prov, Provenance::Scip.as_i64());
        assert_eq!(res, Resolution::Resolved.as_i64());
    }
}
