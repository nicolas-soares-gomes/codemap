//! Full-scan indexing. Walks honoring .gitignore + .codemapignore, parses with tree-sitter,
//! and reconciles each file by `symbol_key` so symbol ids stay stable across reindex.
//! Incremental updates use git (or an mtime/size fallback) — see `reconcile`.

use crate::db::{line_index, writer, Db};
use crate::ts;
use crate::types::{EdgeKind, Language, Provenance, Resolution, Role, SymbolKind};
use anyhow::{Context, Result};
use ignore::WalkBuilder;
use rusqlite::{params, OptionalExtension};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::process::Command;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const MAX_FILE: usize = 2 * 1024 * 1024;
const MAX_CALL_CANDIDATES: i64 = 8;

#[derive(Default, Debug)]
pub struct IndexStats {
    pub files: u64,
    pub symbols: u64,
}

pub fn detect_lang(path: &Path) -> Option<Language> {
    match path.extension().and_then(|e| e.to_str()) {
        Some("rs") => Some(Language::Rust),
        Some("ts" | "mts" | "cts") => Some(Language::TypeScript),
        Some("py" | "pyi") => Some(Language::Python),
        Some("go") => Some(Language::Go),
        Some("java") => Some(Language::Java),
        Some("cs") => Some(Language::CSharp),
        Some("php") => Some(Language::Php),
        Some("c") => Some(Language::C),
        Some("cc" | "cpp" | "cxx" | "hpp" | "hh" | "hxx") => Some(Language::Cpp),
        // .h is ambiguous (C vs C++ headers); default to C.
        Some("h") => Some(Language::C),
        Some("swift") => Some(Language::Swift),
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

/// Reindex a single file in place, preserving stable symbol ids. Prunes the file if
/// it no longer exists. Used by the inline staleness guard.
pub fn reindex_file(db: &mut Db, root: &Path, rel: &str) -> Result<()> {
    let path = root.join(rel);
    let Some(lang) = detect_lang(&path) else {
        return Ok(());
    };
    let bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(_) => {
            let tx = db.conn.transaction()?;
            writer::prune_file(&tx, rel)?;
            tx.commit()?;
            return Ok(());
        }
    };
    let mtime_ns = std::fs::metadata(&path)
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_nanos() as i64)
        .unwrap_or(0);
    let mut stats = IndexStats::default();
    index_one(db, rel, lang, &bytes, mtime_ns, &mut stats)
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
        .query_row("SELECT id FROM file WHERE path_sid=?1", [path_sid], |r| {
            r.get(0)
        })
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
                ExistingSymbol {
                    id: r.get(1)?,
                    name: r.get(2)?,
                    name_path: r.get(3)?,
                },
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
                params![
                    key,
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

/// Pass 2: extract call sites and build syntactic call edges. Name-based resolution only,
/// so edges are provenance=tree_sitter, resolution=ambiguous. Must run after all definitions
/// are indexed. Idempotent: clears each file's prior call edges/occurrences first.
pub fn resolve_calls(db: &mut Db, root: &Path) -> Result<u64> {
    let files: Vec<(i64, String, i64)> = {
        let mut stmt = db.conn.prepare(
            "SELECT f.id, fp.text, f.lang FROM file f JOIN string_pool fp ON fp.id=f.path_sid",
        )?;
        let rows = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows
    };
    let mut edges = 0u64;
    for (file_id, rel, lang_i) in files {
        let Some(lang) = Language::from_i64(lang_i) else {
            continue;
        };
        let bytes = match std::fs::read(root.join(&rel)) {
            Ok(b) => b,
            Err(_) => continue,
        };
        let source = String::from_utf8_lossy(&bytes);
        let calls = ts::extract_calls(lang, &source);
        edges += resolve_calls_for_file(db, file_id, &calls)?;
    }
    Ok(edges)
}

/// Re-resolve the call edges originating in a single file (used by incremental reconcile).
pub fn resolve_calls_file(db: &mut Db, root: &Path, rel: &str) -> Result<u64> {
    let path = root.join(rel);
    let Some(lang) = detect_lang(&path) else {
        return Ok(0);
    };
    let bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(_) => return Ok(0),
    };
    let file_id: Option<i64> = db
        .conn
        .query_row(
            "SELECT f.id FROM file f JOIN string_pool s ON s.id=f.path_sid WHERE s.text=?1",
            [rel],
            |r| r.get(0),
        )
        .optional()?;
    let Some(file_id) = file_id else { return Ok(0) };
    let calls = ts::extract_calls(lang, &String::from_utf8_lossy(&bytes));
    resolve_calls_for_file(db, file_id, &calls)
}

#[derive(Default, Debug)]
pub struct ReconcileStats {
    pub changed: u64,
    pub added: u64,
    pub deleted: u64,
    pub unchanged: u64,
}

/// Incremental reconcile via mtime+size (hashing only suspects), then pruning deletions and
/// re-resolving calls for changed/added files. Cross-file call edges to newly-added symbols
/// may require a full `index` to appear (resolve_calls runs per changed file here).
pub fn reconcile(db: &mut Db, root: &Path) -> Result<ReconcileStats> {
    // Fast path: if this is a git repo with a known indexed_commit, ask git for the exact
    // changed set in O(changes) instead of walking the whole tree.
    if let Some(stats) = try_git_reconcile(db, root)? {
        return Ok(stats);
    }
    let snapshot = file_snapshot(db)?;
    let mut seen: HashSet<String> = HashSet::new();
    let mut changed_files: Vec<String> = Vec::new();
    let mut stats = ReconcileStats::default();

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
        if detect_lang(path).is_none() {
            continue;
        }
        let rel = path
            .strip_prefix(root)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/");
        seen.insert(rel.clone());
        let md = match entry.metadata() {
            Ok(m) => m,
            Err(_) => continue,
        };
        let size = md.len() as i64;
        let mtime = md
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_nanos() as i64)
            .unwrap_or(0);
        match snapshot.get(&rel) {
            None => {
                reindex_file(db, root, &rel)?;
                changed_files.push(rel);
                stats.added += 1;
            }
            Some((m, s, _)) if *m == mtime && *s == size => {
                stats.unchanged += 1;
            }
            Some((_, _, h)) => {
                // mtime/size differ: confirm by hashing before doing work.
                let bytes = std::fs::read(path).unwrap_or_default();
                if blake3::hash(&bytes).as_bytes()[..] != h[..] {
                    reindex_file(db, root, &rel)?;
                    changed_files.push(rel);
                    stats.changed += 1;
                } else {
                    stats.unchanged += 1;
                }
            }
        }
    }

    for rel in snapshot.keys() {
        if !seen.contains(rel) {
            let tx = db.conn.transaction()?;
            writer::prune_file(&tx, rel)?;
            tx.commit()?;
            stats.deleted += 1;
        }
    }

    for rel in &changed_files {
        resolve_calls_file(db, root, rel)?;
    }
    // Seed indexed_commit so the next reconcile can take the git fast-path.
    if let Some(head) = git_head(root) {
        db.set_meta("indexed_commit", &head)?;
        db.set_meta("scanner_mode", "git-seeded")?;
    } else {
        db.set_meta("scanner_mode", "fs")?;
    }
    Ok(stats)
}

