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

    let code = query::read_symbol(&db, dir.path(), charge.id).unwrap();
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
