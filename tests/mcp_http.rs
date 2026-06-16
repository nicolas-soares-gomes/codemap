//! MCP-over-HTTP handshake (feature `mcp-http`): the streamable HTTP transport answers an
//! `initialize` request with the server info.
#![cfg(feature = "mcp-http")]

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn http_initialize_handshake() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(dir.path().join("src/lib.rs"), "fn f() {}\n").unwrap();
    std::fs::create_dir_all(dir.path().join(".codemap")).unwrap();
    {
        let mut db = codemap::db::Db::open(&dir.path().join(".codemap/index.db")).unwrap();
        codemap::index::index_full(&mut db, dir.path()).unwrap();
    }

    let addr: SocketAddr = format!("127.0.0.1:{}", free_port()).parse().unwrap();
    let root = dir.path().to_path_buf();
    tokio::spawn(async move {
        let _ = codemap::mcp::serve_http(root, addr).await;
    });

    let body = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"t","version":"0"}}}"#;
    let req = format!(
        "POST /mcp HTTP/1.1\r\nHost: {addr}\r\nContent-Type: application/json\r\n\
         Accept: application/json, text/event-stream\r\nContent-Length: {}\r\n\
         Connection: close\r\n\r\n{body}",
        body.len()
    );

    // Retry until the server is bound and accepting.
    let mut resp = String::new();
    for _ in 0..40 {
        if let Ok(mut s) = TcpStream::connect(addr) {
            s.write_all(req.as_bytes()).unwrap();
            resp.clear();
            s.read_to_string(&mut resp).unwrap();
            if !resp.is_empty() {
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(50));
    }

    assert!(resp.contains("200"), "expected HTTP 200, got: {resp}");
    assert!(
        resp.contains("serverInfo") && resp.contains("protocolVersion"),
        "expected initialize result, got: {resp}"
    );
}
