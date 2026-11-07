//! The language server: JSON-RPC messages in, responses and notifications out. It does no I/O,
//! so any transport can drive it: standard input and output ([`crate::run_stdio`]), or a
//! WebSocket in a web console.

use std::collections::HashMap;
use std::sync::Arc;

use fml_check::Definitions;
use fml_syntax::{FormatError, FormatOptions, Severity};
use lsp_types::{
    CompletionOptions, CompletionParams, DiagnosticSeverity, DidChangeTextDocumentParams,
    DidCloseTextDocumentParams, DidOpenTextDocumentParams, DocumentFormattingParams,
    DocumentSymbolParams, DocumentSymbolResponse, GotoDefinitionParams, GotoDefinitionResponse,
    HoverParams, HoverProviderCapability, InitializeResult, Location, OneOf, Position,
    PrepareRenameResponse, PublishDiagnosticsParams, ReferenceParams, RenameOptions, RenameParams,
    SemanticTokensFullOptions, SemanticTokensOptions, SemanticTokensParams,
    SemanticTokensServerCapabilities, ServerCapabilities, ServerInfo, SignatureHelpOptions,
    SignatureHelpParams, TextDocumentPositionParams, TextDocumentSyncCapability,
    TextDocumentSyncKind, TextEdit, Url, WorkDoneProgressOptions,
};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::document::Document;
use crate::{completion, hover, navigate, semantic, signature, symbols};

/// JSON-RPC error codes.
const METHOD_NOT_FOUND: i64 = -32601;
const INVALID_PARAMS: i64 = -32602;
const REQUEST_FAILED: i64 = -32803;
const SERVER_NOT_INITIALIZED: i64 = -32002;

/// The definitions maps are checked against, shared with the host.
pub type SharedDefinitions = Arc<dyn Definitions + Send + Sync>;

/// A language server for FML documents.
pub struct Server {
    definitions: SharedDefinitions,
    documents: HashMap<Url, Document>,
    initialized: bool,
    shutdown: bool,
    exited: bool,
}

impl Server {
    /// A server that checks maps against `definitions`.
    pub fn new(definitions: SharedDefinitions) -> Self {
        Self {
            definitions,
            documents: HashMap::new(),
            initialized: false,
            shutdown: false,
            exited: false,
        }
    }

    /// A server over the core definitions of a FHIR release.
    #[cfg(feature = "fhirschema")]
    pub fn with_release(release: fml_check::Release) -> Self {
        Self::new(Arc::new(fml_check::CoreDefinitions::new(release)))
    }

    /// Replaces the definitions, as when packages are added, and checks the open documents
    /// again. Returns their new diagnostics.
    pub fn set_definitions(&mut self, definitions: SharedDefinitions) -> Vec<Value> {
        self.definitions = definitions;
        let uris: Vec<Url> = self.documents.keys().cloned().collect();
        uris.into_iter()
            .filter_map(|uri| {
                let document = self.documents.remove(&uri)?;
                let document =
                    Document::new(document.text, document.version, self.definitions.as_ref());
                let message = diagnostics(&uri, &document);
                self.documents.insert(uri, document);
                Some(message)
            })
            .collect()
    }

    /// Whether the client sent `exit`: the transport should close.
    pub fn exited(&self) -> bool {
        self.exited
    }

    /// The process exit code after `exit`: 0 when `shutdown` came first.
    pub fn exit_code(&self) -> i32 {
        if self.shutdown { 0 } else { 1 }
    }

    /// What the server can do.
    pub fn capabilities() -> ServerCapabilities {
        ServerCapabilities {
            text_document_sync: Some(TextDocumentSyncCapability::Kind(TextDocumentSyncKind::FULL)),
            hover_provider: Some(HoverProviderCapability::Simple(true)),
            completion_provider: Some(CompletionOptions {
                trigger_characters: Some(vec![".".into(), ":".into()]),
                ..CompletionOptions::default()
            }),
            signature_help_provider: Some(SignatureHelpOptions {
                trigger_characters: Some(vec!["(".into(), ",".into()]),
                retrigger_characters: None,
                work_done_progress_options: WorkDoneProgressOptions::default(),
            }),
            definition_provider: Some(OneOf::Left(true)),
            references_provider: Some(OneOf::Left(true)),
            rename_provider: Some(OneOf::Right(RenameOptions {
                prepare_provider: Some(true),
                work_done_progress_options: WorkDoneProgressOptions::default(),
            })),
            document_symbol_provider: Some(OneOf::Left(true)),
            document_formatting_provider: Some(OneOf::Left(true)),
            semantic_tokens_provider: Some(
                SemanticTokensServerCapabilities::SemanticTokensOptions(SemanticTokensOptions {
                    legend: semantic::legend(),
                    full: Some(SemanticTokensFullOptions::Bool(true)),
                    range: None,
                    work_done_progress_options: WorkDoneProgressOptions::default(),
                }),
            ),
            ..ServerCapabilities::default()
        }
    }

