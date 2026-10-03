//! Server state and the `LanguageServer` dispatch.
//!
//! [`State`] holds the workspace, the graph behind an async `RwLock`, the text
//! of every open document, and the debounce generation per document.
//! [`Backend`] is an `Arc<State>` so notification handlers can spawn the
//! debounced diagnostics publish without blocking the (concurrent) dispatcher.

use std::collections::HashMap;
use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex as StdMutex, OnceLock};
use std::time::Duration;

use tokio::sync::{Mutex, RwLock};
use tower_lsp::jsonrpc::Result as LspResult;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer};

use crate::graph::Graph;
use crate::workspace::Workspace;

use super::{completion, definition, diagnostics};

/// How long to wait after the last `didChange` before publishing diagnostics.
const DEBOUNCE: Duration = Duration::from_millis(150);

pub(super) struct State {
    pub(super) client: Client,
    ws: OnceLock<Workspace>,
    graph: RwLock<Option<Graph>>,
    /// Serializes graph construction so concurrent handlers build it once.
    build: Mutex<()>,
    docs: RwLock<HashMap<Url, String>>,
    /// Debounce generation per open document; a publish fires only if still current.
    pending: StdMutex<HashMap<Url, u64>>,
    dynamic_watch: AtomicBool,
}

pub(super) struct Backend(Arc<State>);

impl Deref for Backend {
    type Target = State;
    fn deref(&self) -> &State {
        &self.0
    }
}

impl Backend {
    pub(super) fn new(client: Client, ws: Option<Workspace>) -> Self {
        let lock = OnceLock::new();
        if let Some(ws) = ws {
            let _ = lock.set(ws);
        }
        Self(Arc::new(State {
            client,
            ws: lock,
            graph: RwLock::new(None),
            build: Mutex::new(()),
            docs: RwLock::new(HashMap::new()),
            pending: StdMutex::new(HashMap::new()),
            dynamic_watch: AtomicBool::new(false),
        }))
    }
}

impl State {
    pub(super) fn ws(&self) -> Option<&Workspace> {
        self.ws.get()
    }

    /// Root-relative path for a `file:` URI of a markdown file inside the workspace.
    pub(super) fn rel(&self, uri: &Url) -> Option<PathBuf> {
        let abs = uri.to_file_path().ok()?;
        if !abs.extension().is_some_and(|e| e.eq_ignore_ascii_case("md")) {
            return None;
        }
        self.ws()?.rel(&abs)
    }

    pub(super) fn uri_for(&self, rel: &Path) -> Option<Url> {
        Url::from_file_path(self.ws()?.abs(rel)).ok()
    }

    /// Current text for a document: the open buffer, else the file on disk.
    pub(super) async fn text(&self, uri: &Url) -> Option<String> {
        if let Some(text) = self.docs.read().await.get(uri) {
            return Some(text.clone());
        }
        std::fs::read_to_string(uri.to_file_path().ok()?).ok()
    }

    pub(super) async fn open_uris(&self) -> Vec<Url> {
        let mut uris: Vec<Url> = self.docs.read().await.keys().cloned().collect();
        uris.sort_by(|a, b| a.as_str().cmp(b.as_str()));
        uris
    }

    /// Build the graph on first use. Returns `false` (after logging) when the
    /// workspace is unknown or the build failed.
    pub(super) async fn ensure_graph(&self) -> bool {
        let _guard = self.build.lock().await;
        if self.graph.read().await.is_some() {
            return true;
        }
        let Some(ws) = self.ws().cloned() else {
            self.client.log_message(MessageType::ERROR, "kdb: no workspace (.kdb/) found").await;
            return false;
        };
        let built = tokio::task::spawn_blocking(move || Graph::build(&ws)).await;
        let message = match built {
            Ok(Ok(graph)) => {
                *self.graph.write().await = Some(graph);
                return true;
            }
            Ok(Err(e)) => format!("kdb: failed to build graph: {e:#}"),
            Err(e) => format!("kdb: graph build panicked: {e}"),
        };
        self.client.log_message(MessageType::ERROR, message).await;
        false
    }

    /// Read access to the graph, building it first if needed.
    pub(super) async fn with_graph<T>(&self, f: impl FnOnce(&Graph) -> T) -> Option<T> {
        if !self.ensure_graph().await {
            return None;
        }
        self.graph.read().await.as_ref().map(f)
    }

