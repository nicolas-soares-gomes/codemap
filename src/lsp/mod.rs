//! Tier 2: on-demand precise call edges from a user-installed language server (LSP over stdio).
//! codemap never installs a server; if none is on PATH it returns a tip. Edges confirmed by the
//! server are provenance=lsp, resolution=resolved and upgrade the syntactic (ambiguous) edges
//! in place. Useful for languages without a good SCIP indexer (Kotlin, Clojure, Swift, PHP).

use crate::db::Db;
use crate::types::{EdgeKind, Language, Provenance, Resolution};
use anyhow::{bail, Context, Result};
use rusqlite::{params, OptionalExtension};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);
const READY_TIMEOUT: Duration = Duration::from_secs(45);

/// Enrich one symbol's call edges using its language server. Returns how many edges were
/// confirmed (added or upgraded to lsp/resolved). Never installs the server.
pub fn enrich(db: &mut Db, root: &Path, symbol_id: i64) -> Result<usize> {
    let (rel, sel_line, sel_col, lang_i): (String, u32, u32, i64) = db
        .conn
        .query_row(
            "SELECT fp.text, s.sel_line, s.sel_col, f.lang
             FROM symbol s JOIN file f ON f.id=s.file_id
             JOIN string_pool fp ON fp.id=f.path_sid WHERE s.id=?1",
            [symbol_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .optional()?
        .with_context(|| format!("symbol sym:{symbol_id} not found"))?;

    let lang = Language::from_i64(lang_i).context("unknown language")?;
    let argv = crate::doctor::lsp_invocation(lang);
    let bin = &argv[0];
    if !crate::doctor::binary_present(bin) {
        bail!(
            "language server `{bin}` not found on PATH — install it yourself, then retry. \
             codemap never installs it. See `codemap doctor`."
        );
    }

    let abs = root.join(&rel);
    let text = std::fs::read_to_string(&abs).with_context(|| format!("read {}", abs.display()))?;
    let root_uri = path_uri(&root.canonicalize().unwrap_or_else(|_| root.to_path_buf()));
    let file_uri = path_uri(&abs.canonicalize().unwrap_or(abs.clone()));

    let mut lsp = Lsp::start(&argv, &root_uri)?;
    lsp.notify(
        "textDocument/didOpen",
        json!({"textDocument": {
            "uri": file_uri, "languageId": language_id(lang), "version": 1, "text": text
        }}),
    )?;

    // The server may need to index before call hierarchy is available; poll until ready.
    let pos = json!({"line": sel_line.saturating_sub(1), "character": sel_col});
    let item = lsp.prepare_call_hierarchy(&file_uri, &pos)?;
    let Some(item) = item else {
        lsp.shutdown();
        return Ok(0);
    };

    let mut count = 0usize;
    // Outgoing: this symbol -> callee.
    for call in lsp.calls("callHierarchy/outgoingCalls", &item, "to")? {
        if let Some(target) = self::resolve_item(db, root, &call) {
            count += upsert_edge(db, symbol_id, target)?;
        }
    }
    // Incoming: caller -> this symbol.
    for call in lsp.calls("callHierarchy/incomingCalls", &item, "from")? {
        if let Some(source) = self::resolve_item(db, root, &call) {
            count += upsert_edge(db, source, symbol_id)?;
        }
    }

    lsp.shutdown();
    Ok(count)
}

/// Map an LSP call-hierarchy item (uri + selectionRange) back to a codemap symbol id.
fn resolve_item(db: &Db, root: &Path, item: &Value) -> Option<i64> {
    let uri = item.get("uri")?.as_str()?;
    let rel = uri_to_rel(root, uri)?;
    let file_id: i64 = db
        .conn
        .query_row(
            "SELECT f.id FROM file f JOIN string_pool s ON s.id=f.path_sid WHERE s.text=?1",
            [rel],
            |r| r.get(0),
        )
        .optional()
        .ok()??;
    let line0 = item
        .get("selectionRange")
        .or_else(|| item.get("range"))?
        .get("start")?
        .get("line")?
        .as_u64()? as u32;
    symbol_at(db, file_id, line0 + 1)
}

/// The symbol defined at `line1` (exact selection line), else the innermost callable covering it.
fn symbol_at(db: &Db, file_id: i64, line1: u32) -> Option<i64> {
    if let Ok(Some(id)) = db
        .conn
        .query_row(
            "SELECT id FROM symbol WHERE file_id=?1 AND sel_line=?2 LIMIT 1",
            params![file_id, line1],
            |r| r.get::<_, i64>(0),
        )
        .optional()
    {
        return Some(id);
    }
    db.conn
        .query_row(
            "SELECT id FROM symbol WHERE file_id=?1 AND start_line<=?2 AND end_line>=?2
               AND kind IN (?3,?4) ORDER BY (end_line-start_line) ASC LIMIT 1",
            params![
                file_id,
                line1,
                crate::types::SymbolKind::Function.as_i64(),
                crate::types::SymbolKind::Method.as_i64()
            ],
            |r| r.get::<_, i64>(0),
        )
        .optional()
        .ok()
        .flatten()
}

fn upsert_edge(db: &Db, source: i64, target: i64) -> Result<usize> {
    if source == target {
        return Ok(0);
    }
    let n = db.conn.execute(
        "INSERT INTO edge(source_symbol_id,target_symbol_id,kind,provenance,resolution)
         VALUES (?1,?2,?3,?4,?5)
         ON CONFLICT(source_symbol_id,target_symbol_id,kind)
         DO UPDATE SET provenance=excluded.provenance, resolution=excluded.resolution",
        params![
            source,
            target,
            EdgeKind::Calls.as_i64(),
            Provenance::Lsp.as_i64(),
            Resolution::Resolved.as_i64()
        ],
    )?;
    Ok(n)
}

// ---- minimal synchronous LSP client ----------------------------------------

struct Lsp {
    child: Child,
    stdin: ChildStdin,
    rx: Receiver<Value>,
    next_id: i64,
}

impl Lsp {
    fn start(argv: &[String], root_uri: &str) -> Result<Self> {
        let mut child = Command::new(&argv[0])
            .args(&argv[1..])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .with_context(|| format!("spawn language server `{}`", argv[0]))?;
        let stdin = child.stdin.take().context("server stdin")?;
        let stdout = child.stdout.take().context("server stdout")?;
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || reader_loop(stdout, tx));

        let mut lsp = Lsp {
            child,
            stdin,
            rx,
            next_id: 0,
        };
        lsp.request(
            "initialize",
            json!({
                "processId": std::process::id(),
                "rootUri": root_uri,
                "capabilities": {"textDocument": {"callHierarchy": {"dynamicRegistration": false}}}
            }),
            REQUEST_TIMEOUT,
        )?;
        lsp.notify("initialized", json!({}))?;
        Ok(lsp)
    }

    fn write_msg(&mut self, v: &Value) -> Result<()> {
        let body = serde_json::to_vec(v)?;
        write!(self.stdin, "Content-Length: {}\r\n\r\n", body.len())?;
        self.stdin.write_all(&body)?;
        self.stdin.flush()?;
        Ok(())
    }

    fn notify(&mut self, method: &str, params: Value) -> Result<()> {
        self.write_msg(&json!({"jsonrpc": "2.0", "method": method, "params": params}))
    }

    fn request(&mut self, method: &str, params: Value, timeout: Duration) -> Result<Value> {
        let resp = self.request_raw(method, params, timeout)?;
        if let Some(err) = resp.get("error") {
            bail!("LSP `{method}` error: {err}");
        }
        Ok(resp.get("result").cloned().unwrap_or(Value::Null))
    }

    /// Like `request`, but returns the full response object so callers can inspect a JSON-RPC
    /// error (e.g. `-32801 ContentModified`, sent while the server is still indexing).
    fn request_raw(&mut self, method: &str, params: Value, timeout: Duration) -> Result<Value> {
        self.next_id += 1;
        let id = self.next_id;
        self.write_msg(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}))?;
        let deadline = Instant::now() + timeout;
        loop {
            let remaining = deadline
                .checked_duration_since(Instant::now())
                .context("LSP request timed out")?;
            let msg = self
                .rx
                .recv_timeout(remaining)
                .map_err(|_| anyhow::anyhow!("LSP `{method}` timed out"))?;
            // A response to our request.
            if msg.get("id").and_then(Value::as_i64) == Some(id) && msg.get("method").is_none() {
                return Ok(msg);
            }
            // A server->client request: reply with null so the server doesn't stall.
            if let (Some(rid), true) = (msg.get("id").cloned(), msg.get("method").is_some()) {
                self.write_msg(&json!({"jsonrpc": "2.0", "id": rid, "result": Value::Null}))?;
            }
            // Notifications are ignored.
        }
    }

    /// Poll `prepareCallHierarchy` until the server returns an item or readiness times out.
    /// `-32801 ContentModified` means the server is still indexing — retry until ready.
    fn prepare_call_hierarchy(&mut self, uri: &str, pos: &Value) -> Result<Option<Value>> {
        let params = json!({"textDocument": {"uri": uri}, "position": pos});
        let deadline = Instant::now() + READY_TIMEOUT;
        loop {
            let resp = self.request_raw(
                "textDocument/prepareCallHierarchy",
                params.clone(),
                REQUEST_TIMEOUT,
            )?;
            if resp.get("error").is_none() {
                if let Some(item) = resp
                    .get("result")
                    .and_then(|r| r.as_array()?.first().cloned())
                {
                    return Ok(Some(item));
                }
            }
            if Instant::now() >= deadline {
                return Ok(None);
            }
            std::thread::sleep(Duration::from_millis(400));
        }
    }

    /// Run an incoming/outgoing call-hierarchy request and return the items under `field`,
    /// retrying while the server reports it is still indexing.
    fn calls(&mut self, method: &str, item: &Value, field: &str) -> Result<Vec<Value>> {
        let deadline = Instant::now() + READY_TIMEOUT;
        loop {
            let resp = self.request_raw(method, json!({"item": item}), REQUEST_TIMEOUT)?;
            if resp.get("error").is_none() {
                let res = resp.get("result").cloned().unwrap_or(Value::Null);
                return Ok(res
                    .as_array()
                    .map(|a| a.iter().filter_map(|c| c.get(field).cloned()).collect())
                    .unwrap_or_default());
            }
            if Instant::now() >= deadline {
                return Ok(Vec::new());
            }
            std::thread::sleep(Duration::from_millis(400));
        }
    }

    fn shutdown(mut self) {
        let _ = self.request("shutdown", Value::Null, Duration::from_secs(3));
        let _ = self.notify("exit", Value::Null);
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn reader_loop(stdout: std::process::ChildStdout, tx: mpsc::Sender<Value>) {
    let mut reader = BufReader::new(stdout);
    loop {
        let mut content_length = 0usize;
        loop {
            let mut line = String::new();
            match reader.read_line(&mut line) {
                Ok(0) | Err(_) => return, // EOF or broken pipe
                Ok(_) => {}
            }
            let t = line.trim_end();
            if t.is_empty() {
                break;
            }
            if let Some(n) = t.strip_prefix("Content-Length:") {
                content_length = n.trim().parse().unwrap_or(0);
            }
        }
        if content_length == 0 {
            continue;
        }
        let mut buf = vec![0u8; content_length];
        if reader.read_exact(&mut buf).is_err() {
            return;
        }
        if let Ok(v) = serde_json::from_slice::<Value>(&buf) {
            if tx.send(v).is_err() {
                return;
            }
        }
    }
}

// ---- uri / path helpers ----------------------------------------------------

fn path_uri(p: &Path) -> String {
    format!("file://{}", p.to_string_lossy())
}

/// A `file://` URI made relative to `root` ("/"-normalized), or None if outside the repo.
fn uri_to_rel(root: &Path, uri: &str) -> Option<String> {
    let path = uri.strip_prefix("file://").unwrap_or(uri);
    let path = Path::new(path);
    let canon_root = root.canonicalize().ok()?;
    let rel = path
        .canonicalize()
        .ok()?
        .strip_prefix(&canon_root)
        .ok()?
        .to_string_lossy()
        .replace('\\', "/");
    (!rel.is_empty()).then_some(rel)
}

fn language_id(lang: Language) -> &'static str {
    use Language::*;
    match lang {
        Rust => "rust",
        Go => "go",
        C => "c",
        Cpp => "cpp",
        TypeScript => "typescript",
        JavaScript => "javascript",
        Python => "python",
        Java => "java",
        CSharp => "csharp",
        Php => "php",
        Swift => "swift",
        Kotlin => "kotlin",
        Clojure => "clojure",
    }
}