/// Remove from the index every file that no longer exists on disk. Returns how many were pruned.
pub fn prune(db: &mut Db, root: &Path) -> Result<u64> {
    let snapshot = file_snapshot(db)?;
    let mut n = 0;
    for rel in snapshot.keys() {
        if !root.join(rel).exists() {
            let tx = db.conn.transaction()?;
            writer::prune_file(&tx, rel)?;
            tx.commit()?;
            n += 1;
        }
    }
    Ok(n)
}

/// Incremental reconcile using git's diff between the indexed commit and HEAD plus the working
/// tree status. Returns None (→ fall back to the fs walk) when not a git repo, no indexed_commit
/// yet, or the indexed commit is unreachable (shallow/rebased/gc'd).
fn try_git_reconcile(db: &mut Db, root: &Path) -> Result<Option<ReconcileStats>> {
    if !root.join(".git").exists() {
        return Ok(None);
    }
    let Some(indexed) = db.get_meta("indexed_commit")? else {
        return Ok(None);
    };
    let Some(head) = git_head(root) else {
        return Ok(None);
    };
    if !git_commit_exists(root, &indexed) {
        return Ok(None);
    }

    let mut changed: HashSet<String> = HashSet::new();
    let mut deleted: HashSet<String> = HashSet::new();

    // Committed changes between the indexed commit and HEAD.
    if indexed != head {
        let out = Command::new("git")
            .arg("-C")
            .arg(root)
            .args(["diff", "--name-status", &indexed, &head])
            .output()?;
        for line in String::from_utf8_lossy(&out.stdout).lines() {
            let mut it = line.split('\t');
            let Some(status) = it.next() else { continue };
            let paths: Vec<&str> = it.collect();
            match (status.chars().next(), paths.as_slice()) {
                (Some('D'), [p]) => {
                    deleted.insert((*p).to_string());
                }
                (Some('R') | Some('C'), [old, new]) => {
                    deleted.insert((*old).to_string());
                    changed.insert((*new).to_string());
                }
                (_, [p]) => {
                    changed.insert((*p).to_string());
                }
                _ => {}
            }
        }
    }

    // Working tree + staged + untracked (porcelain respects .gitignore).
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["status", "--porcelain"])
        .output()?;
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        if line.len() < 3 {
            continue;
        }
        let xy = &line[..2];
        let rest = line[3..].trim();
        if let Some((orig, dest)) = rest.split_once(" -> ") {
            deleted.insert(unquote(orig));
            changed.insert(unquote(dest));
        } else if xy.contains('D') {
            deleted.insert(unquote(rest));
        } else {
            changed.insert(unquote(rest));
        }
    }

    let mut stats = ReconcileStats::default();
    let mut touched: Vec<String> = Vec::new();
    for rel in &deleted {
        if !changed.contains(rel) {
            let tx = db.conn.transaction()?;
            writer::prune_file(&tx, rel)?;
            tx.commit()?;
            stats.deleted += 1;
        }
    }
    for rel in &changed {
        if detect_lang(&root.join(rel)).is_some() {
            reindex_file(db, root, rel)?;
            touched.push(rel.clone());
            stats.changed += 1;
        }
    }
    for rel in &touched {
        resolve_calls_file(db, root, rel)?;
    }

    db.set_meta("indexed_commit", &head)?;
    db.set_meta("scanner_mode", "git")?;
    Ok(Some(stats))
}

