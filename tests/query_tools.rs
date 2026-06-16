//! search_code, impact, trace_to_roots, get_references, get_variables.

use codemap::db::Db;
use codemap::{index, query};
use std::fs;

const SRC: &str = "\
pub struct Cfg {
    pub max: u32,
}
fn leaf() {}
fn mid() { leaf(); }
fn root_fn() { mid(); }
";

fn setup() -> (tempfile::TempDir, Db) {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join("src")).unwrap();
    fs::create_dir_all(dir.path().join(".codemap")).unwrap();
    fs::write(dir.path().join("src/g.rs"), SRC).unwrap();
    let mut db = Db::open(&dir.path().join(".codemap/index.db")).unwrap();
    index::index_full(&mut db, dir.path()).unwrap();
    index::resolve_calls(&mut db, dir.path()).unwrap();
    (dir, db)
}

#[test]
fn search_finds_symbol() {
    let (_d, db) = setup();
    let hits = query::search(&db, "leaf", "symbol", 10).unwrap();
    assert!(hits.iter().any(|h| h.name_path == "leaf"));
    // prefix + case-insensitive
    assert!(query::search(&db, "ROOT", "symbol", 10)
        .unwrap()
        .iter()
        .any(|h| h.name_path == "root_fn"));
    assert!(query::search(&db, "x", "semantic", 10).is_err());
}

#[test]
fn impact_is_transitive_callers() {
    let (_d, db) = setup();
    let leaf = query::resolve(&db, "leaf", 5).unwrap()[0].id;
    let imp = query::impact(&db, leaf, 4, 80).unwrap();
    assert!(imp.iter().any(|h| h.name_path == "mid"));
    assert!(
        imp.iter().any(|h| h.name_path == "root_fn"),
        "transitive caller"
    );
}

#[test]
fn trace_reaches_root_entrypoint() {
    let (_d, db) = setup();
    let leaf = query::resolve(&db, "leaf", 5).unwrap()[0].id;
    let roots = query::trace_to_roots(&db, leaf, 6, 40).unwrap();
    assert!(
        roots.iter().any(|h| h.name_path == "root_fn"),
        "root has no callers"
    );
    assert!(
        !roots.iter().any(|h| h.name_path == "mid"),
        "mid has a caller, not a root"
    );
}

#[test]
fn references_resolve_to_enclosing() {
    let (_d, db) = setup();
    let leaf = query::resolve(&db, "leaf", 5).unwrap()[0].id;
    let refs = query::references(&db, leaf, 100).unwrap();
    assert!(refs.iter().any(|r| r.enclosing.as_deref() == Some("mid")));
}

#[test]
fn variables_lists_scope_fields() {
    let (_d, db) = setup();
    let vars = query::variables(&db, "Cfg", 100).unwrap();
    assert!(vars.iter().any(|h| h.name_path == "Cfg/max"));
}
