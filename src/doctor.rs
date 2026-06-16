//! `codemap doctor` — read-only environment check. Never installs anything: it probes for the
//! external tools that unlock precise results (SCIP indexers, language servers) and prints how
//! to install them yourself.
//!
//! CAPS is the single source of truth for what each language supports:
//!   - parse: tree-sitter (always available; gives structure/outline/ranges)
//!   - scip:  an external indexer that produces a precise `.scip` (optional)
//!   - lsp:   a language server used on demand for precise edges (optional)

use crate::types::Language;
use anyhow::Result;
use std::collections::HashMap;
use std::path::Path;
use std::process::{Command, Stdio};

struct LangCaps {
    lang: &'static str,
    scip_indexer: &'static str, // empty = no SCIP indexer for this language
    scip_needs_build: bool,
    scip_bin: &'static str,
    scip_tip: &'static str,
    lsp_name: &'static str,
    lsp_bin: &'static str,
}

const CAPS: &[LangCaps] = &[
    LangCaps { lang: "rust", scip_indexer: "rust-analyzer scip", scip_needs_build: false, scip_bin: "rust-analyzer", scip_tip: "rustup component add rust-analyzer", lsp_name: "rust-analyzer", lsp_bin: "rust-analyzer" },
    LangCaps { lang: "ts/js", scip_indexer: "scip-typescript index", scip_needs_build: false, scip_bin: "scip-typescript", scip_tip: "npm i -g @sourcegraph/scip-typescript", lsp_name: "typescript-language-server", lsp_bin: "typescript-language-server" },
    LangCaps { lang: "python", scip_indexer: "scip-python index", scip_needs_build: false, scip_bin: "scip-python", scip_tip: "npm i -g @sourcegraph/scip-python", lsp_name: "pyright", lsp_bin: "pyright-langserver" },
    LangCaps { lang: "go", scip_indexer: "scip-go", scip_needs_build: false, scip_bin: "scip-go", scip_tip: "go install github.com/sourcegraph/scip-go/cmd/scip-go@latest", lsp_name: "gopls", lsp_bin: "gopls" },
    LangCaps { lang: "java", scip_indexer: "scip-java index", scip_needs_build: true, scip_bin: "scip-java", scip_tip: "cs install scip-java (needs JDK + gradle/maven; project must compile)", lsp_name: "jdtls", lsp_bin: "jdtls" },
    LangCaps { lang: "c/c++", scip_indexer: "scip-clang", scip_needs_build: true, scip_bin: "scip-clang", scip_tip: "install scip-clang; needs compile_commands.json (cmake -DCMAKE_EXPORT_COMPILE_COMMANDS=ON)", lsp_name: "clangd", lsp_bin: "clangd" },
    LangCaps { lang: "c#", scip_indexer: "scip-dotnet index", scip_needs_build: true, scip_bin: "scip-dotnet", scip_tip: "install scip-dotnet; needs .NET 8 SDK + restored solution", lsp_name: "csharp-ls", lsp_bin: "csharp-ls" },
    LangCaps { lang: "kotlin", scip_indexer: "scip-java (semanticdb)", scip_needs_build: true, scip_bin: "scip-java", scip_tip: "scip-kotlin via SemanticDB plugin; needs a compiling build", lsp_name: "kotlin-lsp", lsp_bin: "kotlin-lsp" },
    LangCaps { lang: "php", scip_indexer: "scip-php (3rd-party)", scip_needs_build: false, scip_bin: "scip-php", scip_tip: "composer require --dev davidrjenni/scip-php", lsp_name: "intelephense", lsp_bin: "intelephense" },
    LangCaps { lang: "swift", scip_indexer: "", scip_needs_build: false, scip_bin: "", scip_tip: "no SCIP indexer; use sourcekit-lsp", lsp_name: "sourcekit-lsp", lsp_bin: "sourcekit-lsp" },
    LangCaps { lang: "clojure", scip_indexer: "", scip_needs_build: false, scip_bin: "", scip_tip: "no SCIP indexer; use clojure-lsp", lsp_name: "clojure-lsp", lsp_bin: "clojure-lsp" },
];

/// The external SCIP indexer command + install tip for a language (codemap never runs it).
pub fn scip_cmd(lang: &str) -> Option<String> {
    let c = CAPS
        .iter()
        .find(|c| c.lang == lang || c.lang.split('/').any(|x| x == lang))?;
    Some(if c.scip_indexer.is_empty() {
        format!(
            "{}: no SCIP indexer — use the language server {}",
            c.lang, c.lsp_name
        )
    } else {
        let build = if c.scip_needs_build {
            " [needs a working build]"
        } else {
            ""
        };
        format!(
            "{}: run `{}`{build}\n  install: {}",
            c.lang, c.scip_indexer, c.scip_tip
        )
    })
}

