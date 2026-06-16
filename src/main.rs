use anyhow::{bail, Result};
use clap::{Parser, Subcommand};
use codemap::db::Db;
use codemap::query;
use codemap::types::SymbolKind;
use std::path::{Path, PathBuf};

#[derive(Parser)]
#[command(
    name = "codemap",
    version,
    about = "Deterministic code index for AI agents"
)]
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
        /// Only reindex files changed since the last index (mtime/size).
        #[arg(long)]
        incremental: bool,
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
    /// Search symbols by name (FTS5). mode = symbol|text|semantic.
    Search {
        query: String,
        #[arg(long, default_value = "symbol")]
        mode: String,
        #[arg(long, default_value_t = 30)]
        limit: i64,
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
    /// Transitive callers — what breaks if you change a symbol.
    Impact {
        symbol: String,
        #[arg(long, default_value_t = 4)]
        depth: i64,
        #[arg(long, default_value_t = 80)]
        limit: i64,
        #[arg(long, default_value = ".")]
        root: PathBuf,
    },
    /// Trace the call chain up to root entrypoints.
    Trace {
        symbol: String,
        #[arg(long = "max-depth", default_value_t = 6)]
        max_depth: i64,
        #[arg(long, default_value_t = 40)]
        limit: i64,
        #[arg(long, default_value = ".")]
        root: PathBuf,
    },
    /// Where a symbol is referenced (resolved to the enclosing symbol).
    Refs {
        symbol: String,
        #[arg(long, default_value_t = 100)]
        limit: i64,
        #[arg(long, default_value = ".")]
        root: PathBuf,
    },
    /// Fields/consts/variables declared under a scope (name_path).
    Variables {
        scope: String,
        #[arg(long, default_value_t = 100)]
        limit: i64,
        #[arg(long, default_value = ".")]
        root: PathBuf,
    },
    /// Export a call subgraph around a symbol as DOT or Mermaid.
    Export {
        symbol: String,
        #[arg(long, default_value = "dot")]
        format: String,
        #[arg(long, default_value_t = 2)]
        depth: i64,
        /// Traverse callers instead of callees.
        #[arg(long)]
        callers: bool,
        #[arg(long, default_value = ".")]
        root: PathBuf,
    },
    /// Watch the repo and incrementally reindex on changes (Ctrl-C to stop).
    Watch {
        #[arg(default_value = ".")]
        path: PathBuf,
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
        /// Also install opt-in git hooks (post-commit/merge/checkout -> incremental index).
        #[arg(long)]
        hooks: bool,
        #[arg(long, default_value = ".")]
        root: PathBuf,
    },
    /// Remove the codemap skill from agent hosts.
    Uninstall {
        #[arg(long = "target")]
        targets: Vec<String>,
        #[arg(long)]
        hooks: bool,
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
        Command::Index {
            path,
            incremental,
            tier1,
            scip,
        } => cmd_index(&path, incremental, tier1, scip),
        Command::Resolve { query, limit, root } => cmd_resolve(&root, &query, limit),
        Command::Outline { file, root } => cmd_outline(&root, &file),
        Command::Search {
            query,
            mode,
            limit,
            root,
        } => cmd_search(&root, &query, &mode, limit),
        Command::ReadSymbol { id, root } => cmd_read_symbol(&root, &id),
        Command::Callers {
            symbol,
            depth,
            limit,
            root,
        } => cmd_edges(&root, &symbol, depth, limit, false),
        Command::Callees {
            symbol,
            depth,
            limit,
            root,
        } => cmd_edges(&root, &symbol, depth, limit, true),
        Command::Impact {
            symbol,
            depth,
            limit,
            root,
        } => cmd_impact(&root, &symbol, depth, limit),
        Command::Trace {
            symbol,
            max_depth,
            limit,
            root,
        } => cmd_trace(&root, &symbol, max_depth, limit),
        Command::Refs {
            symbol,
            limit,
            root,
        } => cmd_refs(&root, &symbol, limit),
        Command::Variables { scope, limit, root } => cmd_variables(&root, &scope, limit),
        Command::Export {
            symbol,
            format,
            depth,
            callers,
            root,
        } => cmd_export(&root, &symbol, &format, depth, callers),
        Command::Watch { path } => {
            std::fs::create_dir_all(path.join(".codemap"))?;
            codemap::index::watch(&path)
        }
        Command::Mcp { root } => cmd_mcp(&root),
        Command::Install {
            targets,
            list,
            hooks,
            root,
        } => cmd_install(&root, &targets, list, hooks),
        Command::Uninstall {
            targets,
            hooks,
            root,
        } => cmd_uninstall(&root, &targets, hooks),
    }
}