fn git_head(root: &Path) -> Option<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!s.is_empty()).then_some(s)
}

fn git_commit_exists(root: &Path, sha: &str) -> bool {
    Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["cat-file", "-e", &format!("{sha}^{{commit}}")])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Strip git's surrounding quotes from a path with special characters.
fn unquote(s: &str) -> String {
    let s = s.trim();
    s.strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .unwrap_or(s)
        .to_string()
}

/// path -> (mtime_ns, size, content_hash)
type FileSnapshot = HashMap<String, (i64, i64, Vec<u8>)>;

/// True if a path is inside an internal/ignored dir we must not react to (avoids reindex loops).
fn is_internal(p: &Path) -> bool {
    let s = p.to_string_lossy();
    s.contains("/.codemap/") || s.contains("/.git/") || s.contains("/target/")
}

/// Toggleable watcher: reconciles once on startup, then on debounced filesystem changes.
/// Blocks until the channel closes (Ctrl-C). Events under .codemap/.git/target are ignored.
pub fn watch(root: &Path) -> Result<()> {
    use notify::RecursiveMode;
    use notify_debouncer_full::new_debouncer;

    {
        let mut db = Db::open(&root.join(".codemap").join("index.db"))?;
        reconcile(&mut db, root)?;
    }

    let (tx, rx) = std::sync::mpsc::channel();
    let mut debouncer = new_debouncer(Duration::from_millis(500), None, tx)?;
    debouncer.watch(root, RecursiveMode::Recursive)?;
    eprintln!("codemap: watching {} (Ctrl-C to stop)", root.display());

    for result in rx {
        let events = match result {
            Ok(events) => events,
            Err(errs) => {
                eprintln!("codemap: watch error: {errs:?}");
                continue;
            }
        };
        let relevant = events
            .iter()
            .any(|e| e.paths.iter().any(|p| !is_internal(p)));
        if !relevant {
            continue;
        }
        let mut db = Db::open(&root.join(".codemap").join("index.db"))?;
        match reconcile(&mut db, root) {
            Ok(s) => eprintln!(
                "codemap: reconciled ({} changed, {} added, {} deleted)",
                s.changed, s.added, s.deleted
            ),
            Err(e) => eprintln!("codemap: reconcile error: {e}"),
        }
    }
    Ok(())
}

