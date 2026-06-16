//! Subgraph export to DOT/Mermaid.

use codemap::db::Db;
use codemap::{export, index, query};
use std::fs;

#[test]
fn export_dot_and_mermaid_contain_nodes_and_edges() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::create_dir_all(root.join("src")).unwrap();
    fs::create_dir_all(root.join(".codemap")).unwrap();
    fs::write(root.join("src/g.rs"), "fn callee() {}\nfn caller() { callee(); }\n").unwrap();

    let mut db = Db::open(&root.join(".codemap/index.db")).unwrap();
    index::index_full(&mut db, root).unwrap();
    index::resolve_calls(&mut db, root).unwrap();

    let caller = query::resolve(&db, "caller", 5).unwrap()[0].id;
    let g = query::subgraph(&db, caller, 2, true).unwrap();
    assert!(g.nodes.iter().any(|(_, n)| n == "caller"));
    assert!(g.nodes.iter().any(|(_, n)| n == "callee"));
    assert!(!g.edges.is_empty());

    let dot = export::to_dot(&g);
    assert!(dot.starts_with("digraph codemap"));
    assert!(dot.contains("-> "));
    let mer = export::to_mermaid(&g);
    assert!(mer.starts_with("flowchart LR"));
    assert!(mer.contains("-->"));
}
