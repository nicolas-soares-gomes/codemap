//! Polyglot monorepo: files map to their nearest build root ("index unit"), and several SCIP
//! indexes (one per unit) ingest together, each mapped through its project-root prefix.

use ::scip::types::{Document, Index, Metadata, Occurrence};
use codemap::db::Db;
use codemap::index;
use protobuf::{Message, MessageField};
use std::fs;
use std::path::Path;

fn occ(symbol: &str, line0: i32, role: i32) -> Occurrence {
    let mut o = Occurrence::new();
    o.range = vec![line0, 3, 12];
    o.symbol = symbol.into();
    o.symbol_roles = role;
    o
}

/// A single-document SCIP index whose project root is `unit_abs` (so its `src/lib.rs` resolves
/// to `<unit>/src/lib.rs` in the repo) with a derivable `caller -> callee` edge.
fn unit_scip(unit_abs: &Path, callee: &str, caller: &str) -> Vec<u8> {
    let mut meta = Metadata::new();
    meta.project_root = format!("file://{}", unit_abs.display());

    let mut doc = Document::new();
    doc.relative_path = "src/lib.rs".into();
    doc.occurrences = vec![
        occ(&format!("rust::{callee}#"), 0, 1), // definition
        occ(&format!("rust::{caller}#"), 1, 1), // definition
        occ(&format!("rust::{callee}#"), 2, 0), // reference inside caller
    ];

    let mut idx = Index::new();
    idx.metadata = MessageField::some(meta);
    idx.documents = vec![doc];
    idx.write_to_bytes().unwrap()
}

fn write_crate(root: &Path, name: &str, callee: &str, caller: &str) {
    let dir = root.join("crates").join(name);
    fs::create_dir_all(dir.join("src")).unwrap();
    fs::write(
        dir.join("Cargo.toml"),
        format!("[package]\nname = \"{name}\"\n"),
    )
    .unwrap();
    fs::write(
        dir.join("src/lib.rs"),
        format!("fn {callee}() {{}}\nfn {caller}() {{\n    {callee}();\n}}\n"),
    )
    .unwrap();
}

fn unit_of(db: &Db, rel: &str) -> String {
    db.conn
        .query_row(
            "SELECT u.text FROM file f
             JOIN string_pool fp ON fp.id = f.path_sid
             JOIN string_pool u  ON u.id  = f.index_unit_sid
             WHERE fp.text = ?1",
            [rel],
            |r| r.get(0),
        )
        .unwrap()
}

#[test]
fn files_map_to_nearest_build_root() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write_crate(root, "alpha", "helper", "run");
    write_crate(root, "beta", "util", "go");
    fs::create_dir_all(root.join(".codemap")).unwrap();

    let mut db = Db::open(&root.join(".codemap/index.db")).unwrap();
    index::index_full(&mut db, root).unwrap();

    assert_eq!(unit_of(&db, "crates/alpha/src/lib.rs"), "crates/alpha");
    assert_eq!(unit_of(&db, "crates/beta/src/lib.rs"), "crates/beta");

    let units = index::detect_index_units(root);
    assert!(units
        .iter()
        .any(|(p, k)| p == "crates/alpha" && k == "cargo"));
    assert!(units
        .iter()
        .any(|(p, k)| p == "crates/beta" && k == "cargo"));
}

#[test]
fn ingests_one_scip_per_unit_with_prefix_mapping() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write_crate(root, "alpha", "helper", "run");
    write_crate(root, "beta", "util", "go");
    fs::create_dir_all(root.join(".codemap")).unwrap();

    let mut db = Db::open(&root.join(".codemap/index.db")).unwrap();
    index::index_full(&mut db, root).unwrap();
    index::resolve_calls(&mut db, root).unwrap();

    let alpha = root.join("alpha.scip");
    let beta = root.join("beta.scip");
    fs::write(
        &alpha,
        unit_scip(&root.join("crates/alpha"), "helper", "run"),
    )
    .unwrap();
    fs::write(&beta, unit_scip(&root.join("crates/beta"), "util", "go")).unwrap();

    let stats = codemap::scip::ingest(&mut db, root, &[alpha, beta]).unwrap();
    assert_eq!(stats.covered_files, 2, "both units covered");
    assert!(stats.edges >= 2, "one resolved edge per unit");

    // Both units have a resolved SCIP call edge from caller to callee.
    for (caller, callee) in [("run", "helper"), ("go", "util")] {
        let res: i64 = db
            .conn
            .query_row(
                "SELECT e.resolution FROM edge e
                 JOIN symbol src ON src.id = e.source_symbol_id
                 JOIN symbol dst ON dst.id = e.target_symbol_id
                 JOIN string_pool sn ON sn.id = src.name_sid
                 JOIN string_pool dn ON dn.id = dst.name_sid
                 WHERE sn.text = ?1 AND dn.text = ?2 AND e.provenance = 2",
                [caller, callee],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(res, 0, "{caller} -> {callee} should be resolved by SCIP");
    }
}