    /// Insert or replace one document in the graph from `text`.
    pub(super) async fn upsert(&self, rel: &Path, text: &str) {
        if !self.ensure_graph().await {
            return;
        }
        if let Some(graph) = self.graph.write().await.as_mut() {
            graph.upsert(rel, text);
        }
    }

    /// Reload one document from disk, removing it from the graph if it is gone.
    pub(super) async fn sync_from_disk(&self, rel: &Path) {
        let Some(ws) = self.ws() else { return };
        match std::fs::read_to_string(ws.abs(rel)) {
            Ok(text) => self.upsert(rel, &text).await,
            Err(_) => {
                if let Some(graph) = self.graph.write().await.as_mut() {
                    graph.remove(rel);
                }
            }
        }
    }

    async fn register_watcher(&self) {
        if !self.dynamic_watch.load(Ordering::Relaxed) {
            return;
        }
        let options = DidChangeWatchedFilesRegistrationOptions {
            watchers: vec![FileSystemWatcher {
                glob_pattern: GlobPattern::String("**/*.md".into()),
                kind: Some(WatchKind::Create | WatchKind::Change | WatchKind::Delete),
            }],
        };
        let registration = Registration {
            id: "kdb-watch-markdown".into(),
            method: "workspace/didChangeWatchedFiles".into(),
            register_options: serde_json::to_value(options).ok(),
        };
        if let Err(e) = self.client.register_capability(vec![registration]).await {
            let msg = format!("kdb: failed to register markdown watcher: {e}");
            self.client.log_message(MessageType::WARNING, msg).await;
        }
    }

    /// Apply watched-file events to the graph; `true` if anything changed.
    async fn apply_watched(&self, params: &DidChangeWatchedFilesParams) -> bool {
        let mut changed = false;
        for change in &params.changes {
            let Some(rel) = self.rel(&change.uri) else { continue };
            if change.typ == FileChangeType::DELETED {
                if let Some(graph) = self.graph.write().await.as_mut() {
                    graph.remove(&rel);
                }
            } else if let Some(text) = self.docs.read().await.get(&change.uri).cloned() {
                self.upsert(&rel, &text).await;
            } else {
                self.sync_from_disk(&rel).await;
            }
            changed = true;
        }
        changed
    }
}

#[tower_lsp::async_trait]
impl LanguageServer for Backend {
    async fn initialize(&self, params: InitializeParams) -> LspResult<InitializeResult> {
        if self.ws().is_none()
            && let Some(ws) = params
                .root_uri
                .as_ref()
                .and_then(|u| u.to_file_path().ok())
                .and_then(|p| Workspace::find(&p).ok())
        {
            let _ = self.ws.set(ws);
        }
        let dynamic = params
            .capabilities
            .workspace
            .as_ref()
            .and_then(|w| w.did_change_watched_files)
            .and_then(|c| c.dynamic_registration)
            .unwrap_or(false);
        self.dynamic_watch.store(dynamic, Ordering::Relaxed);

        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Options(
                    TextDocumentSyncOptions {
                        open_close: Some(true),
                        change: Some(TextDocumentSyncKind::FULL),
                        ..TextDocumentSyncOptions::default()
                    },
                )),
                definition_provider: Some(OneOf::Left(true)),
                completion_provider: Some(CompletionOptions {
                    trigger_characters: Some(
                        ["[", "(", "#", "/"].iter().map(|s| s.to_string()).collect(),
                    ),
                    ..CompletionOptions::default()
                }),
                ..ServerCapabilities::default()
            },
            ..InitializeResult::default()
        })
    }

    async fn initialized(&self, _: InitializedParams) {
        self.register_watcher().await;
        let _ = self.ensure_graph().await;
        if let Some(ws) = self.ws() {
            let msg = format!("kdb lsp connected at {}", ws.root.display());
            self.client.log_message(MessageType::INFO, msg).await;
        }
    }

    async fn shutdown(&self) -> LspResult<()> {
        Ok(())
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let uri = params.text_document.uri;
        let text = params.text_document.text;
        self.docs.write().await.insert(uri.clone(), text.clone());
        if let Some(rel) = self.rel(&uri) {
            self.upsert(&rel, &text).await;
        }
        diagnostics::publish_all(self).await;
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        let Some(change) = params.content_changes.into_iter().last() else { return };
        let uri = params.text_document.uri;
        self.docs.write().await.insert(uri.clone(), change.text.clone());
        if let Some(rel) = self.rel(&uri) {
            self.upsert(&rel, &change.text).await;
        }
        let generation = {
            let mut pending = self.pending.lock().unwrap_or_else(|p| p.into_inner());
            let g = pending.entry(uri.clone()).or_insert(0);
            *g += 1;
            *g
        };
        let state = Arc::clone(&self.0);
        tokio::spawn(async move {
            tokio::time::sleep(DEBOUNCE).await;
            let current = state
                .pending
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .get(&uri)
                .copied();
            if current == Some(generation) {
                diagnostics::publish_all(&state).await;
            }
        });
    }

    async fn did_save(&self, params: DidSaveTextDocumentParams) {
        let uri = params.text_document.uri;
        if let (Some(rel), Some(text)) = (self.rel(&uri), self.text(&uri).await) {
            self.upsert(&rel, &text).await;
        }
        diagnostics::publish_all(self).await;
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        let uri = params.text_document.uri;
        self.docs.write().await.remove(&uri);
        self.pending.lock().unwrap_or_else(|p| p.into_inner()).remove(&uri);
        if let Some(rel) = self.rel(&uri) {
            self.sync_from_disk(&rel).await;
        }
        self.client.publish_diagnostics(uri, Vec::new(), None).await;
        diagnostics::publish_all(self).await;
    }

    async fn did_change_watched_files(&self, params: DidChangeWatchedFilesParams) {
        if self.apply_watched(&params).await {
            diagnostics::publish_all(self).await;
        }
    }

    async fn goto_definition(
        &self,
        params: GotoDefinitionParams,
    ) -> LspResult<Option<GotoDefinitionResponse>> {
        Ok(definition::goto_definition(self, params).await)
    }

    async fn completion(&self, params: CompletionParams) -> LspResult<Option<CompletionResponse>> {
        Ok(completion::completion(self, params).await)
    }
}