/// The argv to launch a language server for `lang` over stdio, if codemap knows a common one.
/// codemap never installs it — this is only used when the binary is already on PATH.
pub fn lsp_invocation(lang: Language) -> Vec<String> {
    use Language::*;
    let argv: &[&str] = match lang {
        Rust => &["rust-analyzer"],
        Go => &["gopls"],
        C | Cpp => &["clangd"],
        TypeScript | JavaScript | Tsx => &["typescript-language-server", "--stdio"],
        Python => &["pyright-langserver", "--stdio"],
        Java => &["jdtls"],
        CSharp => &["csharp-ls"],
        Php => &["intelephense", "--stdio"],
        Swift => &["sourcekit-lsp"],
        Kotlin => &["kotlin-lsp", "--stdio"],
        Clojure => &["clojure-lsp"],
    };
    argv.iter().map(|s| s.to_string()).collect()
}

/// True if `bin` is on PATH (probes `bin --version`).
pub fn binary_present(bin: &str) -> bool {
    present(bin)
}

/// Map a detected language to its row key in CAPS.
fn caps_key(lang: Language) -> &'static str {
    use Language::*;
    match lang {
        Rust => "rust",
        TypeScript | JavaScript | Tsx => "ts/js",
        Python => "python",
        Go => "go",
        Java => "java",
        CSharp => "c#",
        Php => "php",
        C | Cpp => "c/c++",
        Swift => "swift",
        Kotlin => "kotlin",
        Clojure => "clojure",
    }
}

pub fn run(root: &Path) -> Result<()> {
    println!("codemap doctor (read-only — never installs anything)");
    println!("  schema_version: {}", crate::db::SCHEMA_VERSION);
    println!("  git: {}", state(present("git")));

    // Only suggest tools for languages actually present in this repo.
    let mut files_per: HashMap<&str, usize> = HashMap::new();
    for (lang, n) in crate::index::detect_repo_languages(root) {
        *files_per.entry(caps_key(lang)).or_insert(0) += n;
    }
    let scanning = !files_per.is_empty();

    println!();
    println!(
        "  {}",
        if scanning {
            "languages found in this repo:"
        } else {
            "no supported files found here — showing all languages:"
        }
    );
    println!("  parse = tree-sitter (always on)   scip = precise index (optional)   lsp = language server (optional)");
    println!(
        "  {:<8} {:<6} {:<6} {:<18} lsp",
        "lang", "files", "parse", "scip"
    );

    let mut tips: Vec<String> = Vec::new();
    for c in CAPS {
        let files = files_per.get(c.lang).copied();
        if scanning && files.is_none() {
            continue;
        }
        let (scip, tip) = scip_state(c);
        if let Some(tip) = tip {
            tips.push(tip);
        }
        let lsp = if c.lsp_bin.is_empty() {
            "-".to_string()
        } else if present(c.lsp_bin) {
            format!("{} ok", c.lsp_name)
        } else {
            format!("{} missing", c.lsp_name)
        };
        let files_s = files.map(|n| n.to_string()).unwrap_or_else(|| "-".into());
        println!(
            "  {:<8} {:<6} {:<6} {:<18} {}",
            c.lang, files_s, "ok", scip, lsp
        );
    }
    if !tips.is_empty() {
        println!(
            "\n  to enable precise results, run these yourself (codemap never installs them):"
        );
        for t in tips {
            println!("    - {t}");
        }
    }

    // In a monorepo, each build root is its own index unit — generate one .scip per unit and
    // ingest them together with repeated `--scip` flags.
    let units = crate::index::detect_index_units(root);
    if units.len() > 1 {
        println!("\n  build roots (index units) — generate a .scip per unit:");
        for (path, kind) in &units {
            println!("    {path:<28} ({kind})");
        }
    }

    println!("\n  Missing scip/lsp is fine — codemap works on tree-sitter alone.");
    Ok(())
}

/// Short matrix cell for the scip column + an optional install tip.
fn scip_state(c: &LangCaps) -> (String, Option<String>) {
    if c.scip_bin.is_empty() {
        return ("none".into(), None);
    }
    let build = if c.scip_needs_build { "/build" } else { "" };
    if present(c.scip_bin) {
        (format!("ok{build}"), None)
    } else {
        (
            format!("missing{build}"),
            Some(format!("{}: {}", c.lang, c.scip_tip)),
        )
    }
}

fn state(ok: bool) -> &'static str {
    if ok {
        "ok"
    } else {
        "missing"
    }
}

/// True if `bin` exists on PATH (probes `bin --version`; spawn failure = absent).
fn present(bin: &str) -> bool {
    Command::new(bin)
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .stdin(Stdio::null())
        .status()
        .is_ok()
}