fn parse_targets(ids: &[String]) -> Result<Vec<codemap::skills::Target>> {
    ids.iter()
        .map(|s| {
            codemap::skills::Target::from_id(s)
                .ok_or_else(|| anyhow::anyhow!("unknown target: {s}"))
        })
        .collect()
}

fn cmd_install(root: &Path, targets: &[String], list: bool, hooks: bool) -> Result<()> {
    let only = parse_targets(targets)?;
    let mut reports = codemap::skills::install(root, &only, list)?;
    if !list && hooks {
        reports.extend(codemap::skills::install_hooks(root)?);
    }
    if reports.is_empty() {
        println!("codemap: no agent hosts detected (try --target claude|cursor|copilot|agents|kilo, or --hooks)");
    }
    for r in &reports {
        println!("{:9} {:?}  {}", r.target, r.action, r.path);
    }
    Ok(())
}

fn cmd_uninstall(root: &Path, targets: &[String], hooks: bool) -> Result<()> {
    let only = parse_targets(targets)?;
    let mut reports = codemap::skills::uninstall(root, &only)?;
    if hooks {
        reports.extend(codemap::skills::uninstall_hooks(root)?);
    }
    for r in &reports {
        println!("{:9} {:?}  {}", r.target, r.action, r.path);
    }
    Ok(())
}

fn cmd_export(root: &Path, symbol: &str, format: &str, depth: i64, callers: bool) -> Result<()> {
    let db = open_existing(root)?;
    let id = query::resolve_arg(&db, symbol)?;
    let g = query::subgraph(&db, id, depth, !callers)?;
    let out = match format {
        "dot" => codemap::export::to_dot(&g),
        "mermaid" => codemap::export::to_mermaid(&g),
        other => bail!("unknown format {other:?} (use dot|mermaid)"),
    };
    print!("{out}");
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
        bail!(
            "index not found at {} — run `codemap index` first",
            p.display()
        );
    }
    Db::open(&p)
}

fn cmd_init(path: &Path) -> Result<()> {
    let dir = path.join(".codemap");
    std::fs::create_dir_all(&dir)?;
    let p = dir.join("index.db");
    let _db = Db::open(&p)?;
    println!(
        "codemap: initialized index at {} (schema v{})",
        p.display(),
        codemap::db::SCHEMA_VERSION
    );
    Ok(())
}

fn cmd_index(path: &Path, incremental: bool, tier1: bool, scip: Option<PathBuf>) -> Result<()> {
    std::fs::create_dir_all(path.join(".codemap"))?;
    let mut db = Db::open(&db_path(path))?;
    if incremental {
        let r = codemap::index::reconcile(&mut db, path)?;
        println!(
            "codemap: incremental — {} changed, {} added, {} deleted, {} unchanged (Tier0)",
            r.changed, r.added, r.deleted, r.unchanged
        );
    } else {
        let stats = codemap::index::index_full(&mut db, path)?;
        let edges = codemap::index::resolve_calls(&mut db, path)?;
        println!(
            "codemap: indexed {} files, {} symbols, {} call edges (Tier0)",
            stats.files, stats.symbols, edges
        );
    }

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
        println!(
            "sym:{} | {} | {}:{} | {}",
            h.id,
            h.name_path,
            h.file,
            h.line,
            kind_label(h.kind)
        );
    }
    if !hits.is_empty() {
        println!("# next: read-symbol <id> for code");
    }
    Ok(())
}