fn file_snapshot(db: &Db) -> Result<FileSnapshot> {
    let mut stmt = db.conn.prepare(
        "SELECT s.text, f.mtime_ns, f.size, f.content_hash
         FROM file f JOIN string_pool s ON s.id=f.path_sid",
    )?;
    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                (
                    r.get::<_, i64>(1)?,
                    r.get::<_, i64>(2)?,
                    r.get::<_, Vec<u8>>(3)?,
                ),
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows.into_iter().collect())
}

fn resolve_calls_for_file(db: &mut Db, file_id: i64, calls: &[ts::CallSite]) -> Result<u64> {
    // (id, start_line, end_line, kind) for enclosing-symbol lookup.
    let callables: Vec<(i64, u32, u32, i64)> = {
        let mut stmt = db
            .conn
            .prepare("SELECT id,start_line,end_line,kind FROM symbol WHERE file_id=?1")?;
        let rows = stmt
            .query_map([file_id], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows
    };

    let tx = db.conn.transaction()?;
    tx.execute(
        "DELETE FROM edge WHERE kind=?1 AND source_symbol_id IN (SELECT id FROM symbol WHERE file_id=?2)",
        params![EdgeKind::Calls.as_i64(), file_id],
    )?;
    tx.execute(
        "DELETE FROM occurrence WHERE file_id=?1 AND role=?2",
        params![file_id, Role::Call.as_i64()],
    )?;

    let mut count = 0u64;
    {
        let mut resolve_stmt = tx.prepare(
            "SELECT s.id FROM symbol s JOIN string_pool n ON n.id=s.name_sid WHERE n.text=?1 LIMIT ?2",
        )?;
        for call in calls {
            let Some(enclosing) = innermost_callable(&callables, call.range.start_line) else {
                continue;
            };
            let candidates: Vec<i64> = resolve_stmt
                .query_map(params![call.name, MAX_CALL_CANDIDATES], |r| r.get(0))?
                .collect::<rusqlite::Result<_>>()?;
            if candidates.is_empty() {
                continue;
            }
            tx.execute(
                "INSERT INTO occurrence(symbol_id,enclosing_id,file_id,role,start_line,start_col,end_line,end_col)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
                params![
                    candidates[0], enclosing, file_id, Role::Call.as_i64(),
                    call.range.start_line, call.range.start_col, call.range.end_line, call.range.end_col
                ],
            )?;
            let occ = tx.last_insert_rowid();
            for &cand in &candidates {
                count += tx.execute(
                    "INSERT OR IGNORE INTO edge(source_symbol_id,target_symbol_id,kind,provenance,resolution)
                     VALUES (?1,?2,?3,?4,?5)",
                    params![enclosing, cand, EdgeKind::Calls.as_i64(), Provenance::TreeSitter.as_i64(), Resolution::Ambiguous.as_i64()],
                )? as u64;
                tx.execute(
                    "INSERT OR IGNORE INTO call_site(source_symbol_id,target_symbol_id,kind,occurrence_id)
                     VALUES (?1,?2,?3,?4)",
                    params![enclosing, cand, EdgeKind::Calls.as_i64(), occ],
                )?;
            }
        }
    }
    tx.commit()?;
    Ok(count)
}

/// Innermost Function/Method whose range contains `line`.
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
