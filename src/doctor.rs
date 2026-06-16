//! `codemap doctor` — detect-only diagnostics. Never installs anything (firm policy): it
//! probes for external SCIP indexers / language servers and prints install tips.
//!
//! The CAPS table is the single source of truth for per-language capability, consumed by
//! doctor (and, later, by Tier1/Tier2 resolution and steering errors).

use anyhow::Result;
use std::process::{Command, Stdio};

struct LangCaps {
    lang: &'static str,
    tier1_indexer: &'static str, // empty = no SCIP indexer
    tier1_needs_build: bool,
    tier1_bin: &'static str,
    tier1_tip: &'static str,
    tier2_lsp: &'static str,
    tier2_bin: &'static str,
}

const CAPS: &[LangCaps] = &[
    LangCaps { lang: "rust", tier1_indexer: "rust-analyzer scip", tier1_needs_build: false, tier1_bin: "rust-analyzer", tier1_tip: "rustup component add rust-analyzer", tier2_lsp: "rust-analyzer", tier2_bin: "rust-analyzer" },
    LangCaps { lang: "ts/js", tier1_indexer: "scip-typescript index", tier1_needs_build: false, tier1_bin: "scip-typescript", tier1_tip: "npm i -g @sourcegraph/scip-typescript", tier2_lsp: "typescript-language-server", tier2_bin: "typescript-language-server" },
    LangCaps { lang: "python", tier1_indexer: "scip-python index", tier1_needs_build: false, tier1_bin: "scip-python", tier1_tip: "npm i -g @sourcegraph/scip-python", tier2_lsp: "pyright", tier2_bin: "pyright-langserver" },
    LangCaps { lang: "go", tier1_indexer: "scip-go", tier1_needs_build: false, tier1_bin: "scip-go", tier1_tip: "go install github.com/sourcegraph/scip-go/cmd/scip-go@latest", tier2_lsp: "gopls", tier2_bin: "gopls" },
    LangCaps { lang: "java", tier1_indexer: "scip-java index", tier1_needs_build: true, tier1_bin: "scip-java", tier1_tip: "cs install scip-java (needs JDK + gradle/maven; project must compile)", tier2_lsp: "jdtls", tier2_bin: "jdtls" },
    LangCaps { lang: "c/c++", tier1_indexer: "scip-clang", tier1_needs_build: true, tier1_bin: "scip-clang", tier1_tip: "install scip-clang; needs compile_commands.json (cmake -DCMAKE_EXPORT_COMPILE_COMMANDS=ON)", tier2_lsp: "clangd", tier2_bin: "clangd" },
    LangCaps { lang: "c#", tier1_indexer: "scip-dotnet index", tier1_needs_build: true, tier1_bin: "scip-dotnet", tier1_tip: "install scip-dotnet; needs .NET 8 SDK + restored solution", tier2_lsp: "csharp-ls", tier2_bin: "csharp-ls" },
    LangCaps { lang: "kotlin", tier1_indexer: "scip-java (semanticdb)", tier1_needs_build: true, tier1_bin: "scip-java", tier1_tip: "scip-kotlin via SemanticDB plugin; needs compiling build", tier2_lsp: "kotlin-lsp", tier2_bin: "kotlin-lsp" },
    LangCaps { lang: "php", tier1_indexer: "scip-php (3rd-party)", tier1_needs_build: false, tier1_bin: "scip-php", tier1_tip: "composer require --dev davidrjenni/scip-php", tier2_lsp: "intelephense", tier2_bin: "intelephense" },
    LangCaps { lang: "swift", tier1_indexer: "", tier1_needs_build: false, tier1_bin: "", tier1_tip: "no SCIP indexer; use sourcekit-lsp (Tier2)", tier2_lsp: "sourcekit-lsp", tier2_bin: "sourcekit-lsp" },
    LangCaps { lang: "clojure", tier1_indexer: "", tier1_needs_build: false, tier1_bin: "", tier1_tip: "no SCIP indexer; clj-kondo bridge is opt-in (post-MVP)", tier2_lsp: "clojure-lsp", tier2_bin: "clojure-lsp" },
];

pub fn run() -> Result<()> {
    println!("codemap doctor (detect-only — never installs)");
    println!("  schema_version: {}", crate::db::SCHEMA_VERSION);
    println!("  git: {}", state(present("git")));
    println!();
    println!(
        "  {:<8} {:<6} {:<18} tier2 (LSP)",
        "lang", "tier0", "tier1 (SCIP)"
    );
    let mut tips: Vec<String> = Vec::new();
    for c in CAPS {
        let (t1, tip) = tier1_state(c);
        if let Some(tip) = tip {
            tips.push(tip);
        }
        let t2 = if c.tier2_bin.is_empty() {
            "-".to_string()
        } else if present(c.tier2_bin) {
            format!("{} ok", c.tier2_lsp)
        } else {
            format!("{} missing", c.tier2_lsp)
        };
        // Kotlin/Clojure have no compatible tree-sitter grammar yet (pre-1.0 / version pin).
        let t0 = if matches!(c.lang, "kotlin" | "clojure") {
            "n/a"
        } else {
            "ok"
        };
        println!("  {:<8} {:<6} {:<18} {}", c.lang, t0, t1, t2);
    }
    if !tips.is_empty() {
        println!(
            "\n  tips for missing Tier1 indexers (run them yourself; codemap never installs):"
        );
        for t in tips {
            println!("    - {t}");
        }
    }
    println!("\n  Missing Tier1/Tier2 is fine — codemap works on Tier0 (tree-sitter) alone.");
    Ok(())
}

/// Short matrix cell + an optional install tip for the tips section.
fn tier1_state(c: &LangCaps) -> (String, Option<String>) {
    if c.tier1_bin.is_empty() {
        return ("none".into(), None);
    }
    let build = if c.tier1_needs_build { "/build" } else { "" };
    if present(c.tier1_bin) {
        (format!("ok{build}"), None)
    } else {
        (
            format!("missing{build}"),
            Some(format!("{}: {}", c.lang, c.tier1_tip)),
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
