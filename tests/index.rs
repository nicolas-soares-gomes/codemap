//! Indexer integration: index a real file, then resolve, read its code, and outline.

use codemap::db::Db;
use codemap::{index, query};
use std::fs;

const SRC: &str = "pub struct PaymentService;\n\
impl PaymentService {\n\
    pub fn charge(&self, amount: u64) -> u64 {\n\
        amount\n\
    }\n\
}\n";

fn setup() -> (tempfile::TempDir, Db) {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join("src")).unwrap();
    fs::write(dir.path().join("src/pay.rs"), SRC).unwrap();
    fs::create_dir_all(dir.path().join(".codemap")).unwrap();
    let db = Db::open(&dir.path().join(".codemap/index.db")).unwrap();
    (dir, db)
}

#[test]
fn index_then_resolve_and_read() {
    let (dir, mut db) = setup();
    let stats = index::index_full(&mut db, dir.path()).unwrap();
    assert_eq!(stats.files, 1);
    assert!(stats.symbols >= 2);

    let hits = query::resolve(&db, "charge", 25).unwrap();
    let charge = hits
        .iter()
        .find(|h| h.name_path == "PaymentService/charge")
        .expect("charge resolved");

    let code = query::read_symbol(&mut db, dir.path(), charge.id).unwrap();
    assert!(code.code.contains("pub fn charge"));
    assert!(code.code.contains("amount"));
    assert!(!code.code.contains("struct PaymentService"), "must be only the method range");
}

#[test]
fn outline_lists_file_symbols() {
    let (dir, mut db) = setup();
    index::index_full(&mut db, dir.path()).unwrap();
    let hits = query::outline(&db, "src/pay.rs").unwrap();
    assert!(hits.iter().any(|h| h.name_path == "PaymentService"));
    assert!(hits.iter().any(|h| h.name_path == "PaymentService/charge"));
}

#[test]
fn symbol_id_is_stable_across_reindex() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join("src")).unwrap();
    fs::create_dir_all(dir.path().join(".codemap")).unwrap();
    let mut db = Db::open(&dir.path().join(".codemap/index.db")).unwrap();

    fs::write(dir.path().join("src/pay.rs"), SRC).unwrap();
    index::index_full(&mut db, dir.path()).unwrap();
    let id_before = query::resolve(&db, "charge", 25).unwrap()[0].id;
    let line_before = query::resolve(&db, "charge", 25).unwrap()[0].line;

    // Prepend a blank line: shifts every range down by 1, but identity is unchanged.
    fs::write(dir.path().join("src/pay.rs"), format!("\n{SRC}")).unwrap();
    index::index_full(&mut db, dir.path()).unwrap();
    let hit_after = query::resolve(&db, "charge", 25).unwrap();
    assert_eq!(hit_after.len(), 1);
    assert_eq!(hit_after[0].id, id_before, "id must survive reindex");
    assert_eq!(hit_after[0].line, line_before + 1, "range must be refreshed");
}

#[test]
fn read_symbol_staleness_guard_reindexes_inline() {
    let (dir, mut db) = setup();
    index::index_full(&mut db, dir.path()).unwrap();
    let id = query::resolve(&db, "charge", 25).unwrap()[0].id;

    // Edit the file on disk WITHOUT reindexing: prepend two lines (shifts ranges).
    let shifted = format!("// a\n// b\n{SRC}");
    std::fs::write(dir.path().join("src/pay.rs"), &shifted).unwrap();

    let c = query::read_symbol(&mut db, dir.path(), id).unwrap();
    assert!(c.reindexed, "should reindex inline on hash change");
    assert!(c.code.contains("pub fn charge"), "must serve the correct (shifted) range");
    assert!(!c.code.contains("struct PaymentService"));
}

#[test]
fn call_edges_link_caller_and_callee() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join("src")).unwrap();
    fs::create_dir_all(dir.path().join(".codemap")).unwrap();
    fs::write(
        dir.path().join("src/g.rs"),
        "fn callee() {}\nfn caller() {\n    callee();\n}\n",
    )
    .unwrap();
    let mut db = Db::open(&dir.path().join(".codemap/index.db")).unwrap();
    index::index_full(&mut db, dir.path()).unwrap();
    let edges = index::resolve_calls(&mut db, dir.path()).unwrap();
    assert!(edges >= 1, "expected a call edge");

    let callee_id = query::resolve(&db, "callee", 5).unwrap()[0].id;
    let caller_id = query::resolve(&db, "caller", 5).unwrap()[0].id;

    let callers = query::callers(&db, callee_id, 1, 50).unwrap();
    assert!(callers.iter().any(|h| h.name_path == "caller"), "caller should call callee");

    let callees = query::callees(&db, caller_id, 1, 50).unwrap();
    assert!(callees.iter().any(|h| h.name_path == "callee"), "caller should reach callee");
}

#[test]
fn reindex_is_idempotent_and_prunes_fts() {
    let (dir, mut db) = setup();
    index::index_full(&mut db, dir.path()).unwrap();
    index::index_full(&mut db, dir.path()).unwrap();

    let n: i64 = db
        .conn
        .query_row(
            "SELECT count(*) FROM symbol s JOIN string_pool np ON np.id=s.name_path_sid
             WHERE np.text='PaymentService/charge'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(n, 1);

    let fts_hits: i64 = db
        .conn
        .query_row("SELECT count(*) FROM symbol_fts WHERE symbol_fts MATCH 'charge'", [], |r| r.get(0))
        .unwrap();
    assert_eq!(fts_hits, 1);
}
