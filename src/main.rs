use anyhow::{bail, Result};
use clap::{Parser, Subcommand};
use codemap::db::Db;
use codemap::query::{self, project};
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
    /// Index state: commit, scan mode, counts, db size, SCIP coverage.
    Status {
        #[arg(long, default_value = ".")]
        root: PathBuf,
    },
    /// Remove files from the index that no longer exist on disk; --gc sweeps orphan strings.
    Prune {
        #[arg(long)]
        gc: bool,
        #[arg(long, default_value = ".")]
        root: PathBuf,
    },
    /// Delete the index (.codemap/index.db) — it is a rebuildable cache.
    Reset {
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    /// Show a symbol's range/kind plus caller/callee counts.
    Inspect {
        symbol: String,
        #[arg(long, default_value = ".")]
        root: PathBuf,
    },
    /// Print the external SCIP indexer command for a language (never runs it).
    ScipCmd { lang: String },
    /// Index all supported files under PATH (tree-sitter).
    Index {
        #[arg(default_value = ".")]
        path: PathBuf,
        /// Only reindex files changed since the last run (git or mtime/size).
        #[arg(long)]
        incremental: bool,
        /// Also ingest a SCIP index for precise edges (defaults to <PATH>/index.scip).
        #[arg(long)]
        with_scip: bool,
        /// Path to the .scip file (implies --with-scip).
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
        Command::Status { root } => cmd_status(&root),
        Command::Prune { gc, root } => cmd_prune(&root, gc),
        Command::Reset { path } => cmd_reset(&path),
        Command::Inspect { symbol, root } => cmd_inspect(&root, &symbol),
        Command::ScipCmd { lang } => cmd_scip_cmd(&lang),
        Command::Index {
            path,
            incremental,
            with_scip,
            scip,
        } => cmd_index(&path, incremental, with_scip, scip),
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

fn cmd_status(root: &Path) -> Result<()> {
    let db = open_existing(root)?;
    let p = db_path(root);
    let size = std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
    let meta = |k: &str| db.get_meta(k).ok().flatten().unwrap_or_else(|| "-".into());
    println!("codemap status ({})", p.display());
    println!("  last commit:   {}", meta("indexed_commit"));
    println!("  scan mode:     {}", meta("scanner_mode"));
    println!("  scip coverage: {}", meta("scip_coverage"));
    println!("  files:         {}", db.count("file")?);
    println!("  symbols:       {}", db.count("symbol")?);
    println!("  occurrences:   {}", db.count("occurrence")?);
    println!("  call edges:    {}", db.count("edge")?);
    println!("  index size:    {} KB", size / 1024);
    Ok(())
}

fn cmd_prune(root: &Path, gc: bool) -> Result<()> {
    let mut db = open_existing(root)?;
    let pruned = codemap::index::prune(&mut db, root)?;
    let mut msg = format!("codemap: pruned {pruned} missing file(s)");
    if gc {
        let swept = db.gc()?;
        msg.push_str(&format!(", swept {swept} unused string(s)"));
    }
    println!("{msg}");
    Ok(())
}

fn cmd_reset(path: &Path) -> Result<()> {
    let p = db_path(path);
    for suffix in ["", "-wal", "-shm"] {
        let f = PathBuf::from(format!("{}{}", p.display(), suffix));
        let _ = std::fs::remove_file(&f);
    }
    println!(
        "codemap: index removed ({}) — rebuild with `codemap index`",
        p.display()
    );
    Ok(())
}

fn cmd_inspect(root: &Path, symbol: &str) -> Result<()> {
    let db = open_existing(root)?;
    let id = query::resolve_arg(&db, symbol)?;
    let (name_path, file, start, end, kind): (String, String, u32, u32, i64) = db.conn.query_row(
        "SELECT np.text, fp.text, s.start_line, s.end_line, s.kind
         FROM symbol s
         JOIN string_pool np ON np.id = s.name_path_sid
         JOIN file f         ON f.id  = s.file_id
         JOIN string_pool fp ON fp.id = f.path_sid
         WHERE s.id = ?1",
        [id],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
    )?;
    let kind_s = codemap::types::SymbolKind::from_i64(kind)
        .map(|k| format!("{k:?}").to_lowercase())
        .unwrap_or_else(|| "?".into());
    let callers = query::callers(&db, id, 1, 10_000)?.len();
    let callees = query::callees(&db, id, 1, 10_000)?.len();
    println!("# sym:{id} {name_path}");
    println!("  {file}:{start}-{end}  ({kind_s})");
    println!("  callers: {callers}  callees: {callees}");
    Ok(())
}

fn cmd_scip_cmd(lang: &str) -> Result<()> {
    match codemap::doctor::scip_cmd(lang) {
        Some(s) => println!("{s}"),
        None => bail!("unknown language: {lang}"),
    }
    Ok(())
}

fn cmd_index(path: &Path, incremental: bool, with_scip: bool, scip: Option<PathBuf>) -> Result<()> {
    std::fs::create_dir_all(path.join(".codemap"))?;
    let mut db = Db::open(&db_path(path))?;
    if incremental {
        let r = codemap::index::reconcile(&mut db, path)?;
        println!(
            "codemap: incremental — {} changed, {} added, {} deleted, {} unchanged",
            r.changed, r.added, r.deleted, r.unchanged
        );
    } else {
        let stats = codemap::index::index_full(&mut db, path)?;
        let edges = codemap::index::resolve_calls(&mut db, path)?;
        println!(
            "codemap: indexed {} files, {} symbols, {} call edges",
            stats.files, stats.symbols, edges
        );
    }

    if with_scip || scip.is_some() {
        let scip_path = scip.unwrap_or_else(|| path.join("index.scip"));
        if !scip_path.exists() {
            bail!(
                "no SCIP index at {} — generate it yourself, then re-run with --scip <path>.\n\
                 codemap never runs the indexer. See `codemap scip-cmd <lang>` for the command.",
                scip_path.display()
            );
        }
        let s = codemap::scip::ingest(&mut db, &scip_path)?;
        println!(
            "codemap: SCIP ingested {} ({} docs, {}/{} files covered = {}%, {} precise edges)",
            scip_path.display(),
            s.documents,
            s.covered_files,
            s.total_files,
            s.coverage_pct(),
            s.edges
        );
        if s.coverage_pct() < 50 {
            eprintln!(
                "codemap: warning — low SCIP coverage ({}%). The indexer build may be incomplete; \
                 uncovered files keep their tree-sitter results (marked ambiguous).",
                s.coverage_pct()
            );
        }
    }
    Ok(())
}

fn cmd_resolve(root: &Path, query: &str, limit: i64) -> Result<()> {
    let db = open_existing(root)?;
    print!("{}", project::resolve(&db, query, limit)?);
    Ok(())
}

fn cmd_search(root: &Path, query: &str, mode: &str, limit: i64) -> Result<()> {
    let db = open_existing(root)?;
    print!("{}", project::search(&db, query, mode, limit)?);
    Ok(())
}

fn cmd_outline(root: &Path, file: &str) -> Result<()> {
    let db = open_existing(root)?;
    print!("{}", project::outline(&db, file)?);
    Ok(())
}

fn cmd_read_symbol(root: &Path, id_arg: &str) -> Result<()> {
    let mut db = open_existing(root)?;
    let id = query::resolve_arg(&db, id_arg)?;
    print!("{}", project::read_symbol(&mut db, root, id)?);
    Ok(())
}

fn cmd_edges(root: &Path, symbol: &str, depth: i64, limit: i64, forward: bool) -> Result<()> {
    let db = open_existing(root)?;
    let id = query::resolve_arg(&db, symbol)?;
    let label = if forward { "callees of" } else { "callers of" };
    print!("{}", project::edges(&db, label, id, depth, limit, forward)?);
    Ok(())
}

fn cmd_impact(root: &Path, symbol: &str, depth: i64, limit: i64) -> Result<()> {
    let db = open_existing(root)?;
    let id = query::resolve_arg(&db, symbol)?;
    print!("{}", project::impact(&db, id, depth, limit)?);
    Ok(())
}

fn cmd_trace(root: &Path, symbol: &str, max_depth: i64, limit: i64) -> Result<()> {
    let db = open_existing(root)?;
    let id = query::resolve_arg(&db, symbol)?;
    print!("{}", project::trace_to_roots(&db, id, max_depth, limit)?);
    Ok(())
}

fn cmd_refs(root: &Path, symbol: &str, limit: i64) -> Result<()> {
    let db = open_existing(root)?;
    let id = query::resolve_arg(&db, symbol)?;
    print!("{}", project::references(&db, id, limit)?);
    Ok(())
}

fn cmd_variables(root: &Path, scope: &str, limit: i64) -> Result<()> {
    let db = open_existing(root)?;
    print!("{}", project::variables(&db, scope, limit)?);
    Ok(())
}