    /// Handles one message and returns what to send back: the response to a request, and
    /// notifications such as diagnostics.
    pub fn handle(&mut self, message: &Value) -> Vec<Value> {
        let Some(method) = message.get("method").and_then(Value::as_str) else {
            // A response to something the server sent; it sends no requests.
            return Vec::new();
        };
        let params = message.get("params").cloned().unwrap_or(Value::Null);
        match message.get("id").cloned() {
            Some(id) => vec![self.request(id, method, params)],
            None => self.notification(method, params),
        }
    }

    fn request(&mut self, id: Value, method: &str, params: Value) -> Value {
        if method == "initialize" {
            self.initialized = true;
            let result = InitializeResult {
                capabilities: Self::capabilities(),
                server_info: Some(ServerInfo {
                    name: "fml-lsp".to_owned(),
                    version: Some(env!("CARGO_PKG_VERSION").to_owned()),
                }),
            };
            return response(id, json!(result));
        }
        if !self.initialized {
            return error(id, SERVER_NOT_INITIALIZED, "the server is not initialized");
        }
        let result = match method {
            "shutdown" => {
                self.shutdown = true;
                Ok(Value::Null)
            }
            "textDocument/hover" => self.with::<HoverParams>(params, |s, p| {
                let (document, offset) = s.at(&p.text_document_position_params)?;
                Some(json!(hover::hover(document, offset)?))
            }),
            "textDocument/completion" => self.with::<CompletionParams>(params, |s, p| {
                let (document, offset) = s.at(&p.text_document_position)?;
                let list = completion::completion(document, s.definitions.as_ref(), offset)?;
                Some(json!(list))
            }),
            "textDocument/signatureHelp" => self.with::<SignatureHelpParams>(params, |s, p| {
                let (document, offset) = s.at(&p.text_document_position_params)?;
                Some(json!(signature::signature_help(document, offset)?))
            }),
            "textDocument/definition" => self.with::<GotoDefinitionParams>(params, |s, p| {
                let position = &p.text_document_position_params;
                let (document, offset) = s.at(position)?;
                let range = navigate::definition(document, offset)?;
                Some(json!(GotoDefinitionResponse::Scalar(Location::new(
                    position.text_document.uri.clone(),
                    document.range(range),
                ))))
            }),
            "textDocument/references" => self.with::<ReferenceParams>(params, |s, p| {
                let position = &p.text_document_position;
                let (document, offset) = s.at(position)?;
                let (symbol, _) = navigate::symbol_at(document, offset)?;
                let mut ranges = navigate::occurrences(document, &symbol);
                if !p.context.include_declaration && !ranges.is_empty() {
                    ranges.remove(0);
                }
                let locations: Vec<Location> = ranges
                    .into_iter()
                    .map(|r| Location::new(position.text_document.uri.clone(), document.range(r)))
                    .collect();
                Some(json!(locations))
            }),
            "textDocument/prepareRename" => {
                self.with::<TextDocumentPositionParams>(params, |s, p| {
                    let (document, offset) = s.at(&p)?;
                    let (_, range) = navigate::symbol_at(document, offset)?;
                    Some(json!(PrepareRenameResponse::Range(document.range(range))))
                })
            }
            "textDocument/rename" => match serde_json::from_value::<RenameParams>(params) {
                Err(e) => Err((INVALID_PARAMS, e.to_string())),
                Ok(p) => {
                    let position = &p.text_document_position;
                    match self.at(position) {
                        None => Ok(Value::Null),
                        Some((document, offset)) => navigate::rename(
                            document,
                            &position.text_document.uri,
                            offset,
                            &p.new_name,
                        )
                        .map(|edit| json!(edit))
                        .map_err(|problem| (REQUEST_FAILED, problem)),
                    }
                }
            },
            "textDocument/documentSymbol" => self.with::<DocumentSymbolParams>(params, |s, p| {
                let document = s.documents.get(&p.text_document.uri)?;
                Some(json!(DocumentSymbolResponse::Nested(
                    symbols::document_symbols(document)
                )))
            }),
            "textDocument/formatting" => self.with::<DocumentFormattingParams>(params, |s, p| {
                let document = s.documents.get(&p.text_document.uri)?;
                let options = FormatOptions {
                    indent_width: p.options.tab_size as usize,
                    use_tabs: !p.options.insert_spaces,
                };
                match fml_syntax::format(&document.text, &options) {
                    Ok(formatted) if formatted == document.text => Some(json!([])),
                    Ok(formatted) => {
                        let end = document.position(fml_syntax::TextSize::of(&document.text));
                        let range = lsp_types::Range::new(Position::new(0, 0), end);
                        Some(json!([TextEdit::new(range, formatted)]))
                    }
                    // Nothing to format until the syntax errors are fixed.
                    Err(FormatError::Syntax(_) | FormatError::Unstable(_)) => None,
                }
            }),
            "textDocument/semanticTokens/full" => {
                self.with::<SemanticTokensParams>(params, |s, p| {
                    let document = s.documents.get(&p.text_document.uri)?;
                    Some(json!(semantic::semantic_tokens(document)))
                })
            }
            _ => Err((METHOD_NOT_FOUND, format!("unknown method `{method}`"))),
        };
        match result {
            Ok(value) => response(id, value),
            Err((code, message)) => error(id, code, &message),
        }
    }

