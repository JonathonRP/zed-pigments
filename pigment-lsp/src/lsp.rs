use std::collections::HashMap;

use tokio::sync::RwLock;
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer, LspService, Server};

use crate::parser::{parse_document, ColorNode};
use crate::utils::{color_presentations, color_summary};

const LSP_NAME: &str = "Zed Pigments";

#[derive(Clone, Debug)]
struct DocumentState {
    language_id: String,
    version: i32,
    text: String,
    colors: Vec<ColorNode>,
}

struct Backend {
    client: Client,
    documents: RwLock<HashMap<Url, DocumentState>>,
}

impl Backend {
    async fn open_document(&self, document: TextDocumentItem) {
        let mut documents = self.documents.write().await;
        if documents
            .get(&document.uri)
            .is_some_and(|current| current.version > document.version)
        {
            return;
        }
        let colors = parse_document(&document.text, &document.language_id);
        documents.insert(
            document.uri,
            DocumentState {
                language_id: document.language_id,
                version: document.version,
                text: document.text,
                colors,
            },
        );
    }

    async fn change_document(&self, params: DidChangeTextDocumentParams) {
        let uri = params.text_document.uri;
        let version = params.text_document.version;
        let mut documents = self.documents.write().await;
        let Some(document) = documents.get_mut(&uri) else {
            drop(documents);
            self.client
                .log_message(
                    MessageType::WARNING,
                    format!("ignored change for unopened document {uri}"),
                )
                .await;
            return;
        };
        if version <= document.version {
            return;
        }

        match apply_content_changes(&document.text, &params.content_changes) {
            Ok(text) => {
                document.colors = parse_document(&text, &document.language_id);
                document.text = text;
                document.version = version;
            }
            Err(error) => {
                drop(documents);
                self.client
                    .log_message(
                        MessageType::WARNING,
                        format!("ignored invalid change for {uri}: {error}"),
                    )
                    .await;
            }
        }
    }
}

#[tower_lsp::async_trait]
impl LanguageServer for Backend {
    async fn initialize(&self, _: InitializeParams) -> Result<InitializeResult> {
        Ok(InitializeResult {
            server_info: Some(ServerInfo {
                name: LSP_NAME.to_owned(),
                version: Some(env!("CARGO_PKG_VERSION").to_owned()),
            }),
            capabilities: ServerCapabilities {
                position_encoding: Some(PositionEncodingKind::UTF16),
                text_document_sync: Some(TextDocumentSyncCapability::Options(
                    TextDocumentSyncOptions {
                        open_close: Some(true),
                        change: Some(TextDocumentSyncKind::FULL),
                        ..Default::default()
                    },
                )),
                color_provider: Some(ColorProviderCapability::Simple(true)),
                hover_provider: Some(HoverProviderCapability::Simple(true)),
                ..ServerCapabilities::default()
            },
        })
    }

    async fn initialized(&self, _: InitializedParams) {
        self.client
            .log_message(MessageType::INFO, "Zed Pigments initialized")
            .await;
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        self.open_document(params.text_document).await;
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        self.documents
            .write()
            .await
            .remove(&params.text_document.uri);
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        self.change_document(params).await;
    }

    async fn hover(&self, params: HoverParams) -> Result<Option<Hover>> {
        let uri = params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;
        let documents = self.documents.read().await;
        let Some(document) = documents.get(&uri) else {
            return Ok(None);
        };
        let Some(node) = document
            .colors
            .iter()
            .find(|node| position >= node.range.start && position < node.range.end)
        else {
            return Ok(None);
        };

        Ok(Some(Hover {
            contents: HoverContents::Markup(MarkupContent {
                kind: MarkupKind::Markdown,
                value: color_summary(node.lsp_color()),
            }),
            range: Some(node.range),
        }))
    }

    async fn document_color(&self, params: DocumentColorParams) -> Result<Vec<ColorInformation>> {
        let documents = self.documents.read().await;
        let colors = documents
            .get(&params.text_document.uri)
            .map(|document| {
                document
                    .colors
                    .iter()
                    .map(|node| ColorInformation {
                        range: node.range,
                        color: node.lsp_color(),
                    })
                    .collect()
            })
            .unwrap_or_default();
        Ok(colors)
    }

    async fn color_presentation(
        &self,
        params: ColorPresentationParams,
    ) -> Result<Vec<ColorPresentation>> {
        let documents = self.documents.read().await;
        let original = documents
            .get(&params.text_document.uri)
            .and_then(|document| {
                document
                    .colors
                    .iter()
                    .find(|node| node.range == params.range)
                    .map(|node| (node.matched.as_str(), node.lsp_color()))
            });
        Ok(color_presentations(params.color, params.range, original))
    }
}

fn apply_content_changes(
    original: &str,
    changes: &[TextDocumentContentChangeEvent],
) -> std::result::Result<String, String> {
    if changes.is_empty() {
        return Err("change list was empty".to_owned());
    }

    let mut text = original.to_owned();
    for change in changes {
        if let Some(range) = change.range {
            let start = byte_offset(&text, range.start)
                .ok_or_else(|| format!("invalid range start {:?}", range.start))?;
            let end = byte_offset(&text, range.end)
                .ok_or_else(|| format!("invalid range end {:?}", range.end))?;
            if start > end {
                return Err("range start followed range end".to_owned());
            }
            text.replace_range(start..end, &change.text);
        } else {
            text = change.text.clone();
        }
    }
    Ok(text)
}

fn byte_offset(text: &str, position: Position) -> Option<usize> {
    let mut line_start = 0;
    for _ in 0..position.line {
        let newline = text[line_start..].find('\n')?;
        line_start += newline + 1;
    }

    let mut line_end = text[line_start..]
        .find('\n')
        .map_or(text.len(), |relative| line_start + relative);
    if line_end > line_start && text.as_bytes()[line_end - 1] == b'\r' {
        line_end -= 1;
    }

    let target = position.character as usize;
    let mut utf16 = 0;
    for (relative, character) in text[line_start..line_end].char_indices() {
        if utf16 == target {
            return Some(line_start + relative);
        }
        utf16 += character.len_utf16();
        if utf16 > target {
            return None;
        }
    }
    (utf16 == target).then_some(line_end)
}

pub async fn start() {
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();
    let (service, socket) = LspService::new(|client| Backend {
        client,
        documents: RwLock::new(HashMap::new()),
    });
    Server::new(stdin, stdout, socket).serve(service).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn applies_full_and_incremental_utf16_changes() {
        let original = "😀 color: #fff;";
        let changes = vec![TextDocumentContentChangeEvent {
            range: Some(Range::new(Position::new(0, 10), Position::new(0, 14))),
            range_length: Some(4),
            text: "red".to_owned(),
        }];
        assert_eq!(
            apply_content_changes(original, &changes).unwrap(),
            "😀 color: red;"
        );

        let full = vec![TextDocumentContentChangeEvent {
            range: None,
            range_length: None,
            text: "a: #000;".to_owned(),
        }];
        assert_eq!(apply_content_changes(original, &full).unwrap(), "a: #000;");
    }

    #[test]
    fn rejects_positions_inside_surrogate_pairs() {
        assert_eq!(byte_offset("😀", Position::new(0, 1)), None);
        assert_eq!(byte_offset("😀", Position::new(0, 2)), Some(4));
    }
}
