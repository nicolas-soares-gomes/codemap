//! Incremental reconcile via the git fast-path (diff indexed_commit..HEAD).

use codemap::db::Db;
use codemap::{index, query};
use std::fs;
use std::path::Path;
use std::process::Command;

fn git(dir: &Path, args: &[&str]) {
    let ok = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args([
            "-c",
            "user.email=t@e",
            "-c",
            "user.name=t",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .status()
        .unwrap()
        .success();
    assert!(ok, "git {args:?} failed");
}

#[test]
fn git_fast_path_picks_up_new_commit() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::create_dir_all(root.join("src")).unwrap();
    fs::create_dir_all(root.join(".codemap")).unwrap();
    fs::write(root.join(".gitignore"), "/.codemap/\n").unwrap();
    fs::write(root.join("src/a.rs"), "fn a() {}\n").unwrap();
    git(root, &["init", "-q"]);
    git(root, &["add", "-A"]);
    git(root, &["commit", "-q", "-m", "init"]);

    let mut db = Db::open(&root.join(".codemap/index.db")).unwrap();
    index::index_full(&mut db, root).unwrap();
    // First reconcile takes the fs path and seeds indexed_commit.
    index::reconcile(&mut db, root).unwrap();
    assert!(db.get_meta("indexed_commit").unwrap().is_some());

    // A new commit adds b.rs.
    fs::write(root.join("src/b.rs"), "fn b() {}\n").unwrap();
    git(root, &["add", "-A"]);
    git(root, &["commit", "-q", "-m", "add b"]);

    let stats = index::reconcile(&mut db, root).unwrap();
    assert_eq!(
        db.get_meta("scanner_mode").unwrap().as_deref(),
        Some("git"),
        "second reconcile must use the git fast-path"
    );
    assert!(stats.changed >= 1, "b.rs detected as changed");
    assert_eq!(
        query::resolve(&db, "b", 5).unwrap().len(),
        1,
        "new file indexed via git path"
    );
}
