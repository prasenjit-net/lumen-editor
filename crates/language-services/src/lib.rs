//! Pure language-service contracts shared by parser-backed highlighting and an
//! eventual Language Server Protocol client.
//!
//! This crate deliberately owns neither native processes nor UI state. The app
//! and service layers use these types to keep document versions, UTF-16 protocol
//! positions, token snapshots, and language registration consistent.
use std::{ops::Range, path::Path};

pub mod highlight;
pub mod protocol;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct LanguageId(&'static str);

impl LanguageId {
    pub const RUST: Self = Self("rust");
    pub const GO: Self = Self("go");
    pub const JSON: Self = Self("json");

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ServerDescriptor {
    pub lsp_language_id: &'static str,
    pub executable_candidates: &'static [&'static str],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParserKind {
    TreeSitterRust,
    TreeSitterGo,
    TreeSitterJson,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ParserDescriptor {
    pub kind: ParserKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LanguageDescriptor {
    pub id: LanguageId,
    pub label: &'static str,
    pub aliases: &'static [&'static str],
    pub filenames: &'static [&'static str],
    pub filename_prefixes: &'static [&'static str],
    pub extensions: &'static [&'static str],
    pub workspace_markers: &'static [&'static str],
    pub parser: Option<ParserDescriptor>,
    pub server: Option<ServerDescriptor>,
}

const RUST_SERVER: ServerDescriptor = ServerDescriptor {
    lsp_language_id: "rust",
    executable_candidates: &["rust-analyzer"],
};

const GO_SERVER: ServerDescriptor = ServerDescriptor {
    lsp_language_id: "go",
    executable_candidates: &["gopls"],
};

const RUST: LanguageDescriptor = LanguageDescriptor {
    id: LanguageId::RUST,
    label: "Rust",
    aliases: &["rs"],
    filenames: &[],
    filename_prefixes: &[],
    extensions: &["rs"],
    workspace_markers: &["Cargo.toml"],
    parser: Some(ParserDescriptor {
        kind: ParserKind::TreeSitterRust,
    }),
    server: Some(RUST_SERVER),
};

const GO: LanguageDescriptor = LanguageDescriptor {
    id: LanguageId::GO,
    label: "Go",
    aliases: &["golang"],
    filenames: &[],
    filename_prefixes: &[],
    extensions: &["go"],
    workspace_markers: &["go.work", "go.mod"],
    parser: Some(ParserDescriptor {
        kind: ParserKind::TreeSitterGo,
    }),
    server: Some(GO_SERVER),
};

const JSON: LanguageDescriptor = LanguageDescriptor {
    id: LanguageId::JSON,
    label: "JSON",
    aliases: &[],
    filenames: &[],
    filename_prefixes: &[],
    extensions: &["json"],
    workspace_markers: &[],
    parser: Some(ParserDescriptor {
        kind: ParserKind::TreeSitterJson,
    }),
    server: None,
};

const BUILTIN_LANGUAGES: &[LanguageDescriptor] = &[RUST, GO, JSON];

/// Static language catalog. Language contributions will extend this catalog in
/// one location, instead of scattering file-type checks throughout the app.
pub struct LanguageRegistry;

impl LanguageRegistry {
    #[must_use]
    pub const fn builtins() -> &'static [LanguageDescriptor] {
        BUILTIN_LANGUAGES
    }

    #[must_use]
    pub fn by_id(id: LanguageId) -> Option<&'static LanguageDescriptor> {
        Self::builtins()
            .iter()
            .find(|descriptor| descriptor.id == id)
    }

    #[must_use]
    pub fn by_label(label: &str) -> Option<&'static LanguageDescriptor> {
        Self::builtins()
            .iter()
            .find(|descriptor| descriptor.label == label)
    }

    /// Full filenames take priority over prefixes, which take priority over an
    /// extension. This preserves the detection policy needed for build files.
    #[must_use]
    pub fn detect(path: &Path) -> Option<&'static LanguageDescriptor> {
        let name = path.file_name()?.to_str()?;
        Self::builtins()
            .iter()
            .find(|descriptor| descriptor.filenames.contains(&name))
            .or_else(|| {
                Self::builtins().iter().find(|descriptor| {
                    descriptor
                        .filename_prefixes
                        .iter()
                        .any(|prefix| name.starts_with(prefix))
                })
            })
            .or_else(|| {
                let extension = path.extension()?.to_str()?;
                Self::builtins()
                    .iter()
                    .find(|descriptor| descriptor.extensions.contains(&extension))
            })
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DocumentVersion(pub u64);

