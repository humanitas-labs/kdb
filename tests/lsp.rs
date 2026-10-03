//! Drives `kdb2 lsp` over stdio JSON-RPC. Cases ported from the v1 suite
//! (`projects/kdb/tests/lsp.rs`), minus hover, symbols, and formatting.
//!
//! Tests that need link resolution are `#[ignore = "needs graph"]` until the
//! graph agent lands `src/graph/`; run them with `cargo test --test lsp -- --ignored`.

mod common;

use std::fs;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use common::TestWorkspace;
use serde_json::{Value, json};
use tower_lsp::lsp_types::Url;

const WAIT: Duration = Duration::from_secs(5);

struct Fixture {
    ws: TestWorkspace,
    a: Url,
    b: Url,
    scratch: Url,
}

impl Fixture {
    fn new() -> Self {
        let ws = TestWorkspace::new();
        ws.write("a.md", "# A\n\n## Details\n\nSee [B](b.md#target)\nSee [[b#target]]\n");
        ws.write("b.md", "# B\n\n## Target\nHello from target section.\n");
        let uri = |rel: &str| Url::from_file_path(ws.path(rel)).unwrap();
        let (a, b, scratch) = (uri("a.md"), uri("b.md"), uri("scratch.md"));
        Self { ws, a, b, scratch }
    }

    fn root(&self) -> &Path {
        &self.ws.root
    }
}

struct Session {
    child: Child,
    stdin: ChildStdin,
    rx: mpsc::Receiver<Value>,
    buffered: Vec<Value>,
}

