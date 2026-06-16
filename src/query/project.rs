//! Compact, token-budgeted projection shared by the CLI and the MCP server. Wraps the query
//! functions, detects limit truncation (by fetching limit+1), applies a token ceiling, and
//! always signals what was cut (`truncated_by`) plus a `# next:` hint — never truncates silently.

use super::{Code, EdgeHit, Hit, RefHit};
use crate::db::Db;
use crate::types::SymbolKind;
use anyhow::Result;
use std::path::Path;

/// Safe ceiling below the ~25k MCP response budget (chars/4 heuristic).
const TOKEN_CEILING: usize = 22_000;

fn est_tokens(s: &str) -> usize {
    s.len() / 4 + 1
}

fn kind_label(k: Option<SymbolKind>) -> String {
    k.map(|k| format!("{k:?}").to_lowercase())
        .unwrap_or_else(|| "?".into())
}

/// Assemble a compact block: header + fields line + rows (cut at the token ceiling) + next hint.
/// `limit_cut` = the query returned more than `limit` rows (extra already dropped by the caller).
fn assemble(label: &str, fields: &str, rows: Vec<String>, limit_cut: bool, next: &str) -> String {
    let mut by: Option<&str> = if limit_cut { Some("limit") } else { None };
    let mut used = est_tokens(label) + est_tokens(fields) + est_tokens(next) + 8;
    let mut emitted = Vec::new();
    for row in &rows {
        let t = est_tokens(row);
        if used + t > TOKEN_CEILING {
            by = Some("token");
            break;
        }
        used += t;
        emitted.push(row.clone());
    }
    let dropped = rows.len() - emitted.len();
    let head = match by {
        Some(b) => format!(
            "# {label}  ({} shown, truncated_by={b}, dropped\u{2248}{})",
            emitted.len(),
            if b == "token" { dropped } else { dropped + 1 }
        ),
        None => format!("# {label}  ({} shown)", emitted.len()),
    };
    let mut out = format!("{head}\n# fields: {fields}\n");
    for r in &emitted {
        out.push_str(r);
        out.push('\n');
    }
    out.push_str("# next: ");
    out.push_str(next);
    out.push('\n');
    out
}

fn hit_row(h: &Hit) -> String {
    format!(
        "sym:{} | {} | {}:{} | {}",
        h.id,
        h.name_path,
        h.file,
        h.line,
        kind_label(h.kind)
    )
}

fn edge_row(h: &EdgeHit) -> String {
    let pr = match (h.provenance, h.resolution) {
        (Some(p), Some(r)) => format!("{}/{}", p.abbrev(), r.abbrev()),
        _ => "-".into(),
    };
    format!(
        "sym:{} | {} | {}:{} | {} | {} | {}",
        h.id,
        h.name_path,
        h.file,
        h.line,
        kind_label(h.kind),
        h.depth,
        pr
    )
}

/// Fetch limit+1, drop the extra, and report whether it was cut by the limit.
fn capped<T>(mut rows: Vec<T>, limit: i64) -> (Vec<T>, bool) {
    if rows.len() as i64 > limit {
        rows.truncate(limit.max(0) as usize);
        (rows, true)
    } else {
        (rows, false)
    }
}

const HITS_FIELDS: &str = "id | name_path | file:line | kind";
const EDGES_FIELDS: &str = "id | name_path | file:line | kind | depth | prov/res";

pub fn resolve(db: &Db, query: &str, limit: i64) -> Result<String> {
    let (hits, cut) = capped(super::resolve(db, query, limit + 1)?, limit);
    Ok(assemble(
        &format!("resolve \"{query}\""),
        HITS_FIELDS,
        hits.iter().map(hit_row).collect(),
        cut,
        "read_symbol(id) for code | get_callers(id)",
    ))
}

