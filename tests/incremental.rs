//! Incremental reconcile: only changed files are reprocessed; deletions are pruned.

use codemap::db::Db;
use codemap::{index, query};
use std::fs;

#[test]
fn reconcile_handles_change_add_delete() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::create_dir_all(root.join("src")).unwrap();
    fs::create_dir_all(root.join(".codemap")).unwrap();
    fs::write(root.join("src/a.rs"), "pub fn a() {}\n").unwrap();
    fs::write(root.join("src/b.rs"), "pub fn b() {}\n").unwrap();
    fs::write(root.join("src/keep.rs"), "pub fn keep() {}\n").unwrap();

    let mut db = Db::open(&root.join(".codemap/index.db")).unwrap();
    index::index_full(&mut db, root).unwrap();

    // change a, add c, delete b, leave keep untouched.
    fs::write(root.join("src/a.rs"), "pub fn a() {}\npub fn a2() {}\n").unwrap();
    fs::write(root.join("src/c.rs"), "pub fn c() {}\n").unwrap();
    fs::remove_file(root.join("src/b.rs")).unwrap();

    let r = index::reconcile(&mut db, root).unwrap();
    assert_eq!(r.added, 1, "c.rs added");
    assert_eq!(r.deleted, 1, "b.rs deleted");
    assert!(r.changed >= 1, "a.rs changed");
    assert!(r.unchanged >= 1, "keep.rs unchanged");

    assert_eq!(query::resolve(&db, "a2", 5).unwrap().len(), 1, "new symbol indexed");
    assert_eq!(query::resolve(&db, "c", 5).unwrap().len(), 1, "added file indexed");
    assert!(query::resolve(&db, "b", 5).unwrap().is_empty(), "deleted file pruned");
    assert_eq!(query::resolve(&db, "keep", 5).unwrap().len(), 1, "kept symbol intact");
}
