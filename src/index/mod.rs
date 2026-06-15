//! Tier0 full-scan indexing. Walks honoring .gitignore + .codemapignore, parses with
//! tree-sitter, writes per file in a transaction (prune + insert). Incremental (git/mtime)
//! and parallelism (rayon) land in M4.

use crate::db::{line_index, writer, Db};
use crate::ts;
use crate::types::Language;
use anyhow::{Context, Result};
use ignore::WalkBuilder;
use rusqlite::params;
use std::collections::HashMap;
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
    writer::prune_file(&tx, rel)?;
    let path_sid = writer::intern(&tx, rel)?;
    tx.execute(
        "INSERT INTO file(path_sid,lang,size,mtime_ns,content_hash,line_count,indexed_at,tier)
         VALUES (?1,?2,?3,?4,?5,?6,?7,0)",
        params![
            path_sid,
            lang.as_i64(),
            bytes.len() as i64,
            mtime_ns,
            hash.as_bytes().to_vec(),
            line_count,
            now
        ],
    )?;
    let file_id = tx.last_insert_rowid();
    tx.execute(
        "INSERT INTO line_index(file_id,offsets) VALUES (?1,?2)",
        params![file_id, line_index::encode(&offsets)],
    )?;

    // Deterministic disambiguator: Nth definition with the same (name_path, kind) in the file.
    let mut counter: HashMap<(String, i64), u32> = HashMap::new();
    for ex in &extracted {
        let kind_i = ex.kind.as_i64();
        let nth = {
            let c = counter.entry((ex.name_path.clone(), kind_i)).or_insert(0);
            let v = *c;
            *c += 1;
            v
        };
        let key = blake3::hash(format!("{rel}\0{}\0{kind_i}\0{nth}", ex.name_path).as_bytes());
        let name_sid = writer::intern(&tx, &ex.name)?;
        let np_sid = writer::intern(&tx, &ex.name_path)?;
        tx.execute(
            "INSERT INTO symbol(symbol_key,file_id,name_sid,name_path_sid,kind,
                                start_line,start_col,end_line,end_col,sel_line,sel_col)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
            params![
                key.as_bytes().to_vec(),
                file_id,
                name_sid,
                np_sid,
                kind_i,
                ex.range.start_line,
                ex.range.start_col,
                ex.range.end_line,
                ex.range.end_col,
                ex.sel_line,
                ex.sel_col
            ],
        )?;
        let sid = tx.last_insert_rowid();
        tx.execute(
            "INSERT INTO symbol_fts(rowid,name,name_path) VALUES (?1,?2,?3)",
            params![sid, ex.name, ex.name_path],
        )?;
        stats.symbols += 1;
    }
    tx.commit()?;
    stats.files += 1;
    Ok(())
}
