//! Schema-level tests proving the adversarial-review fixes. If any fails, the schema
//! foundation is broken.

use codemap::db::Db;
use codemap::types::EdgeKind;
use rusqlite::params;

fn intern(db: &Db, s: &str) -> i64 {
    db.conn
        .execute("INSERT OR IGNORE INTO string_pool(text) VALUES (?1)", [s])
        .unwrap();
    db.conn
        .query_row("SELECT id FROM string_pool WHERE text=?1", [s], |r| r.get(0))
        .unwrap()
}

fn add_file(db: &Db, id: i64, path: &str) {
    let p = intern(db, path);
    db.conn
        .execute(
            "INSERT INTO file(id,path_sid,lang,size,mtime_ns,content_hash,line_count,indexed_at,tier)
             VALUES (?1,?2,0,0,0,X'00',0,0,0)",
            params![id, p],
        )
        .unwrap();
}

fn add_symbol(db: &Db, id: i64, file_id: i64, name: &str, name_path: &str, line: u32, col: u32) {
    let n = intern(db, name);
    let np = intern(db, name_path);
    let key = blake3::hash(format!("{file_id}:{name_path}:{id}").as_bytes());
    db.conn
        .execute(
            "INSERT INTO symbol(id,symbol_key,file_id,name_sid,name_path_sid,kind,
                                start_line,start_col,end_line,end_col,sel_line,sel_col)
             VALUES (?1,?2,?3,?4,?5,0,?6,?7,?6,?7,?6,?7)",
            params![id, key.as_bytes().to_vec(), file_id, n, np, line, col],
        )
        .unwrap();
}

fn add_edge(db: &Db, src: i64, tgt: i64, kind: EdgeKind) {
    db.conn
        .execute(
            "INSERT INTO edge(source_symbol_id,target_symbol_id,kind,provenance,resolution)
             VALUES (?1,?2,?3,0,1)",
            params![src, tgt, kind.as_i64()],
        )
        .unwrap();
}

// Fix #2: a Tier0 edge with no call-site must insert.
#[test]
fn edge_without_callsite_inserts() {
    let db = Db::open_in_memory().unwrap();
    add_file(&db, 1, "src/a.rs");
    add_symbol(&db, 10, 1, "foo", "foo", 1, 0);
    add_symbol(&db, 11, 1, "bar", "bar", 5, 0);
    add_edge(&db, 10, 11, EdgeKind::Calls);

    let n: i64 = db
        .conn
        .query_row("SELECT count(*) FROM edge", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 1);
}

// Fix #1: a range with col > 4096 must not corrupt (bit-packing would).
#[test]
fn range_col_over_4096_roundtrips_exactly() {
    let db = Db::open_in_memory().unwrap();
    add_file(&db, 1, "dist/bundle.min.js");
    add_symbol(&db, 10, 1, "m", "m", 100, 5000);

    let (line, col): (u32, u32) = db
        .conn
        .query_row("SELECT start_line, start_col FROM symbol WHERE id=10", [], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .unwrap();
    assert_eq!((line, col), (100, 5000));
}

// Fix #4: contentless FTS5 has no orphan after a correct prune.
#[test]
fn fts5_no_orphan_when_pruned_correctly() {
    let db = Db::open_in_memory().unwrap();
    add_file(&db, 1, "src/pay.rs");
    add_symbol(&db, 10, 1, "charge", "PaymentService/charge", 12, 4);
    db.conn
        .execute(
            "INSERT INTO symbol_fts(rowid,name,name_path) VALUES (10,'charge','PaymentService/charge')",
            [],
        )
        .unwrap();

    let before: i64 = db
        .conn
        .query_row("SELECT count(*) FROM symbol_fts WHERE symbol_fts MATCH 'charge'", [], |r| r.get(0))
        .unwrap();
    assert_eq!(before, 1);

    // Correct prune: 'delete' with OLD values, same transaction, before deleting the symbol.
    let tx = db.conn.unchecked_transaction().unwrap();
    tx.execute(
        "INSERT INTO symbol_fts(symbol_fts,rowid,name,name_path) VALUES('delete',10,'charge','PaymentService/charge')",
        [],
    )
    .unwrap();
    tx.execute("DELETE FROM symbol WHERE id=10", []).unwrap();
    tx.commit().unwrap();

    let after: i64 = db
        .conn
        .query_row("SELECT count(*) FROM symbol_fts WHERE symbol_fts MATCH 'charge'", [], |r| r.get(0))
        .unwrap();
    assert_eq!(after, 0);
}

// Recursive CTE with a cycle terminates and yields correct MIN(depth).
#[test]
fn cte_with_cycle_terminates() {
    let db = Db::open_in_memory().unwrap();
    add_file(&db, 1, "src/g.rs");
    for id in 1..=4 {
        add_symbol(&db, id, 1, &format!("s{id}"), &format!("s{id}"), id as u32, 0);
    }
    add_edge(&db, 1, 2, EdgeKind::Calls);
    add_edge(&db, 2, 3, EdgeKind::Calls);
    add_edge(&db, 3, 1, EdgeKind::Calls); // cycle back to 1
    add_edge(&db, 1, 4, EdgeKind::Calls);

    let mut got = db.callees(1, EdgeKind::Calls.as_i64(), 10, 100).unwrap();
    got.sort();
    assert_eq!(got, vec![(2, 1), (3, 2), (4, 1)]);
}

#[test]
fn callers_reverse_direction() {
    let db = Db::open_in_memory().unwrap();
    add_file(&db, 1, "src/g.rs");
    for id in 1..=3 {
        add_symbol(&db, id, 1, &format!("s{id}"), &format!("s{id}"), id as u32, 0);
    }
    add_edge(&db, 2, 1, EdgeKind::Calls);
    add_edge(&db, 3, 2, EdgeKind::Calls);

    let mut got = db.callers(1, EdgeKind::Calls.as_i64(), 10, 100).unwrap();
    got.sort();
    assert_eq!(got, vec![(2, 1), (3, 2)]);
}

#[test]
fn reopen_keeps_schema_version() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("index.db");
    {
        let _db = Db::open(&path).unwrap();
    }
    let db = Db::open(&path).unwrap();
    let v: i64 = db
        .conn
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(v, codemap::db::SCHEMA_VERSION);
}
