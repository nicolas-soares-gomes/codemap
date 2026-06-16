use anyhow::{bail, Result};
use clap::{Parser, Subcommand};
use codemap::db::Db;
use codemap::query;
use codemap::types::SymbolKind;
use std::path::{Path, PathBuf};

#[derive(Parser)]
#[command(name = "codemap", version, about = "Deterministic code index for AI agents")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Environment diagnostics (detect-only; never installs).
    Doctor,
    /// Create/open the .codemap/index.db and apply migrations.
    Init {
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    /// Index (full, Tier0) all supported files under PATH.
    Index {
        #[arg(default_value = ".")]
        path: PathBuf,
        /// After Tier0, ingest a SCIP index for resolved edges (Tier1).
        #[arg(long)]
        tier1: bool,
        /// Path to the .scip file (default: <PATH>/index.scip with --tier1).
        #[arg(long)]
        scip: Option<PathBuf>,
    },
    /// Resolve a name/name_path to symbol ids (no code).
    Resolve {
        query: String,
        #[arg(long, default_value_t = 25)]
        limit: i64,
        #[arg(long, default_value = ".")]
        root: PathBuf,
    },
    /// File outline (symbols by line, no code).
    Outline {
        file: String,
        #[arg(long, default_value = ".")]
        root: PathBuf,
    },
    /// Read one symbol's code (minimal range). Accepts `sym:N` or `N`.
    ReadSymbol {
        id: String,
        #[arg(long, default_value = ".")]
        root: PathBuf,
    },
    /// Functions that call a symbol (resolved edges, no code).
    Callers {
        symbol: String,
        #[arg(long, default_value_t = 1)]
        depth: i64,
        #[arg(long, default_value_t = 50)]
        limit: i64,
        #[arg(long, default_value = ".")]
        root: PathBuf,
    },
    /// Functions a symbol calls (resolved edges, no code).
    Callees {
        symbol: String,
        #[arg(long, default_value_t = 1)]
        depth: i64,
        #[arg(long, default_value_t = 50)]
        limit: i64,
        #[arg(long, default_value = ".")]
        root: PathBuf,
    },
    /// Run the MCP server over stdio (tools for AI agents).
    Mcp {
        #[arg(long, default_value = ".")]
        root: PathBuf,
    },
    /// Install the codemap skill into detected agent hosts (writes text files only).
    Install {
        /// Restrict to specific hosts (claude, cursor, copilot, agents, kilo). Repeatable.
        #[arg(long = "target")]
        targets: Vec<String>,
        /// Show what would be written without writing.
        #[arg(long)]
        list: bool,
        #[arg(long, default_value = ".")]
        root: PathBuf,
    },
    /// Remove the codemap skill from agent hosts.
    Uninstall {
        #[arg(long = "target")]
        targets: Vec<String>,
        #[arg(long, default_value = ".")]
        root: PathBuf,
    },
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    match Cli::parse().command {
        Command::Doctor => codemap::doctor::run(),
        Command::Init { path } => cmd_init(&path),
        Command::Index { path, tier1, scip } => cmd_index(&path, tier1, scip),
        Command::Resolve { query, limit, root } => cmd_resolve(&root, &query, limit),
        Command::Outline { file, root } => cmd_outline(&root, &file),
        Command::ReadSymbol { id, root } => cmd_read_symbol(&root, &id),
        Command::Callers { symbol, depth, limit, root } => cmd_edges(&root, &symbol, depth, limit, false),
        Command::Callees { symbol, depth, limit, root } => cmd_edges(&root, &symbol, depth, limit, true),
        Command::Mcp { root } => cmd_mcp(&root),
        Command::Install { targets, list, root } => cmd_install(&root, &targets, list),
        Command::Uninstall { targets, root } => cmd_uninstall(&root, &targets),
    }
}

fn parse_targets(ids: &[String]) -> Result<Vec<codemap::skills::Target>> {
    ids.iter()
        .map(|s| codemap::skills::Target::from_id(s).ok_or_else(|| anyhow::anyhow!("unknown target: {s}")))
        .collect()
}

fn cmd_install(root: &Path, targets: &[String], list: bool) -> Result<()> {
    let only = parse_targets(targets)?;
    let reports = codemap::skills::install(root, &only, list)?;
    if reports.is_empty() {
        println!("codemap: no agent hosts detected (try --target claude|cursor|copilot|agents|kilo)");
    }
    for r in &reports {
        println!("{:9} {:?}  {}", r.target, r.action, r.path);
    }
    Ok(())
}

fn cmd_uninstall(root: &Path, targets: &[String]) -> Result<()> {
    let only = parse_targets(targets)?;
    let reports = codemap::skills::uninstall(root, &only)?;
    for r in &reports {
        println!("{:9} {:?}  {}", r.target, r.action, r.path);
    }
    Ok(())
}