pub fn search(db: &Db, query: &str, mode: &str, limit: i64) -> Result<String> {
    let (hits, cut) = capped(super::search(db, query, mode, limit + 1)?, limit);
    Ok(assemble(
        &format!("search \"{query}\" (mode={mode})"),
        HITS_FIELDS,
        hits.iter().map(hit_row).collect(),
        cut,
        "read_symbol(id) | get_callers(id) | search(query, mode=text)",
    ))
}

pub fn outline(db: &Db, file: &str) -> Result<String> {
    let hits = super::outline(db, file)?;
    Ok(assemble(
        &format!("outline {file}"),
        HITS_FIELDS,
        hits.iter().map(hit_row).collect(),
        false,
        "read_symbol(id) for code | get_callers(id)",
    ))
}

pub fn variables(db: &Db, scope: &str, limit: i64) -> Result<String> {
    let (hits, cut) = capped(super::variables(db, scope, limit + 1)?, limit);
    Ok(assemble(
        &format!("variables in {scope}"),
        HITS_FIELDS,
        hits.iter().map(hit_row).collect(),
        cut,
        "read_symbol(id) for code",
    ))
}

pub fn edges(
    db: &Db,
    label: &str,
    id: i64,
    depth: i64,
    limit: i64,
    forward: bool,
) -> Result<String> {
    let rows = if forward {
        super::callees(db, id, depth, limit + 1)?
    } else {
        super::callers(db, id, depth, limit + 1)?
    };
    let (rows, cut) = capped(rows, limit);
    Ok(assemble(
        &format!("{label} sym:{id} (depth<={depth})"),
        EDGES_FIELDS,
        rows.iter().map(edge_row).collect(),
        cut,
        "read_symbol(id) for code | raise --limit/--depth to continue",
    ))
}

pub fn impact(db: &Db, id: i64, depth: i64, limit: i64) -> Result<String> {
    let (rows, cut) = capped(super::impact(db, id, depth, limit + 1)?, limit);
    Ok(assemble(
        &format!("impact of sym:{id} (depth<={depth})"),
        EDGES_FIELDS,
        rows.iter().map(edge_row).collect(),
        cut,
        "read_symbol(id) | raise --limit to see more affected",
    ))
}

pub fn trace_to_roots(db: &Db, id: i64, depth: i64, limit: i64) -> Result<String> {
    let (rows, cut) = capped(super::trace_to_roots(db, id, depth, limit + 1)?, limit);
    Ok(assemble(
        &format!("roots reaching sym:{id} (depth<={depth})"),
        EDGES_FIELDS,
        rows.iter().map(edge_row).collect(),
        cut,
        "read_symbol(id) | impact(id) for the opposite direction",
    ))
}

pub fn references(db: &Db, id: i64, limit: i64) -> Result<String> {
    let (refs, cut) = capped(super::references(db, id, limit + 1)?, limit);
    let rows: Vec<String> = refs.iter().map(ref_row).collect();
    Ok(assemble(
        &format!("references to sym:{id}"),
        "in_symbol | file:line | role",
        rows,
        cut,
        "read_symbol(in_symbol) for context | get_callers for calls only",
    ))
}

fn ref_row(r: &RefHit) -> String {
    let enc = r.enclosing.as_deref().unwrap_or("(top-level)");
    let role = r
        .role
        .map(|x| format!("{x:?}").to_lowercase())
        .unwrap_or_else(|| "?".into());
    format!("{enc} | {}:{} | {role}", r.file, r.line)
}

/// read_symbol is the only code path; render the symbol's range with line numbers.
pub fn read_symbol(db: &mut Db, root: &Path, id: i64) -> Result<String> {
    let c = super::read_symbol(db, root, id)?;
    Ok(render_code(&c))
}

pub fn render_code(c: &Code) -> String {
    let state = if c.reindexed { " (reindexed)" } else { "" };
    let mut s = format!(
        "# sym:{} {}  {}:{}-{}{state}\n",
        c.id, c.name_path, c.file, c.start_line, c.end_line
    );
    for (i, line) in c.code.lines().enumerate() {
        s.push_str(&format!("{:>5}  {line}\n", c.start_line as usize + i));
    }
    s
}
