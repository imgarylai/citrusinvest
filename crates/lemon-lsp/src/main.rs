//! `lemon-lsp` — a Language Server Protocol server for the Lemon DSL.
//!
//! This is a thin `tower-lsp` shim over [`lemon::services`]: every bit of
//! language intelligence — diagnostics, hover, completions — is a pure function
//! in the `lemon` library, which is unit-tested and measured for coverage. This
//! binary only translates between the LSP wire types and those pure functions
//! and runs the stdio event loop, so it is excluded from the coverage gate like
//! the other `src/main.rs` entry points.
//!
//! It keeps an in-memory copy of every open document (full-text sync) and
//! recomputes diagnostics on open and on every change. Hover and completion are
//! answered on demand from the stored text.
//!
//! Run it over stdio (how editors launch it):
//!
//! ```text
//! lemon-lsp
//! ```

use std::collections::HashMap;

use tokio::sync::Mutex;
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer, LspService, Server};

use lemon::services::{self, CompletionKind, Severity};

/// The server state: the client handle, the open-document store, and the
/// optional engine series list that enables unknown-series diagnostics.
struct Backend {
    client: Client,
    /// URI → current full text of the document.
    docs: Mutex<HashMap<Url, String>>,
    /// Known data-series names from `initializationOptions.series`. When set,
    /// diagnostics flag unknown/typo'd series; when empty, that check is skipped.
    series: Mutex<Vec<String>>,
}

impl Backend {
    fn new(client: Client) -> Self {
        Self {
            client,
            docs: Mutex::new(HashMap::new()),
            series: Mutex::new(Vec::new()),
        }
    }

    /// Compute diagnostics for `text` and publish them for `uri`.
    async fn publish(&self, uri: Url, text: &str, version: Option<i32>) {
        let series = self.series.lock().await.clone();
        let known = (!series.is_empty()).then_some(series.as_slice());
        let diags = services::diagnostics(text, known)
            .into_iter()
            .map(to_lsp_diagnostic)
            .collect();
        self.client.publish_diagnostics(uri, diags, version).await;
    }
}

#[tower_lsp::async_trait]
impl LanguageServer for Backend {
    async fn initialize(&self, params: InitializeParams) -> Result<InitializeResult> {
        // `initializationOptions: { "series": ["close", "pe", ...] }` supplies the
        // engine's known data-series names, enabling unknown-series diagnostics.
        if let Some(series) = params
            .initialization_options
            .as_ref()
            .and_then(|v| v.get("series"))
            .and_then(|v| v.as_array())
        {
            let names: Vec<String> = series
                .iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect();
            *self.series.lock().await = names;
        }
        Ok(InitializeResult {
            server_info: Some(ServerInfo {
                name: "lemon-lsp".to_string(),
                version: Some(env!("CARGO_PKG_VERSION").to_string()),
            }),
            capabilities: ServerCapabilities {
                // Full-document sync: the client sends the whole buffer on change.
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::FULL,
                )),
                hover_provider: Some(HoverProviderCapability::Simple(true)),
                completion_provider: Some(CompletionOptions {
                    // Re-trigger completion after `(` and `,` so keyword-argument
                    // names surface as soon as the user opens a call.
                    trigger_characters: Some(vec!["(".to_string(), ",".to_string()]),
                    ..Default::default()
                }),
                ..Default::default()
            },
        })
    }

    async fn initialized(&self, _: InitializedParams) {
        self.client
            .log_message(MessageType::INFO, "lemon-lsp ready")
            .await;
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let doc = params.text_document;
        self.docs
            .lock()
            .await
            .insert(doc.uri.clone(), doc.text.clone());
        self.publish(doc.uri, &doc.text, Some(doc.version)).await;
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        // FULL sync → the last change contains the entire new document text.
        let Some(change) = params.content_changes.into_iter().last() else {
            return;
        };
        let uri = params.text_document.uri;
        self.docs
            .lock()
            .await
            .insert(uri.clone(), change.text.clone());
        self.publish(uri, &change.text, Some(params.text_document.version))
            .await;
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        self.docs.lock().await.remove(&params.text_document.uri);
        // Clear diagnostics for the closed file.
        self.client
            .publish_diagnostics(params.text_document.uri, Vec::new(), None)
            .await;
    }

    async fn hover(&self, params: HoverParams) -> Result<Option<Hover>> {
        let pos = params.text_document_position_params;
        let text = match self.docs.lock().await.get(&pos.text_document.uri) {
            Some(t) => t.clone(),
            None => return Ok(None),
        };
        let (line, col) = from_lsp_position(pos.position);
        Ok(services::hover(&text, line, col).map(|h| Hover {
            contents: HoverContents::Markup(MarkupContent {
                kind: MarkupKind::Markdown,
                value: h.markdown,
            }),
            range: Some(Range {
                start: to_lsp_position(h.line, h.col),
                end: to_lsp_position(h.end_line, h.end_col),
            }),
        }))
    }

    async fn completion(&self, params: CompletionParams) -> Result<Option<CompletionResponse>> {
        let pos = params.text_document_position;
        let text = match self.docs.lock().await.get(&pos.text_document.uri) {
            Some(t) => t.clone(),
            None => return Ok(None),
        };
        let (line, col) = from_lsp_position(pos.position);
        let items = services::completions(&text, line, col)
            .into_iter()
            .map(to_lsp_completion)
            .collect();
        Ok(Some(CompletionResponse::Array(items)))
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }
}

/// LSP `Position` (0-based line, 0-based UTF-16 character) → the services'
/// 1-based `(line, col)`. Lemon source is ASCII identifiers/operators, so the
/// UTF-16 character offset equals the character column here.
fn from_lsp_position(p: Position) -> (usize, usize) {
    (p.line as usize + 1, p.character as usize + 1)
}

/// The services' 1-based `(line, col)` → an LSP `Position`.
fn to_lsp_position(line: usize, col: usize) -> Position {
    Position {
        line: (line.saturating_sub(1)) as u32,
        character: (col.saturating_sub(1)) as u32,
    }
}

fn to_lsp_diagnostic(d: services::Diagnostic) -> Diagnostic {
    Diagnostic {
        range: Range {
            start: to_lsp_position(d.line, d.col),
            end: to_lsp_position(d.end_line, d.end_col),
        },
        severity: Some(match d.severity {
            Severity::Error => DiagnosticSeverity::ERROR,
            Severity::Warning => DiagnosticSeverity::WARNING,
        }),
        source: Some("lemon".to_string()),
        message: d.message,
        ..Default::default()
    }
}

fn to_lsp_completion(c: services::CompletionItem) -> CompletionItem {
    CompletionItem {
        label: c.label,
        kind: Some(match c.kind {
            CompletionKind::Function => CompletionItemKind::FUNCTION,
            CompletionKind::Field => CompletionItemKind::FIELD,
            CompletionKind::Variable => CompletionItemKind::VARIABLE,
            CompletionKind::Series => CompletionItemKind::VALUE,
            CompletionKind::Keyword => CompletionItemKind::KEYWORD,
        }),
        detail: (!c.detail.is_empty()).then_some(c.detail),
        documentation: (!c.documentation.is_empty()).then_some(Documentation::MarkupContent(
            MarkupContent {
                kind: MarkupKind::Markdown,
                value: c.documentation,
            },
        )),
        insert_text: Some(c.insert_text),
        ..Default::default()
    }
}

#[tokio::main]
async fn main() {
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();
    let (service, socket) = LspService::new(Backend::new);
    Server::new(stdin, stdout, socket).serve(service).await;
}
