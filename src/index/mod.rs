//! Tier0 full-scan indexing. Walks honoring .gitignore + .codemapignore, parses with
//! tree-sitter, and reconciles each file by `symbol_key` so symbol ids stay stable across
//! reindex. Incremental (git/mtime) and parallelism (rayon) land in M4.

use crate::db::{line_index, writer, Db};
use crate::ts;
use crate::types::Language;
use anyhow::{Context, Result};
use ignore::WalkBuilder;
use rusqlite::{params, OptionalExtension};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_FILE: usize = 2 * 1024 * 1024;

#[derive(Default, Debug)]
pub struct IndexStats {
    pub files: u64,
    pub symbols: u64,
}

pub fn detect_lang(path: &Path) -> Option<Language> {
    match path.extension().and_then(|e| e.to_str()) {
        Some("rs") => Some(Language::Rust),
        _ => None,
    }
}

pub fn index_full(db: &mut Db, root: &Path) -> Result<IndexStats> {
    let mut stats = IndexStats::default();
    for entry in WalkBuilder::new(root)
        .add_custom_ignore_filename(".codemapignore")
        .build()
    {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        if !entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
            continue;
        }
        let path = entry.path();
        let Some(lang) = detect_lang(path) else {
            continue;
        };
        let bytes = match std::fs::read(path) {
            Ok(b) => b,
            Err(_) => continue,
        };
        if bytes.len() > MAX_FILE || bytes.iter().take(8192).any(|b| *b == 0) {
            continue;
        }
        let mtime_ns = entry
            .metadata()
            .ok()
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_nanos() as i64)
            .unwrap_or(0);
        let rel = path
            .strip_prefix(root)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/");
        index_one(db, &rel, lang, &bytes, mtime_ns, &mut stats)
            .with_context(|| format!("index {rel}"))?;
    }
    db.set_meta("scanner_mode", "fs")?;
    Ok(stats)
}

struct ExistingSymbol {
    id: i64,
    name: String,
    name_path: String,
}

fn index_one(
    db: &mut Db,
    rel: &str,
    lang: Language,
    bytes: &[u8],
    mtime_ns: i64,
    stats: &mut IndexStats,
) -> Result<()> {
    let source = String::from_utf8_lossy(bytes);
    let extracted = ts::extract(lang, &source);
    let offsets = line_index::compute_offsets(bytes);
    let line_count = offsets.len() as i64;
    let hash = blake3::hash(bytes);
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);

    let tx = db.conn.transaction()?;
    let path_sid = writer::intern(&tx, rel)?;

    // Upsert the file row, keeping its id stable.
    let file_id: Option<i64> = tx
        .query_row("SELECT id FROM file WHERE path_sid=?1", [path_sid], |r| r.get(0))
        .optional()?;
    let file_id = match file_id {
        Some(fid) => {
            tx.execute(
                "UPDATE file SET lang=?2,size=?3,mtime_ns=?4,content_hash=?5,line_count=?6,indexed_at=?7
                 WHERE id=?1",
                params![fid, lang.as_i64(), bytes.len() as i64, mtime_ns, hash.as_bytes().to_vec(), line_count, now],
            )?;
            fid
        }
        None => {
            tx.execute(
                "INSERT INTO file(path_sid,lang,size,mtime_ns,content_hash,line_count,indexed_at,tier)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,0)",
                params![path_sid, lang.as_i64(), bytes.len() as i64, mtime_ns, hash.as_bytes().to_vec(), line_count, now],
            )?;
            tx.last_insert_rowid()
        }
    };
    tx.execute(
        "INSERT INTO line_index(file_id,offsets) VALUES (?1,?2)
         ON CONFLICT(file_id) DO UPDATE SET offsets=excluded.offsets",
        params![file_id, line_index::encode(&offsets)],
    )?;

    // Existing symbols of this file, keyed by symbol_key (for stable-id reconcile).
    let mut existing: HashMap<Vec<u8>, ExistingSymbol> = HashMap::new();
    {
        let mut stmt = tx.prepare(
            "SELECT s.symbol_key, s.id, n.text, np.text FROM symbol s
             JOIN string_pool n  ON n.id  = s.name_sid
             JOIN string_pool np ON np.id = s.name_path_sid
             WHERE s.file_id=?1",
        )?;
        let rows = stmt.query_map([file_id], |r| {
            Ok((
                r.get::<_, Vec<u8>>(0)?,
                ExistingSymbol { id: r.get(1)?, name: r.get(2)?, name_path: r.get(3)? },
            ))
        })?;
        for row in rows {
            let (k, v) = row?;
            existing.insert(k, v);
        }
    }

    let mut seen: HashSet<Vec<u8>> = HashSet::new();
    let mut counter: HashMap<(String, i64), u32> = HashMap::new();
    for ex in &extracted {
        let kind_i = ex.kind.as_i64();
        let nth = {
            let c = counter.entry((ex.name_path.clone(), kind_i)).or_insert(0);
            let v = *c;
            *c += 1;
            v
        };
        let key = blake3::hash(format!("{rel}\0{}\0{kind_i}\0{nth}", ex.name_path).as_bytes())
            .as_bytes()
            .to_vec();
        seen.insert(key.clone());

        if let Some(prev) = existing.get(&key) {
            // Same identity (name_path/kind/nth) → keep id, refresh positions.
            tx.execute(
                "UPDATE symbol SET start_line=?2,start_col=?3,end_line=?4,end_col=?5,sel_line=?6,sel_col=?7
                 WHERE id=?1",
                params![prev.id, ex.range.start_line, ex.range.start_col, ex.range.end_line, ex.range.end_col, ex.sel_line, ex.sel_col],
            )?;
        } else {
            let name_sid = writer::intern(&tx, &ex.name)?;
            let np_sid = writer::intern(&tx, &ex.name_path)?;
            tx.execute(
                "INSERT INTO symbol(symbol_key,file_id,name_sid,name_path_sid,kind,
                                    start_line,start_col,end_line,end_col,sel_line,sel_col)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
                params![key, file_id, name_sid, np_sid, kind_i,
                    ex.range.start_line, ex.range.start_col, ex.range.end_line, ex.range.end_col, ex.sel_line, ex.sel_col],
            )?;
            let sid = tx.last_insert_rowid();
            tx.execute(
                "INSERT INTO symbol_fts(rowid,name,name_path) VALUES (?1,?2,?3)",
                params![sid, ex.name, ex.name_path],
            )?;
        }
        stats.symbols += 1;
    }

    // Delete symbols that disappeared (manual FTS delete with OLD values, then row).
    for (key, prev) in &existing {
        if !seen.contains(key) {
            tx.execute(
                "INSERT INTO symbol_fts(symbol_fts,rowid,name,name_path) VALUES('delete',?1,?2,?3)",
                params![prev.id, prev.name, prev.name_path],
            )?;
            tx.execute("DELETE FROM symbol WHERE id=?1", [prev.id])?;
        }
    }

    tx.commit()?;
    stats.files += 1;
    Ok(())
}