fn cmd_search(root: &Path, query: &str, mode: &str, limit: i64) -> Result<()> {
    let db = open_existing(root)?;
    let hits = query::search(&db, query, mode, limit)?;
    println!(
        "# search \"{query}\"  (mode={mode}, {} matches)",
        hits.len()
    );
    println!("# fields: id | name_path | file:line | kind");
    for h in &hits {
        println!(
            "sym:{} | {} | {}:{} | {}",
            h.id,
            h.name_path,
            h.file,
            h.line,
            kind_label(h.kind)
        );
    }
    Ok(())
}

fn cmd_outline(root: &Path, file: &str) -> Result<()> {
    let db = open_existing(root)?;
    let hits = query::outline(&db, file)?;
    println!("# outline {file}  ({} symbols)", hits.len());
    for h in &hits {
        println!(
            "sym:{} | {} | :{} | {}",
            h.id,
            h.name_path,
            h.line,
            kind_label(h.kind)
        );
    }
    Ok(())
}

fn cmd_read_symbol(root: &Path, id_arg: &str) -> Result<()> {
    let mut db = open_existing(root)?;
    let id = query::resolve_arg(&db, id_arg)?;
    let c = query::read_symbol(&mut db, root, id)?;
    let state = if c.reindexed { " (reindexed)" } else { "" };
    println!(
        "# sym:{} {}  {}:{}-{}{}",
        c.id, c.name_path, c.file, c.start_line, c.end_line, state
    );
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
    print_edges(if forward { "callees of" } else { "callers of" }, id, &hits);
    Ok(())
}

fn cmd_impact(root: &Path, symbol: &str, depth: i64, limit: i64) -> Result<()> {
    let db = open_existing(root)?;
    let id = query::resolve_arg(&db, symbol)?;
    let hits = query::impact(&db, id, depth, limit)?;
    print_edges("impact of", id, &hits);
    Ok(())
}

fn cmd_trace(root: &Path, symbol: &str, max_depth: i64, limit: i64) -> Result<()> {
    let db = open_existing(root)?;
    let id = query::resolve_arg(&db, symbol)?;
    let hits = query::trace_to_roots(&db, id, max_depth, limit)?;
    print_edges("roots reaching", id, &hits);
    Ok(())
}

fn cmd_refs(root: &Path, symbol: &str, limit: i64) -> Result<()> {
    let db = open_existing(root)?;
    let id = query::resolve_arg(&db, symbol)?;
    let refs = query::references(&db, id, limit)?;
    println!("# references to sym:{id}  ({} shown)", refs.len());
    println!("# fields: in_symbol | file:line | role");
    for r in &refs {
        let enc = r.enclosing.as_deref().unwrap_or("(top-level)");
        let role = r
            .role
            .map(|x| format!("{x:?}").to_lowercase())
            .unwrap_or_else(|| "?".into());
        println!("{enc} | {}:{} | {role}", r.file, r.line);
    }
    Ok(())
}

fn cmd_variables(root: &Path, scope: &str, limit: i64) -> Result<()> {
    let db = open_existing(root)?;
    let hits = query::variables(&db, scope, limit)?;
    println!("# variables in {scope}  ({} shown)", hits.len());
    println!("# fields: id | name_path | file:line | kind");
    for h in &hits {
        println!(
            "sym:{} | {} | {}:{} | {}",
            h.id,
            h.name_path,
            h.file,
            h.line,
            kind_label(h.kind)
        );
    }
    Ok(())
}

fn print_edges(label: &str, id: i64, hits: &[query::EdgeHit]) {
    println!("# {label} sym:{id}  ({} shown)", hits.len());
    println!("# fields: id | name_path | file:line | kind | depth | prov/res");
    for h in hits {
        let pr = match (h.provenance, h.resolution) {
            (Some(p), Some(r)) => format!("{}/{}", p.abbrev(), r.abbrev()),
            _ => "-".into(),
        };
        println!(
            "sym:{} | {} | {}:{} | {} | {} | {}",
            h.id,
            h.name_path,
            h.file,
            h.line,
            kind_label(h.kind),
            h.depth,
            pr
        );
    }
}

fn kind_label(k: Option<SymbolKind>) -> String {
    k.map(|k| format!("{k:?}").to_lowercase())
        .unwrap_or_else(|| "?".into())
}
