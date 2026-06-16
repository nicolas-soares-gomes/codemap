//! MCP server (rmcp, stdio): exposes the codemap navigation tools to agents. Navigation
//! tools return compact, code-free rows; read_symbol is the only tool that returns code.

use crate::db::Db;
use crate::query::{self, Code, EdgeHit, Hit};
use rmcp::handler::server::wrapper::Parameters;
use rmcp::{tool, tool_handler, tool_router, ServerHandler, ServiceExt};
use serde::Deserialize;
use std::path::PathBuf;

#[derive(Clone)]
pub struct CodemapServer {
    root: PathBuf,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ResolveArgs {
    /// Name, name_path, or substring to look up.
    pub query: String,
    pub limit: Option<i64>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SymbolArgs {
    /// `sym:N`, a name, or a name_path like `Type/method`.
    pub symbol: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct OutlineArgs {
    /// Repo-relative file path.
    pub file: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct EdgesArgs {
    /// `sym:N`, a name, or a name_path.
    pub symbol: String,
    pub depth: Option<i64>,
    pub limit: Option<i64>,
}

#[tool_router]
impl CodemapServer {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    #[tool(
        name = "codemap_resolve_symbol",
        description = "Find symbols matching a name and return stable ids + name_paths. Use INSTEAD OF grepping for a function/class name: returns exact ids to chain into read_symbol/get_callers, with no code. Compact rows (id, name_path, file:line, kind)."
    )]
    fn resolve_symbol(&self, Parameters(a): Parameters<ResolveArgs>) -> Result<String, String> {
        let db = self.db()?;
        let hits = query::resolve(&db, &a.query, a.limit.unwrap_or(25)).map_err(e)?;
        Ok(proj_hits(&format!("resolve \"{}\"", a.query), &hits))
    }

    #[tool(
        name = "codemap_read_symbol",
        description = "Return ONE symbol's code (its range only, not the whole file), with line numbers. The only tool that returns code — reach it via an id from resolve_symbol/get_callers."
    )]
    fn read_symbol(&self, Parameters(a): Parameters<SymbolArgs>) -> Result<String, String> {
        let mut db = self.db()?;
        let id = query::resolve_arg(&db, &a.symbol).map_err(e)?;
        let code = query::read_symbol(&mut db, &self.root, id).map_err(e)?;
        Ok(proj_read(&code))
    }

    #[tool(
        name = "codemap_get_callers",
        description = "Functions that CALL a symbol, following real edges (deduped), no code. Use INSTEAD OF grepping the function name. Rows carry prov/res (e.g. ts/ambiguous) so you know which edges to trust."
    )]
    fn get_callers(&self, Parameters(a): Parameters<EdgesArgs>) -> Result<String, String> {
        self.edges(&a, false)
    }

    #[tool(
        name = "codemap_get_callees",
        description = "Functions a symbol CALLS, following real edges (deduped), no code. Rows carry prov/res. Pass an id to read_symbol for code."
    )]
    fn get_callees(&self, Parameters(a): Parameters<EdgesArgs>) -> Result<String, String> {
        self.edges(&a, true)
    }

    #[tool(
        name = "codemap_get_file_outline",
        description = "Top-level symbols of a file (no code). Use INSTEAD OF reading the whole file to understand its structure."
    )]
    fn get_file_outline(&self, Parameters(a): Parameters<OutlineArgs>) -> Result<String, String> {
        let db = self.db()?;
        let hits = query::outline(&db, &a.file).map_err(e)?;
        Ok(proj_hits(&format!("outline {}", a.file), &hits))
    }

    fn db(&self) -> Result<Db, String> {
        let p = self.root.join(".codemap").join("index.db");
        if !p.exists() {
            return Err(format!("index not found at {} — run `codemap index` first", p.display()));
        }
        Db::open(&p).map_err(e)
    }

    fn edges(&self, a: &EdgesArgs, forward: bool) -> Result<String, String> {
        let db = self.db()?;
        let id = query::resolve_arg(&db, &a.symbol).map_err(e)?;
        let depth = a.depth.unwrap_or(1);
        let limit = a.limit.unwrap_or(50);
        let hits = if forward {
            query::callees(&db, id, depth, limit)
        } else {
            query::callers(&db, id, depth, limit)
        }
        .map_err(e)?;
        let label = if forward { "callees" } else { "callers" };
        Ok(proj_edges(label, id, depth, &hits))
    }
}

#[tool_handler]
impl ServerHandler for CodemapServer {}

pub async fn serve_stdio(root: PathBuf) -> anyhow::Result<()> {
    let service = CodemapServer::new(root).serve(rmcp::transport::io::stdio()).await?;
    service.waiting().await?;
    Ok(())
}

fn e<E: std::fmt::Display>(err: E) -> String {
    err.to_string()
}

fn kind_label(k: Option<crate::types::SymbolKind>) -> String {
    k.map(|k| format!("{k:?}").to_lowercase()).unwrap_or_else(|| "?".into())
}

fn proj_hits(header: &str, hits: &[Hit]) -> String {
    let mut s = format!("# {header}  ({} matches)\n# fields: id | name_path | file:line | kind\n", hits.len());
    for h in hits {
        s.push_str(&format!("sym:{} | {} | {}:{} | {}\n", h.id, h.name_path, h.file, h.line, kind_label(h.kind)));
    }
    if !hits.is_empty() {
        s.push_str("# next: read_symbol(id) for code | get_callers(id) for who uses it\n");
    }
    s
}

fn proj_edges(label: &str, root_id: i64, depth: i64, hits: &[EdgeHit]) -> String {
    let mut s = format!(
        "# {label} of sym:{root_id}  (depth<={depth}, {} shown)\n# fields: id | name_path | file:line | kind | depth | prov/res\n",
        hits.len()
    );
    for h in hits {
        let pr = match (h.provenance, h.resolution) {
            (Some(p), Some(r)) => format!("{}/{}", p.abbrev(), r.abbrev()),
            _ => "-".into(),
        };
        s.push_str(&format!(
            "sym:{} | {} | {}:{} | {} | {} | {}\n",
            h.id, h.name_path, h.file, h.line, kind_label(h.kind), h.depth, pr
        ));
    }
    s.push_str("# next: read_symbol(id) for code\n");
    s
}

fn proj_read(c: &Code) -> String {
    let state = if c.reindexed { " (reindexed)" } else { "" };
    let mut s = format!("# sym:{} {}  {}:{}-{}{state}\n", c.id, c.name_path, c.file, c.start_line, c.end_line);
    for (i, line) in c.code.lines().enumerate() {
        s.push_str(&format!("{:>5}  {line}\n", c.start_line as usize + i));
    }
    s
}
