//! `codemap grep`: regex content search bridged to the symbol graph. Finds values inside
//! strings/consts (mapped to their enclosing symbol) AND in non-indexed config files.

use codemap::db::Db;
use codemap::{index, query};
use std::fs;

#[test]
fn grep_finds_values_and_maps_to_enclosing_symbol() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::create_dir_all(root.join("src")).unwrap();
    fs::create_dir_all(root.join(".codemap")).unwrap();
    // A value inside a const (indexed source).
    fs::write(
        root.join("src/lib.rs"),
        "pub const SWAP_URL: &str = \"https://api.1inch.dev/swap\";\nfn helper() {}\n",
    )
    .unwrap();
    // A value in a non-indexed, hidden config file.
    fs::write(
        root.join(".env.example"),
        "ONEINCH_API_KEY=\"your-1inch-key\"\n",
    )
    .unwrap();

    let mut db = Db::open(&root.join(".codemap/index.db")).unwrap();
    index::index_full(&mut db, root).unwrap();

    let hits = query::grep(&db, root, "1inch", false, 50).unwrap();

    // The string value inside the const maps to its enclosing symbol.
    assert!(
        hits.iter().any(|h| h.file == "src/lib.rs"
            && h.enclosing
                .as_ref()
                .map(|(_, np)| np == "SWAP_URL")
                .unwrap_or(false)),
        "value inside a const should map to SWAP_URL, got {hits:?}"
    );
    // The value in a hidden, non-indexed config file is still found (no enclosing symbol).
    assert!(
        hits.iter()
            .any(|h| h.file == ".env.example" && h.enclosing.is_none()),
        "value in a non-indexed dotfile should be found with no enclosing symbol, got {hits:?}"
    );

    // Regex works (not just literal).
    let re_hits = query::grep(&db, root, r"api\.1inch\.\w+", false, 50).unwrap();
    assert!(re_hits.iter().any(|h| h.file == "src/lib.rs"));
}