impl Session {
    fn start(root: &Path) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_kdb2"))
            .args(["lsp", &root.to_string_lossy()])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("spawn kdb2 lsp");
        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            while let Ok(Some(msg)) = read_message(&mut reader) {
                if tx.send(msg).is_err() {
                    break;
                }
            }
        });
        Self { child, stdin, rx, buffered: Vec::new() }
    }

    fn initialize_with(&mut self, root: &Path, capabilities: Value) -> Value {
        let root_uri = Url::from_file_path(root).unwrap();
        self.send(json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
            "processId": Value::Null, "rootUri": root_uri, "capabilities": capabilities }}));
        let response = self.wait_for_id(1);
        self.send(json!({"jsonrpc": "2.0", "method": "initialized", "params": {}}));
        response
    }

    fn initialize(&mut self, root: &Path) -> Value {
        self.initialize_with(root, json!({}))
    }

    fn send(&mut self, message: Value) {
        let body = message.to_string();
        write!(self.stdin, "Content-Length: {}\r\n\r\n{}", body.len(), body).unwrap();
        self.stdin.flush().unwrap();
    }

    fn request(&mut self, id: i64, method: &str, params: Value) -> Value {
        self.send(json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
        self.wait_for_id(id)
    }

    fn notify(&mut self, method: &str, params: Value) {
        self.send(json!({"jsonrpc": "2.0", "method": method, "params": params}));
    }

    fn open(&mut self, uri: &Url, text: &str) {
        self.notify("textDocument/didOpen", json!({"textDocument": {
            "uri": uri, "languageId": "markdown", "version": 1, "text": text }}));
    }

    fn change(&mut self, uri: &Url, text: &str) {
        self.notify("textDocument/didChange", json!({
            "textDocument": {"uri": uri, "version": 2}, "contentChanges": [{"text": text}]}));
    }

    fn at(uri: &Url, line: u32, character: u32) -> Value {
        json!({"textDocument": {"uri": uri}, "position": {"line": line, "character": character}})
    }

    fn wait_for_id(&mut self, id: i64) -> Value {
        self.wait_for(|m| m.get("id").and_then(Value::as_i64) == Some(id))
    }

    /// The next `publishDiagnostics` for `uri`; returns the diagnostics array.
    fn diagnostics(&mut self, uri: &Url) -> Vec<Value> {
        let msg = self.wait_for(|m| {
            m["method"] == "textDocument/publishDiagnostics" && m["params"]["uri"] == uri.as_str()
        });
        msg["params"]["diagnostics"].as_array().cloned().expect("diagnostics array")
    }

    fn wait_for(&mut self, mut pred: impl FnMut(&Value) -> bool) -> Value {
        if let Some(i) = self.buffered.iter().position(&mut pred) {
            return self.buffered.remove(i);
        }
        let deadline = Instant::now() + WAIT;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            assert!(!remaining.is_zero(), "timed out waiting for LSP message");
            match self.rx.recv_timeout(remaining.min(Duration::from_millis(200))) {
                Ok(msg) if pred(&msg) => return msg,
                Ok(msg) => self.buffered.push(msg),
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(mpsc::RecvTimeoutError::Disconnected) => panic!("LSP stdout closed"),
            }
        }
    }

    fn shutdown(&mut self) {
        let _ = self.request(999, "shutdown", Value::Null);
        self.notify("exit", json!({}));
        let deadline = Instant::now() + Duration::from_secs(3);
        while Instant::now() < deadline {
            if let Ok(Some(_)) = self.child.try_wait() {
                return;
            }
            thread::sleep(Duration::from_millis(50));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        if let Ok(None) = self.child.try_wait() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

fn read_message(reader: &mut BufReader<ChildStdout>) -> io::Result<Option<Value>> {
    let mut length = None;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 {
            return Ok(None);
        }
        let header = line.trim_end();
        if header.is_empty() {
            break;
        }
        if let Some(raw) = header.strip_prefix("Content-Length:") {
            length = Some(raw.trim().parse::<usize>().map_err(io::Error::other)?);
        }
    }
    let Some(length) = length else { return Ok(None) };
    let mut body = vec![0u8; length];
    reader.read_exact(&mut body)?;
    Ok(Some(serde_json::from_slice(&body).map_err(io::Error::other)?))
}

fn labels(response: &Value) -> Vec<String> {
    response["result"]
        .as_array()
        .expect("completion array")
        .iter()
        .filter_map(|item| item["label"].as_str().map(str::to_owned))
        .collect()
}

#[test]
fn initialize_advertises_expected_capabilities() {
    let fx = Fixture::new();
    let mut s = Session::start(fx.root());
    let init = s.initialize(fx.root());
    let caps = &init["result"]["capabilities"];
    assert_eq!(caps["definitionProvider"], json!(true));
    assert_eq!(caps["textDocumentSync"]["openClose"], json!(true));
    assert_eq!(caps["textDocumentSync"]["change"], json!(1));
    assert!(caps["hoverProvider"].is_null());
    assert!(caps["documentSymbolProvider"].is_null());
    assert!(caps["documentFormattingProvider"].is_null());
    let triggers = caps["completionProvider"]["triggerCharacters"].as_array().unwrap();
    for t in ["[", "(", "#"] {
        assert!(triggers.contains(&json!(t)), "missing trigger {t}");
    }
    s.shutdown();
}

#[test]
fn initialize_registers_markdown_watcher_when_supported() {
    let fx = Fixture::new();
    let mut s = Session::start(fx.root());
    let caps = json!({"workspace": {"didChangeWatchedFiles": {"dynamicRegistration": true}}});
    s.initialize_with(fx.root(), caps);
    let req = s.wait_for(|m| m["method"] == "client/registerCapability");
    let regs = req["params"]["registrations"].as_array().unwrap();
    assert_eq!(regs.len(), 1);
    assert_eq!(regs[0]["method"], json!("workspace/didChangeWatchedFiles"));
    let watcher = &regs[0]["registerOptions"]["watchers"][0];
    assert_eq!(watcher["globPattern"], json!("**/*.md"));
    assert_eq!(watcher["kind"], json!(7));
    s.send(json!({"jsonrpc": "2.0", "id": req["id"], "result": Value::Null}));
    s.shutdown();
}

#[test]
#[ignore = "needs graph"]
fn diagnostics_publish_on_open_change_and_close() {
    let fx = Fixture::new();
    let mut s = Session::start(fx.root());
    s.initialize(fx.root());

    s.open(&fx.scratch, "# Scratch\n\n[bad](missing.md)\n");
    let diags = s.diagnostics(&fx.scratch);
    assert_eq!(diags.len(), 1);
    assert!(diags[0]["message"].as_str().unwrap().contains("target file not found"));
    assert_eq!(diags[0]["severity"], json!(1));
    assert_eq!(diags[0]["range"]["start"], json!({"line": 2, "character": 0}));
    assert_eq!(diags[0]["range"]["end"], json!({"line": 2, "character": 17}));

    s.change(&fx.scratch, "# Scratch\n\n[ok](b.md#target)\n");
    assert!(s.diagnostics(&fx.scratch).is_empty());

    s.notify("textDocument/didClose", json!({"textDocument": {"uri": fx.scratch}}));
    assert!(s.diagnostics(&fx.scratch).is_empty());
    s.shutdown();
}

#[test]
#[ignore = "needs graph"]
fn diagnostics_include_missing_heading_and_embed_errors() {
    let fx = Fixture::new();
    let mut s = Session::start(fx.root());
    s.initialize(fx.root());
    s.open(&fx.scratch, "# Scratch\n\n[bad](b.md#missing-heading)\n\n![[nope]]\n");
    let diags = s.diagnostics(&fx.scratch);
    assert_eq!(diags.len(), 2);
    assert!(diags[0]["message"].as_str().unwrap().contains("heading not found"));
    assert_eq!(diags[0]["range"]["start"]["line"], json!(2));
    assert!(diags[1]["message"].as_str().unwrap().contains("not found"));
    assert_eq!(diags[1]["range"]["start"]["line"], json!(4));
    s.shutdown();
}

#[test]
#[ignore = "needs graph"]
fn diagnostics_refresh_other_open_docs_on_inbound_change() {
    let fx = Fixture::new();
    let mut s = Session::start(fx.root());
    s.initialize(fx.root());
    s.open(&fx.a, &fx.ws.read("a.md"));
    assert!(s.diagnostics(&fx.a).is_empty());

    // Renaming the heading in b.md (unsaved) breaks a.md's anchors.
    s.open(&fx.b, "# B\n\n## Renamed\n");
    let _ = s.diagnostics(&fx.b);
    let diags = s.diagnostics(&fx.a);
    assert_eq!(diags.len(), 2);
    assert!(diags[0]["message"].as_str().unwrap().contains("heading not found"));
    s.shutdown();
}

#[test]
#[ignore = "needs graph"]
fn watched_file_events_refresh_graph_and_diagnostics() {
    let fx = Fixture::new();
    let mut s = Session::start(fx.root());
    s.initialize(fx.root());
    s.open(&fx.a, &fx.ws.read("a.md"));
    assert!(s.diagnostics(&fx.a).is_empty());

    fs::create_dir_all(fx.ws.path("archived")).unwrap();
    fs::rename(fx.ws.path("b.md"), fx.ws.path("archived/b.md")).unwrap();
    let moved = Url::from_file_path(fx.ws.path("archived/b.md")).unwrap();
    s.notify("workspace/didChangeWatchedFiles", json!({"changes": [
        {"uri": fx.b, "type": 3}, {"uri": moved, "type": 1}]}));
    let diags = s.diagnostics(&fx.a);
    assert!(!diags.is_empty());
    assert!(diags[0]["message"].as_str().unwrap().contains("target file not found: b.md"));

    s.change(&fx.a, "# A\n\n## Details\n\nSee [B](archived/b.md#target)\nSee [[archived/b#target]]\n");
    assert!(s.diagnostics(&fx.a).is_empty());
    s.shutdown();
}

#[test]
#[ignore = "needs graph"]
fn goto_definition_resolves_markdown_and_wikilink_targets() {
    let fx = Fixture::new();
    let mut s = Session::start(fx.root());
    s.initialize(fx.root());

    // `See [B](b.md#target)` on line 4; `See [[b#target]]` on line 5.
    let md = s.request(3, "textDocument/definition", Session::at(&fx.a, 4, 10));
    assert_eq!(md["result"]["uri"], json!(fx.b.as_str()));
    assert_eq!(md["result"]["range"]["start"]["line"], json!(2));

    let wiki = s.request(7, "textDocument/definition", Session::at(&fx.a, 5, 8));
    assert_eq!(wiki["result"]["uri"], json!(fx.b.as_str()));
    assert_eq!(wiki["result"]["range"]["start"]["line"], json!(2));

    // Off any link, and on a link to a missing file: no result.
    let none = s.request(8, "textDocument/definition", Session::at(&fx.a, 0, 1));
    assert!(none["result"].is_null());
    s.open(&fx.scratch, "# Scratch\n\n[bad](missing.md)\n");
    let bad = s.request(9, "textDocument/definition", Session::at(&fx.scratch, 2, 8));
    assert!(bad["result"].is_null());
    s.shutdown();
}

#[test]
#[ignore = "needs graph"]
fn completion_offers_files_then_headings_from_buffer_state() {
    let fx = Fixture::new();
    let mut s = Session::start(fx.root());
    s.initialize(fx.root());

    s.open(&fx.scratch, "# Scratch\n\nSee [[\n");
    let _ = s.diagnostics(&fx.scratch);
    let files = s.request(5, "textDocument/completion", Session::at(&fx.scratch, 2, 6));
    let names = labels(&files);
    assert!(names.contains(&"b".to_string()), "{names:?}");
    // The unsaved open file is itself a candidate (wikilinks drop `.md`).
    assert!(names.contains(&"scratch".to_string()), "{names:?}");
    let b = files["result"].as_array().unwrap().iter().find(|i| i["label"] == "b").unwrap();
    assert_eq!(b["kind"], json!(17)); // CompletionItemKind::FILE

    // Heading completion right after the change, without waiting for diagnostics.
    s.change(&fx.scratch, "# Scratch\n\nSee [[b#\n");
    let heads = s.request(6, "textDocument/completion", Session::at(&fx.scratch, 2, 8));
    let items = heads["result"].as_array().unwrap();
    let target = items.iter().find(|i| i["label"] == "Target").expect("Target heading");
    assert_eq!(target["kind"], json!(1)); // CompletionItemKind::TEXT
    assert_eq!(target["textEdit"]["newText"], json!("target"));
    assert_eq!(target["textEdit"]["range"]["start"]["character"], json!(6));

    // Markdown links keep `.md`.
    s.change(&fx.scratch, "# Scratch\n\n[x](\n");
    let md = s.request(7, "textDocument/completion", Session::at(&fx.scratch, 2, 4));
    assert!(labels(&md).contains(&"b.md".to_string()));

    // Bare `#` completes the current file's headings.
    s.change(&fx.scratch, "# Scratch\n\n[x](#\n");
    let own = s.request(8, "textDocument/completion", Session::at(&fx.scratch, 2, 5));
    assert_eq!(labels(&own), vec!["Scratch".to_string()]);
    s.shutdown();
}

#[test]
#[ignore = "needs graph"]
fn heading_completion_reverts_to_disk_after_target_close() {
    let fx = Fixture::new();
    let mut s = Session::start(fx.root());
    s.initialize(fx.root());

    s.open(&fx.b, "# B\n\n## Renamed\n");
    let _ = s.diagnostics(&fx.b);
    // Opening scratch re-publishes every open doc (b, then scratch, sorted by URI).
    s.open(&fx.scratch, "# Scratch\n\n[[b#\n");
    let _ = s.diagnostics(&fx.b);
    let _ = s.diagnostics(&fx.scratch);
    let open = s.request(11, "textDocument/completion", Session::at(&fx.scratch, 2, 4));
    assert!(labels(&open).contains(&"Renamed".to_string()));

    // Close clears b, then re-publishes the remaining open docs; consume both so
    // the completion below is ordered after the close handler.
    s.notify("textDocument/didClose", json!({"textDocument": {"uri": fx.b}}));
    assert!(s.diagnostics(&fx.b).is_empty());
    let _ = s.diagnostics(&fx.scratch);
    let closed = s.request(12, "textDocument/completion", Session::at(&fx.scratch, 2, 4));
    assert!(labels(&closed).contains(&"Target".to_string()));
    s.shutdown();
}