impl DocumentVersion {
    #[must_use]
    pub const fn next(self) -> Self {
        Self(self.0.saturating_add(1))
    }
}

/// Stable identity supplied by Lumen's document model. It survives tab
/// reordering; a path or tab index is never sufficient for LSP routing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DocumentId(pub u64);

/// A canonical document URI supplied by the app/service boundary. This crate
/// intentionally does not canonicalize paths or touch the filesystem.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DocumentUri(String);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DocumentUriError {
    Empty,
    ContainsControlCharacter,
}

impl DocumentUri {
    pub fn new(uri: impl Into<String>) -> Result<Self, DocumentUriError> {
        let uri = uri.into();
        if uri.is_empty() {
            return Err(DocumentUriError::Empty);
        }
        if uri.chars().any(char::is_control) {
            return Err(DocumentUriError::ContainsControlCharacter);
        }
        Ok(Self(uri))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The stable local document identity together with the URI advertised to a
/// server. A rename produces a new key so delayed responses for the old URI
/// cannot decorate the renamed document.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct DocumentKey {
    pub id: DocumentId,
    pub uri: DocumentUri,
}

/// Increments whenever a server session is replaced for a workspace.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SessionGeneration(pub u64);

impl SessionGeneration {
    #[must_use]
    pub const fn next(self) -> Self {
        Self(self.0.saturating_add(1))
    }
}

/// Version assigned by the LSP session to full-text `didOpen`/`didChange`
/// synchronization. It is distinct from Lumen's local document revision.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ServerDocumentVersion(pub u64);

impl ServerDocumentVersion {
    #[must_use]
    pub const fn next(self) -> Self {
        Self(self.0.saturating_add(1))
    }
}

/// Identity captured when sending a request. Replies are accepted only when
/// every field still matches the document/session currently displayed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RequestStamp {
    pub document: DocumentKey,
    pub document_version: DocumentVersion,
    pub server_version: ServerDocumentVersion,
    pub session: SessionGeneration,
}

impl RequestStamp {
    #[must_use]
    pub fn is_current_for(&self, active: &Self) -> bool {
        self == active
    }
}

/// A protocol position. LSP uses zero-based UTF-16 code-unit columns.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Position {
    pub line: u32,
    pub character: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PositionError {
    LineOutOfBounds,
    CharacterOutOfBounds,
    SplitsUtf16CodePoint,
    ByteOutOfBounds,
    SplitsUtf8CodePoint,
}

#[derive(Clone, Debug)]
struct TextLine {
    content: Range<usize>,
    full_end: usize,
}

fn text_lines(text: &str) -> Vec<TextLine> {
    let bytes = text.as_bytes();
    let mut lines = Vec::new();
    let mut start = 0;
    for (index, byte) in bytes.iter().copied().enumerate() {
        if byte == b'\n' {
            let content_end = if index > start && bytes[index - 1] == b'\r' {
                index - 1
            } else {
                index
            };
            lines.push(TextLine {
                content: start..content_end,
                full_end: index + 1,
            });
            start = index + 1;
        }
    }
    lines.push(TextLine {
        content: start..text.len(),
        full_end: text.len(),
    });
    lines
}

