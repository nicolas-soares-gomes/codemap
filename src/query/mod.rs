//! Query engine: resolve_symbol, read_symbol, outline, callers/callees, references, search.
//! Code is read on demand and re-validated against the file on disk before being served.

use crate::db::{line_index, Db};
use crate::types::{Provenance, Resolution, Role, SymbolKind};
use anyhow::{anyhow, Result};
use rusqlite::OptionalExtension;
use std::path::Path;

pub mod project;

/// Hard cap on nodes a single traversal may materialize (RAM guard on dense graphs).
const NODE_BUDGET: i64 = 20_000;

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
    // `bounded` caps how many nodes the recursive CTE materializes (RAM guard on dense hubs):
    // with no ORDER BY before it, SQLite stops the recursion once NODE_BUDGET rows are produced.
    let sql = format!(
        "WITH RECURSIVE walk(sym, depth, prov, res, path) AS (
             SELECT ?1, 0, -1, -1, ',' || ?1 || ','
           UNION ALL
             SELECT e.{to_col}, w.depth + 1, e.provenance, e.resolution, w.path || e.{to_col} || ','
             FROM edge e JOIN walk w ON e.{from_col} = w.sym
             WHERE e.kind = 1 AND w.depth < ?2 AND instr(w.path, ',' || e.{to_col} || ',') = 0
         ),
         bounded AS (SELECT sym, depth, prov, res FROM walk WHERE depth > 0 LIMIT ?4),
         best AS (
             SELECT sym, depth, prov, res, ROW_NUMBER() OVER (PARTITION BY sym ORDER BY depth) rn
             FROM bounded
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
        .query_map(rusqlite::params![root_id, depth, limit, NODE_BUDGET], |r| {
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

/// Transitive callers — what breaks if this symbol changes.
pub fn impact(db: &Db, root_id: i64, max_depth: i64, limit: i64) -> Result<Vec<EdgeHit>> {
    callers(db, root_id, max_depth, limit)
}

/// Reachable ancestors that are roots (no incoming call edges) — the entrypoints.
pub fn trace_to_roots(db: &Db, root_id: i64, max_depth: i64, limit: i64) -> Result<Vec<EdgeHit>> {
    let ancestors = callers(db, root_id, max_depth, 1000)?;
    let mut has_caller = db
        .conn
        .prepare("SELECT 1 FROM edge WHERE target_symbol_id=?1 AND kind=1 LIMIT 1")?;
    let mut roots = Vec::new();
    for a in ancestors {
        if has_caller
            .query_row([a.id], |_| Ok(()))
            .optional()?
            .is_none()
        {
            roots.push(a);
        }
        if roots.len() as i64 >= limit {
            break;
        }
    }
    Ok(roots)
}

/// A reference occurrence: the enclosing symbol and location where a target symbol is used.
#[derive(Debug, Clone)]
pub struct RefHit {
    pub enclosing: Option<String>,
    pub file: String,
    pub line: u32,
    pub role: Option<Role>,
}

pub fn references(db: &Db, symbol_id: i64, limit: i64) -> Result<Vec<RefHit>> {
    let mut stmt = db.conn.prepare(
        "SELECT np.text, fp.text, o.start_line, o.role
         FROM occurrence o
         JOIN file f          ON f.id = o.file_id
         JOIN string_pool fp  ON fp.id = f.path_sid
         LEFT JOIN symbol enc ON enc.id = o.enclosing_id
         LEFT JOIN string_pool np ON np.id = enc.name_path_sid
         WHERE o.symbol_id = ?1
         ORDER BY fp.text, o.start_line
         LIMIT ?2",
    )?;
    let rows = stmt
        .query_map(rusqlite::params![symbol_id, limit], |r| {
            Ok(RefHit {
                enclosing: r.get::<_, Option<String>>(0)?,
                file: r.get(1)?,
                line: r.get(2)?,
                role: Role::from_i64(r.get(3)?),
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Fields/consts/variables declared under a scope name_path.
pub fn variables(db: &Db, scope: &str, limit: i64) -> Result<Vec<Hit>> {
    let prefix = format!("{scope}/");
    let mut stmt = db.conn.prepare(
        "SELECT s.id, np.text, fp.text, s.start_line, s.kind
         FROM symbol s
         JOIN string_pool np ON np.id = s.name_path_sid
         JOIN file f         ON f.id  = s.file_id
         JOIN string_pool fp ON fp.id = f.path_sid
         WHERE np.text LIKE ?1 || '%' AND s.kind IN (?2, ?3, ?4)
         ORDER BY s.start_line
         LIMIT ?5",
    )?;
    let hits = stmt
        .query_map(
            rusqlite::params![
                prefix,
                SymbolKind::Field.as_i64(),
                SymbolKind::Variable.as_i64(),
                SymbolKind::Const.as_i64(),
                limit
            ],
            row_to_hit,
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(hits)
}

/// Search symbols by name. mode=symbol (default) is a fast FTS5 token-PREFIX match — use it when
/// you know how a name starts. mode=text is a slower case-insensitive SUBSTRING match over the
/// name and name_path — use it for a fragment in the middle of an identifier (e.g. `inch` finds
/// `OneinchClient`). codemap indexes symbols, not file contents, so neither finds text that only
/// appears in strings/comments/config — use grep for that. semantic is unavailable.
pub fn search(db: &Db, query: &str, mode: &str, limit: i64) -> Result<Vec<Hit>> {
    if mode == "semantic" {
        return Err(anyhow!(
            "semantic search is not available (opt-in sqlite-vec, not built) — use mode=symbol or text"
        ));
    }
    let term: String = query
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '_')
        .collect();
    if term.is_empty() {
        return Ok(Vec::new());
    }
    if mode == "text" {
        return search_substring(db, &term, limit);
    }
    let mut stmt = db.conn.prepare(
        "SELECT s.id, np.text, fp.text, s.start_line, s.kind
         FROM symbol_fts
         JOIN symbol s       ON s.id  = symbol_fts.rowid
         JOIN string_pool np ON np.id = s.name_path_sid
         JOIN file f         ON f.id  = s.file_id
         JOIN string_pool fp ON fp.id = f.path_sid
         WHERE symbol_fts MATCH ?1
         LIMIT ?2",
    )?;
    let hits = stmt
        .query_map(rusqlite::params![format!("{term}*"), limit], row_to_hit)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(hits)
}

/// Case-insensitive substring match over symbol name and name_path (LIKE %term%). `_` in the term
/// is escaped so it stays literal rather than acting as a LIKE wildcard.
fn search_substring(db: &Db, term: &str, limit: i64) -> Result<Vec<Hit>> {
    let pattern = format!("%{}%", term.replace('_', "\\_"));
    let mut stmt = db.conn.prepare(
        "SELECT s.id, np.text, fp.text, s.start_line, s.kind
         FROM symbol s
         JOIN string_pool n  ON n.id  = s.name_sid
         JOIN string_pool np ON np.id = s.name_path_sid
         JOIN file f         ON f.id  = s.file_id
         JOIN string_pool fp ON fp.id = f.path_sid
         WHERE n.text LIKE ?1 ESCAPE '\\' OR np.text LIKE ?1 ESCAPE '\\'
         ORDER BY s.id LIMIT ?2",
    )?;
    let hits = stmt
        .query_map(rusqlite::params![pattern, limit], row_to_hit)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(hits)
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

const GREP_MAX_FILE: usize = 2 * 1024 * 1024;
const GREP_LINE_CAP: usize = 200;

/// An indexed symbol's span for enclosing-symbol lookup: (id, name_path, start_line, end_line).
type SymRange = (i64, String, u32, u32);

/// A content match: the file/line, the matched line (trimmed/truncated), and the enclosing
/// symbol if the file is indexed (None for non-source files like config — same as plain grep).
#[derive(Debug)]
pub struct GrepHit {
    pub file: String,
    pub line: u32,
    pub text: String,
    pub enclosing: Option<(i64, String)>, // (symbol id, name_path)
}

/// Regex search of file CONTENTS across the repo (honoring .gitignore/.codemapignore), with each
/// hit mapped to the innermost symbol whose range contains it. Unlike symbol search, this reads
/// the files — so it finds values inside strings/consts/config, and bridges grep to the graph.
pub fn grep(
    db: &Db,
    root: &Path,
    pattern: &str,
    ignore_case: bool,
    limit: i64,
) -> Result<Vec<GrepHit>> {
    use ignore::WalkBuilder;
    let re = regex::RegexBuilder::new(pattern)
        .case_insensitive(ignore_case)
        .build()
        .map_err(|e| anyhow!("invalid regex: {e}"))?;

    // Cache per file: the indexed symbols (id, name_path, start, end), or None if not indexed.
    let mut sym_cache: std::collections::HashMap<String, Option<Vec<SymRange>>> =
        std::collections::HashMap::new();
    let mut hits = Vec::new();

    for entry in WalkBuilder::new(root)
        .add_custom_ignore_filename(".codemapignore")
        .hidden(false) // search dotfiles like .env.example, where config values live
        .filter_entry(|e| {
            let n = e.file_name();
            n != ".git" && n != ".codemap" // but never descend into these
        })
        .build()
        .flatten()
    {
        if hits.len() as i64 >= limit {
            break;
        }
        if !entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
            continue;
        }
        let path = entry.path();
        let bytes = match std::fs::read(path) {
            Ok(b) => b,
            Err(_) => continue,
        };
        if bytes.len() > GREP_MAX_FILE || bytes.iter().take(8192).any(|b| *b == 0) {
            continue; // too big or binary
        }
        let rel = path
            .strip_prefix(root)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/");
        let text = String::from_utf8_lossy(&bytes);
        let mut matched_lines: Vec<(u32, &str)> = Vec::new();
        for (i, line) in text.lines().enumerate() {
            if re.is_match(line) {
                matched_lines.push((i as u32 + 1, line));
                if hits.len() + matched_lines.len() >= limit as usize {
                    break;
                }
            }
        }
        if matched_lines.is_empty() {
            continue;
        }
        let symbols = sym_cache
            .entry(rel.clone())
            .or_insert_with(|| symbols_for_file(db, &rel).ok().flatten());
        for (line_no, line) in matched_lines {
            let enclosing = symbols
                .as_ref()
                .and_then(|syms| innermost_symbol(syms, line_no));
            hits.push(GrepHit {
                file: rel.clone(),
                line: line_no,
                text: truncate_line(line),
                enclosing,
            });
            if hits.len() as i64 >= limit {
                break;
            }
        }
    }
    Ok(hits)
}

fn truncate_line(line: &str) -> String {
    let t = line.trim();
    if t.chars().count() > GREP_LINE_CAP {
        let cut: String = t.chars().take(GREP_LINE_CAP).collect();
        format!("{cut}…")
    } else {
        t.to_string()
    }
}

/// Indexed symbols of a file as (id, name_path, start_line, end_line), or None if not indexed.
fn symbols_for_file(db: &Db, rel: &str) -> Result<Option<Vec<SymRange>>> {
    let file_id: Option<i64> = db
        .conn
        .query_row(
            "SELECT f.id FROM file f JOIN string_pool s ON s.id=f.path_sid WHERE s.text=?1",
            [rel],
            |r| r.get(0),
        )
        .optional()?;
    let Some(file_id) = file_id else {
        return Ok(None);
    };
    let mut stmt = db.conn.prepare(
        "SELECT s.id, np.text, s.start_line, s.end_line
         FROM symbol s JOIN string_pool np ON np.id=s.name_path_sid
         WHERE s.file_id=?1",
    )?;
    let rows = stmt
        .query_map([file_id], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(Some(rows))
}

/// Innermost symbol (smallest range) whose [start,end] contains `line`.
fn innermost_symbol(syms: &[SymRange], line: u32) -> Option<(i64, String)> {
    syms.iter()
        .filter(|(_, _, s, e)| *s <= line && line <= *e)
        .min_by_key(|(_, _, s, e)| e - s)
        .map(|(id, np, _, _)| (*id, np.clone()))
}
