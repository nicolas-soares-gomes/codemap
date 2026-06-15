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
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    match Cli::parse().command {
        Command::Doctor => codemap::doctor::run(),
        Command::Init { path } => cmd_init(&path),
        Command::Index { path } => cmd_index(&path),
        Command::Resolve { query, limit, root } => cmd_resolve(&root, &query, limit),
        Command::Outline { file, root } => cmd_outline(&root, &file),
        Command::ReadSymbol { id, root } => cmd_read_symbol(&root, &id),
    }
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

fn cmd_index(path: &Path) -> Result<()> {
    std::fs::create_dir_all(path.join(".codemap"))?;
    let mut db = Db::open(&db_path(path))?;
    let stats = codemap::index::index_full(&mut db, path)?;
    println!("codemap: indexed {} files, {} symbols", stats.files, stats.symbols);
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
    let id: i64 = id_arg
        .strip_prefix("sym:")
        .unwrap_or(id_arg)
        .parse()
        .map_err(|_| anyhow::anyhow!("invalid id: {id_arg} (use sym:N or N)"))?;
    let db = open_existing(root)?;
    let c = query::read_symbol(&db, root, id)?;
    println!("# sym:{} {}  {}:{}-{}", c.id, c.name_path, c.file, c.start_line, c.end_line);
    for (i, line) in c.code.lines().enumerate() {
        println!("{:>5}  {line}", c.start_line as usize + i);
    }
    Ok(())
}

fn kind_label(k: Option<SymbolKind>) -> String {
    k.map(|k| format!("{k:?}").to_lowercase()).unwrap_or_else(|| "?".into())
}