    /// Parses the parameters and runs `f`; a `None` result is a `null` response.
    fn with<P: DeserializeOwned>(
        &mut self,
        params: Value,
        f: impl FnOnce(&Self, P) -> Option<Value>,
    ) -> Result<Value, (i64, String)> {
        let params: P =
            serde_json::from_value(params).map_err(|e| (INVALID_PARAMS, e.to_string()))?;
        Ok(f(self, params).unwrap_or(Value::Null))
    }

    fn at(
        &self,
        position: &TextDocumentPositionParams,
    ) -> Option<(&Document, fml_syntax::TextSize)> {
        let document = self.documents.get(&position.text_document.uri)?;
        Some((document, document.offset(position.position)))
    }

    fn notification(&mut self, method: &str, params: Value) -> Vec<Value> {
        match method {
            "exit" => {
                self.exited = true;
                Vec::new()
            }
            "textDocument/didOpen" => {
                let Ok(p) = serde_json::from_value::<DidOpenTextDocumentParams>(params) else {
                    return Vec::new();
                };
                self.open(
                    p.text_document.uri,
                    p.text_document.text,
                    p.text_document.version,
                )
            }
            "textDocument/didChange" => {
                let Ok(p) = serde_json::from_value::<DidChangeTextDocumentParams>(params) else {
                    return Vec::new();
                };
                // Full synchronisation: the last change is the whole text.
                let Some(change) = p.content_changes.into_iter().last() else {
                    return Vec::new();
                };
                self.open(p.text_document.uri, change.text, p.text_document.version)
            }
            "textDocument/didClose" => {
                let Ok(p) = serde_json::from_value::<DidCloseTextDocumentParams>(params) else {
                    return Vec::new();
                };
                self.documents.remove(&p.text_document.uri);
                vec![notification(
                    "textDocument/publishDiagnostics",
                    json!(PublishDiagnosticsParams::new(
                        p.text_document.uri,
                        Vec::new(),
                        None
                    )),
                )]
            }
            _ => Vec::new(),
        }
    }

    fn open(&mut self, uri: Url, text: String, version: i32) -> Vec<Value> {
        let document = Document::new(text, version, self.definitions.as_ref());
        let message = diagnostics(&uri, &document);
        self.documents.insert(uri, document);
        vec![message]
    }
}

fn diagnostics(uri: &Url, document: &Document) -> Value {
    let diagnostics = document
        .diagnostics
        .iter()
        .map(|d| lsp_types::Diagnostic {
            range: document.range(d.range),
            severity: Some(match d.severity {
                Severity::Error => DiagnosticSeverity::ERROR,
                Severity::Warning => DiagnosticSeverity::WARNING,
            }),
            source: Some("fml".to_owned()),
            message: d.message.clone(),
            ..lsp_types::Diagnostic::default()
        })
        .collect();
    notification(
        "textDocument/publishDiagnostics",
        json!(PublishDiagnosticsParams::new(
            uri.clone(),
            diagnostics,
            Some(document.version)
        )),
    )
}

fn response(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn error(id: Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

fn notification(method: &str, params: Value) -> Value {
    json!({ "jsonrpc": "2.0", "method": method, "params": params })
}
