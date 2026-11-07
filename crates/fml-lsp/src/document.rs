//! An open document: its text, syntax tree, analysis and problems, and conversions between
//! LSP positions (UTF-16) and offsets.

use fml_check::{Analysis, Definitions};
use fml_syntax::{
    Diagnostic, LineIndex, Parse, Severity, SyntaxNode, SyntaxToken, TextRange, TextSize,
    TokenAtOffset, ast,
};
use lsp_types::{Position, Range};

pub(crate) struct Document {
    pub text: String,
    pub version: i32,
    pub parse: Parse,
    pub index: LineIndex,
    pub analysis: Analysis,
    /// Syntax, compile and type problems, ordered by position. Type problems are left out while
    /// the map has syntax errors: they follow from a tree that is not what was meant.
    pub diagnostics: Vec<Diagnostic>,
}

impl Document {
    pub fn new(text: String, version: i32, definitions: &dyn Definitions) -> Self {
        let parse = fml_syntax::parse(&text);
        let tree = parse.tree();
        let analysis = fml_check::analyze(&tree, definitions);
        let mut diagnostics = parse.diagnostics().to_vec();
        diagnostics.extend(fml_model::compile_tree(&tree).diagnostics);
        if !diagnostics.iter().any(|d| d.severity == Severity::Error) {
            diagnostics.extend(analysis.diagnostics.iter().cloned());
        }
        diagnostics.sort_by_key(|d| (d.range.start(), d.range.end()));
        diagnostics.dedup_by(|a, b| a.range == b.range && a.message == b.message);
        Self {
            index: LineIndex::new(&text),
            text,
            version,
            parse,
            analysis,
            diagnostics,
        }
    }

    pub fn tree(&self) -> ast::SourceFile {
        self.parse.tree()
    }

    pub fn syntax(&self) -> SyntaxNode {
        self.parse.syntax()
    }

    pub fn offset(&self, position: Position) -> TextSize {
        self.index
            .offset_utf16(&self.text, position.line, position.character)
    }

    pub fn position(&self, offset: TextSize) -> Position {
        let at = self.index.line_col(&self.text, offset);
        Position::new(at.line, at.col_utf16)
    }

    pub fn range(&self, range: TextRange) -> Range {
        Range::new(self.position(range.start()), self.position(range.end()))
    }

    /// The text from the start of the line to `offset`.
    pub fn line_before(&self, offset: TextSize) -> &str {
        let end = usize::from(offset);
        let start = self.text[..end].rfind(['\n', '\r']).map_or(0, |i| i + 1);
        &self.text[start..end]
    }

    /// The token the cursor at `offset` is in or right after: a name being typed ends at the
    /// cursor, so the token to the left wins over whitespace to the right.
    pub fn token_at(&self, offset: TextSize) -> Option<SyntaxToken> {
        match self.syntax().token_at_offset(offset) {
            TokenAtOffset::None => None,
            TokenAtOffset::Single(token) => Some(token),
            TokenAtOffset::Between(left, right) => {
                if left.kind().is_trivia() {
                    Some(right)
                } else {
                    Some(left)
                }
            }
        }
    }
}