/// Convert an LSP UTF-16 position into a UTF-8 source-byte offset. CRLF is one
/// line ending, not two editable characters.
pub fn position_to_byte(text: &str, position: Position) -> Result<usize, PositionError> {
    let lines = text_lines(text);
    let line = lines
        .get(usize::try_from(position.line).map_err(|_| PositionError::LineOutOfBounds)?)
        .ok_or(PositionError::LineOutOfBounds)?;
    let content = &text[line.content.clone()];
    let mut units = 0_u32;
    for (offset, character) in content.char_indices() {
        if units == position.character {
            return Ok(line.content.start + offset);
        }
        let width = character.len_utf16() as u32;
        if position.character > units && position.character < units + width {
            return Err(PositionError::SplitsUtf16CodePoint);
        }
        units += width;
    }
    if units == position.character {
        Ok(line.content.end)
    } else {
        Err(PositionError::CharacterOutOfBounds)
    }
}

/// Convert a UTF-8 source-byte offset into LSP's zero-based UTF-16 position.
pub fn byte_to_position(text: &str, byte: usize) -> Result<Position, PositionError> {
    if byte > text.len() {
        return Err(PositionError::ByteOutOfBounds);
    }
    if !text.is_char_boundary(byte) {
        return Err(PositionError::SplitsUtf8CodePoint);
    }
    for (line_index, line) in text_lines(text).iter().enumerate() {
        if byte < line.content.start {
            break;
        }
        if byte <= line.content.end {
            let units = text[line.content.start..byte]
                .chars()
                .map(|character| character.len_utf16() as u32)
                .sum();
            return Ok(Position {
                line: u32::try_from(line_index).expect("document has too many lines"),
                character: units,
            });
        }
        if byte < line.full_end {
            let units = text[line.content.clone()]
                .chars()
                .map(|character| character.len_utf16() as u32)
                .sum();
            return Ok(Position {
                line: u32::try_from(line_index).expect("document has too many lines"),
                character: units,
            });
        }
    }
    Err(PositionError::ByteOutOfBounds)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TokenClass {
    Comment,
    String,
    Escape,
    Keyword,
    Operator,
    Punctuation,
    Number,
    Boolean,
    Constant,
    Type,
    Function,
    Method,
    Parameter,
    Variable,
    Property,
    Namespace,
    Attribute,
    Macro,
    Tag,
    Selector,
    Heading,
    Invalid,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TokenSpan {
    pub range: Range<usize>,
    pub class: TokenClass,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TokenSnapshotError {
    InvalidBoundary,
    InvalidOrder,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TokenSnapshot {
    pub version: DocumentVersion,
    pub spans: Vec<TokenSpan>,
}

/// Server-advertised semantic-token names. The transport layer obtains this
/// during `initialize`; this pure representation is safe to test without a
/// running language server.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SemanticTokenLegend {
    pub token_types: Vec<String>,
    pub token_modifiers: Vec<String>,
}

/// A semantic token decoded into the editor's UTF-8 source coordinate space.
/// Modifiers retain the LSP bitset so later UI policy can use them without
/// reparsing a server message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SemanticToken {
    pub range: Range<usize>,
    pub class: TokenClass,
    pub modifiers: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SemanticTokenError {
    InvalidDataLength,
    ArithmeticOverflow,
    UnknownTokenType,
    InvalidPosition(PositionError),
    EmptyRange,
}

/// Decode LSP's delta-encoded semantic token payload. Ranges are converted
/// immediately so no UI component needs to interpret UTF-16 columns.
pub fn decode_semantic_tokens(
    text: &str,
    legend: &SemanticTokenLegend,
    data: &[u32],
) -> Result<Vec<SemanticToken>, SemanticTokenError> {
    if !data.len().is_multiple_of(5) {
        return Err(SemanticTokenError::InvalidDataLength);
    }
    let mut line = 0_u32;
    let mut character = 0_u32;
    let mut tokens = Vec::with_capacity(data.len() / 5);
    for chunk in data.as_chunks::<5>().0 {
        line = line
            .checked_add(chunk[0])
            .ok_or(SemanticTokenError::ArithmeticOverflow)?;
        character = if chunk[0] == 0 {
            character
                .checked_add(chunk[1])
                .ok_or(SemanticTokenError::ArithmeticOverflow)?
        } else {
            chunk[1]
        };
        let start = Position { line, character };
        let end = Position {
            line,
            character: character
                .checked_add(chunk[2])
                .ok_or(SemanticTokenError::ArithmeticOverflow)?,
        };
        let range = position_to_byte(text, start)
            .and_then(|start| position_to_byte(text, end).map(|end| start..end))
            .map_err(SemanticTokenError::InvalidPosition)?;
        if range.is_empty() {
            return Err(SemanticTokenError::EmptyRange);
        }
        let token_type = legend
            .token_types
            .get(usize::try_from(chunk[3]).map_err(|_| SemanticTokenError::UnknownTokenType)?)
            .ok_or(SemanticTokenError::UnknownTokenType)?;
        tokens.push(SemanticToken {
            range,
            class: semantic_class(token_type),
            modifiers: chunk[4],
        });
    }
    Ok(tokens)
}

fn semantic_class(token_type: &str) -> TokenClass {
    match token_type {
        "namespace" => TokenClass::Namespace,
        "type" | "class" | "enum" | "interface" | "struct" | "typeParameter" => TokenClass::Type,
        "parameter" => TokenClass::Parameter,
        "variable" => TokenClass::Variable,
        "property" => TokenClass::Property,
        "enumMember" | "enum_member" => TokenClass::Constant,
        "function" => TokenClass::Function,
        "method" => TokenClass::Method,
        "macro" => TokenClass::Macro,
        "keyword" => TokenClass::Keyword,
        "string" | "regexp" => TokenClass::String,
        "number" => TokenClass::Number,
        "comment" => TokenClass::Comment,
        "operator" => TokenClass::Operator,
        "decorator" => TokenClass::Attribute,
        _ => TokenClass::Variable,
    }
}

impl TokenSnapshot {
    /// Reject invalid or overlapping ranges before they reach the renderer.
    pub fn new(
        text: &str,
        version: DocumentVersion,
        spans: Vec<TokenSpan>,
    ) -> Result<Self, TokenSnapshotError> {
        let mut previous_end = 0;
        for span in &spans {
            if span.range.start > span.range.end
                || span.range.end > text.len()
                || !text.is_char_boundary(span.range.start)
                || !text.is_char_boundary(span.range.end)
            {
                return Err(TokenSnapshotError::InvalidBoundary);
            }
            if span.range.start < previous_end {
                return Err(TokenSnapshotError::InvalidOrder);
            }
            previous_end = span.range.end;
        }
        Ok(Self { version, spans })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_detects_rust_and_go_without_claiming_unknown_files() {
        assert_eq!(
            LanguageRegistry::detect(Path::new("src/main.rs")).map(|item| item.id),
            Some(LanguageId::RUST)
        );
        assert_eq!(
            LanguageRegistry::detect(Path::new("cmd/server.go")).map(|item| item.id),
            Some(LanguageId::GO)
        );
        assert_eq!(
            LanguageRegistry::detect(Path::new("settings.json")).map(|item| item.id),
            Some(LanguageId::JSON)
        );
        assert!(LanguageRegistry::detect(Path::new("CMakeLists.txt")).is_none());
        assert_eq!(
            LanguageRegistry::by_id(LanguageId::GO)
                .and_then(|language| language.server)
                .expect("Go has a server descriptor")
                .executable_candidates,
            &["gopls"]
        );
    }

    #[test]
    fn request_stamps_reject_renamed_and_stale_responses() {
        let document = DocumentKey {
            id: DocumentId(17),
            uri: DocumentUri::new("file:///workspace/src/main.rs").unwrap(),
        };
        let current = RequestStamp {
            document: document.clone(),
            document_version: DocumentVersion(4),
            server_version: ServerDocumentVersion(6),
            session: SessionGeneration(2),
        };
        assert!(current.is_current_for(&current));

        let renamed = RequestStamp {
            document: DocumentKey {
                id: document.id,
                uri: DocumentUri::new("file:///workspace/src/lib.rs").unwrap(),
            },
            ..current.clone()
        };
        assert!(!renamed.is_current_for(&current));
        assert!(
            !RequestStamp {
                document_version: DocumentVersion(5),
                ..current.clone()
            }
            .is_current_for(&current)
        );
        assert!(
            !RequestStamp {
                server_version: ServerDocumentVersion(7),
                ..current.clone()
            }
            .is_current_for(&current)
        );
        assert!(
            !RequestStamp {
                session: SessionGeneration(3),
                ..current.clone()
            }
            .is_current_for(&current)
        );
    }

    #[test]
    fn document_uri_rejects_empty_and_control_values() {
        assert_eq!(DocumentUri::new(""), Err(DocumentUriError::Empty));
        assert_eq!(
            DocumentUri::new("file:///workspace/\nmain.rs"),
            Err(DocumentUriError::ContainsControlCharacter)
        );
    }

    #[test]
    fn semantic_tokens_decode_utf16_crlf_deltas_and_modifiers() {
        let text = "fn 🌍\r\nvalue";
        let legend = SemanticTokenLegend {
            token_types: vec!["keyword".into(), "variable".into()],
            token_modifiers: vec!["declaration".into()],
        };
        let tokens = decode_semantic_tokens(
            text,
            &legend,
            &[
                0, 0, 2, 0, 0, // `fn`
                1, 0, 5, 1, 1, // `value`, after CRLF and emoji on line zero
            ],
        )
        .unwrap();
        assert_eq!(
            tokens,
            vec![
                SemanticToken {
                    range: 0..2,
                    class: TokenClass::Keyword,
                    modifiers: 0,
                },
                SemanticToken {
                    range: text.find("value").unwrap()..text.len(),
                    class: TokenClass::Variable,
                    modifiers: 1,
                },
            ]
        );
    }

    #[test]
    fn semantic_tokens_reject_malformed_payloads_and_unknown_types() {
        let legend = SemanticTokenLegend::default();
        assert_eq!(
            decode_semantic_tokens("text", &legend, &[0, 0, 4]),
            Err(SemanticTokenError::InvalidDataLength)
        );
        assert_eq!(
            decode_semantic_tokens("text", &legend, &[0, 0, 4, 0, 0]),
            Err(SemanticTokenError::UnknownTokenType)
        );
    }

    #[test]
    fn positions_round_trip_unicode_and_crlf_without_splitting_surrogates() {
        let text = "a😀e\r\nβ";
        assert_eq!(
            position_to_byte(
                text,
                Position {
                    line: 0,
                    character: 3
                }
            ),
            Ok(5)
        );
        assert_eq!(
            position_to_byte(
                text,
                Position {
                    line: 0,
                    character: 2
                }
            ),
            Err(PositionError::SplitsUtf16CodePoint)
        );
        assert_eq!(
            byte_to_position(text, 6),
            Ok(Position {
                line: 0,
                character: 4
            })
        );
        assert_eq!(
            byte_to_position(text, 7),
            Ok(Position {
                line: 0,
                character: 4
            })
        );
        assert_eq!(
            byte_to_position(text, 8),
            Ok(Position {
                line: 1,
                character: 0
            })
        );
    }

    #[test]
    fn token_snapshots_require_ordered_utf8_boundaries() {
        let text = "fn 😀";
        let valid = TokenSnapshot::new(
            text,
            DocumentVersion(4),
            vec![TokenSpan {
                range: 0..2,
                class: TokenClass::Keyword,
            }],
        )
        .expect("valid token span");
        assert_eq!(valid.version, DocumentVersion(4));

        assert_eq!(
            TokenSnapshot::new(
                text,
                DocumentVersion(4),
                vec![TokenSpan {
                    range: 4..5,
                    class: TokenClass::String,
                }],
            ),
            Err(TokenSnapshotError::InvalidBoundary)
        );
        assert_eq!(
            TokenSnapshot::new(
                text,
                DocumentVersion(4),
                vec![
                    TokenSpan {
                        range: 1..2,
                        class: TokenClass::Keyword,
                    },
                    TokenSpan {
                        range: 0..1,
                        class: TokenClass::Keyword,
                    },
                ],
            ),
            Err(TokenSnapshotError::InvalidOrder)
        );
    }
}
