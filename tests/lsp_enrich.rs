//! Tier-2 LSP enrichment (feature `tier2-lsp`): a user-installed language server confirms call
//! edges, upgrading them to provenance=lsp/resolution=resolved.
//!
//! `#[ignore]` by default: it drives a REAL rust-analyzer, so it's slow and environment-dependent
//! (CI runners ship a rustup `rust-analyzer` shim that exits immediately). Run it explicitly with
//! `cargo test --features tier2-lsp -- --ignored` on a machine with a working rust-analyzer.
#![cfg(feature = "tier2-lsp")]

use codemap::db::Db;
use codemap::{index, lsp, query};

#[test]
#[ignore = "drives a real rust-analyzer; run with `--ignored` locally"]
fn enrich_upgrades_edges_via_rust_analyzer() {
    if !codemap::doctor::binary_present("rust-analyzer") {
        eprintln!("skipping: rust-analyzer not on PATH");
        return;
    }

    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(
        dir.path().join("Cargo.toml"),
        "[package]\nname = \"t\"\nversion = \"0.0.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("src/lib.rs"),
        "pub fn callee() -> i32 { 1 }\npub fn caller() -> i32 { callee() }\n",
    )
    .unwrap();
    std::fs::create_dir_all(dir.path().join(".codemap")).unwrap();

    let mut db = Db::open(&dir.path().join(".codemap/index.db")).unwrap();
    index::index_full(&mut db, dir.path()).unwrap();
    index::resolve_calls(&mut db, dir.path()).unwrap();

    let caller_id = query::resolve(&db, "caller", 5).unwrap()[0].id;
    let n = lsp::enrich(&mut db, dir.path(), caller_id).unwrap();
    assert!(n >= 1, "expected >=1 LSP-confirmed edge, got {n}");

    let callee_id = query::resolve(&db, "callee", 5).unwrap()[0].id;
    let (prov, res): (i64, i64) = db
        .conn
        .query_row(
            "SELECT provenance, resolution FROM edge
             WHERE source_symbol_id=?1 AND target_symbol_id=?2 AND kind=1",
            [caller_id, callee_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(prov, 3, "provenance should be lsp");
    assert_eq!(res, 0, "resolution should be resolved");
}