fn cmd_mcp(root: &Path) -> Result<()> {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(codemap::mcp::serve_stdio(root.to_path_buf()))
}

fn db_path(root: &Path) -> PathBuf {
    root.join(".codemap").join("index.db")
}

fn open_existing(root: &Path) -> Result<Db> {
    let p = db_path(root);
    if !p.exists() {
        bail!("index not found at {} — run `codemap index` first", p.display());
    }
    Db::open(&p)
}

fn cmd_init(path: &Path) -> Result<()> {
    let dir = path.join(".codemap");
    std::fs::create_dir_all(&dir)?;
    let p = dir.join("index.db");
    let _db = Db::open(&p)?;
    println!("codemap: initialized index at {} (schema v{})", p.display(), codemap::db::SCHEMA_VERSION);
    Ok(())
}

fn cmd_index(path: &Path, tier1: bool, scip: Option<PathBuf>) -> Result<()> {
    std::fs::create_dir_all(path.join(".codemap"))?;
    let mut db = Db::open(&db_path(path))?;
    let stats = codemap::index::index_full(&mut db, path)?;
    let edges = codemap::index::resolve_calls(&mut db, path)?;
    println!("codemap: indexed {} files, {} symbols, {} call edges (Tier0)", stats.files, stats.symbols, edges);

    if tier1 || scip.is_some() {
        let scip_path = scip.unwrap_or_else(|| path.join("index.scip"));
        if !scip_path.exists() {
            bail!(
                "Tier1 needs a SCIP index at {} — generate it yourself, then re-run with --scip <path>.\n\
                 codemap never installs/runs the indexer. See `codemap doctor` for the per-language command.",
                scip_path.display()
            );
        }
        let s = codemap::scip::ingest(&mut db, &scip_path)?;
        println!(
            "codemap: Tier1 ingested {} ({} documents, {} files covered, {} resolved edges)",
            scip_path.display(),
            s.documents,
            s.covered_files,
            s.edges
        );
    }
    Ok(())
}

fn cmd_resolve(root: &Path, query: &str, limit: i64) -> Result<()> {
    let db = open_existing(root)?;
    let hits = query::resolve(&db, query, limit)?;
    println!("# resolve \"{query}\"  ({} matches)", hits.len());
    println!("# fields: id | name_path | file:line | kind");
    for h in &hits {
        println!("sym:{} | {} | {}:{} | {}", h.id, h.name_path, h.file, h.line, kind_label(h.kind));
    }
    if !hits.is_empty() {
        println!("# next: read-symbol <id> for code");
    }
    Ok(())
}

fn cmd_outline(root: &Path, file: &str) -> Result<()> {
    let db = open_existing(root)?;
    let hits = query::outline(&db, file)?;
    println!("# outline {file}  ({} symbols)", hits.len());
    for h in &hits {
        println!("sym:{} | {} | :{} | {}", h.id, h.name_path, h.line, kind_label(h.kind));
    }
    Ok(())
}

fn cmd_read_symbol(root: &Path, id_arg: &str) -> Result<()> {
    let mut db = open_existing(root)?;
    let id = query::resolve_arg(&db, id_arg)?;
    let c = query::read_symbol(&mut db, root, id)?;
    let state = if c.reindexed { " (reindexed)" } else { "" };
    println!("# sym:{} {}  {}:{}-{}{}", c.id, c.name_path, c.file, c.start_line, c.end_line, state);
    for (i, line) in c.code.lines().enumerate() {
        println!("{:>5}  {line}", c.start_line as usize + i);
    }
    Ok(())
}

fn cmd_edges(root: &Path, symbol: &str, depth: i64, limit: i64, forward: bool) -> Result<()> {
    let db = open_existing(root)?;
    let id = query::resolve_arg(&db, symbol)?;
    let hits = if forward {
        query::callees(&db, id, depth, limit)?
    } else {
        query::callers(&db, id, depth, limit)?
    };
    let label = if forward { "callees" } else { "callers" };
    println!("# {label} of sym:{id}  (depth<={depth}, {} shown)", hits.len());
    println!("# fields: id | name_path | file:line | kind | depth | prov/res");
    for h in &hits {
        let pr = match (h.provenance, h.resolution) {
            (Some(p), Some(r)) => format!("{}/{}", p.abbrev(), r.abbrev()),
            _ => "-".into(),
        };
        println!("sym:{} | {} | {}:{} | {} | {} | {}", h.id, h.name_path, h.file, h.line, kind_label(h.kind), h.depth, pr);
    }
    Ok(())
}

fn kind_label(k: Option<SymbolKind>) -> String {
    k.map(|k| format!("{k:?}").to_lowercase()).unwrap_or_else(|| "?".into())
}