/// Byte offset of an LSP position (UTF-16 columns). `None` if the line does not exist.
pub(super) fn position_to_offset(text: &str, pos: Position) -> Option<usize> {
    let mut start = 0usize;
    for _ in 0..pos.line {
        start += text[start..].find('\n')? + 1;
    }
    let end = text[start..].find('\n').map_or(text.len(), |i| start + i);
    let mut col = 0u32;
    for (i, ch) in text[start..end].char_indices() {
        if col >= pos.character {
            return Some(start + i);
        }
        col += ch.len_utf16() as u32;
    }
    Some(end)
}

/// LSP position (UTF-16 columns) of a byte offset, clamped to the text.
pub(super) fn offset_to_position(text: &str, offset: usize) -> Position {
    let mut offset = offset.min(text.len());
    while !text.is_char_boundary(offset) {
        offset -= 1;
    }
    let before = &text[..offset];
    let line = before.matches('\n').count();
    let line_start = before.rfind('\n').map_or(0, |i| i + 1);
    let character = before[line_start..].encode_utf16().count();
    Position::new(line as u32, character as u32)
}

/// Relative path from directory `from_dir` to file `to` (both root-relative).
pub(super) fn relative_path(from_dir: &Path, to: &Path) -> PathBuf {
    let from: Vec<_> = from_dir.components().collect();
    let to: Vec<_> = to.components().collect();
    let common = from.iter().zip(&to).take_while(|(a, b)| a == b).count();
    let mut out = PathBuf::new();
    for _ in common..from.len() {
        out.push("..");
    }
    for c in &to[common..] {
        out.push(c);
    }
    out
}

/// Forward-slash string for use in a link target.
pub(super) fn slash(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn positions_round_trip_utf16() {
        let text = "ab\nc\u{1F600}d\n";
        assert_eq!(position_to_offset(text, Position::new(0, 1)), Some(1));
        assert_eq!(position_to_offset(text, Position::new(1, 0)), Some(3));
        // emoji is two UTF-16 units, four bytes
        assert_eq!(position_to_offset(text, Position::new(1, 3)), Some(8));
        assert_eq!(position_to_offset(text, Position::new(1, 99)), Some(9));
        assert_eq!(position_to_offset(text, Position::new(5, 0)), None);
        assert_eq!(offset_to_position(text, 8), Position::new(1, 3));
        assert_eq!(offset_to_position(text, 3), Position::new(1, 0));
        assert_eq!(offset_to_position(text, 999), Position::new(2, 0));
    }

    #[test]
    fn relative_paths() {
        let rp = |a: &str, b: &str| slash(&relative_path(Path::new(a), Path::new(b)));
        assert_eq!(rp("", "b.md"), "b.md");
        assert_eq!(rp("docs", "docs/x.md"), "x.md");
        assert_eq!(rp("docs/a", "notes/y.md"), "../../notes/y.md");
    }
}
