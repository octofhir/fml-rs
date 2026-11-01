//! Lossless syntax tree, error-tolerant parser and formatter for the
//! [FHIR Mapping Language](https://build.fhir.org/ig/HL7/mapping-language-ig/).
//!
//! ```
//! let parse = fml_syntax::parse(r#"
//! uses "http://hl7.org/fhir/StructureDefinition/Patient" as source
//! uses "http://hl7.org/fhir/StructureDefinition/Patient" as target
//!
//! group Copy(source src : Patient, target tgt : Patient) {
//!   src.name -> tgt.name;
//! }
//! "#);
//! assert!(parse.diagnostics().is_empty());
//!
//! let file = parse.tree();
//! let group = file.groups().next().unwrap();
//! assert_eq!(group.name().as_deref(), Some("Copy"));
//! ```
//!
//! The tree keeps every byte of the input, comments and whitespace included, so
//! `parse(text).syntax().to_string() == text` for any input.

pub mod ast;
mod diagnostic;
mod format;
mod kind;
mod lexer;
mod parser;

use rowan::GreenNode;

pub use diagnostic::{Diagnostic, LineCol, LineIndex, Severity};
pub use format::{
    FormatError, FormatOptions, expression_text, format, format_tree, quote_fhirpath_string,
};
pub use kind::{FmlLanguage, SyntaxElement, SyntaxKind, SyntaxNode, SyntaxToken};
pub use lexer::{Token, tokenize};
pub use rowan::{Direction, NodeOrToken, TextRange, TextSize};

/// The result of parsing: a syntax tree that covers the whole input, plus the problems found.
#[derive(Debug, Clone)]
pub struct Parse {
    green: GreenNode,
    diagnostics: Vec<Diagnostic>,
}

impl Parse {
    /// The root of the untyped syntax tree.
    pub fn syntax(&self) -> SyntaxNode {
        SyntaxNode::new_root(self.green.clone())
    }

    /// The typed root of the tree.
    pub fn tree(&self) -> ast::SourceFile {
        ast::SourceFile::cast(self.syntax()).expect("the root is always a source file")
    }

    /// Lexical and syntax errors, ordered by position.
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    pub fn has_errors(&self) -> bool {
        self.diagnostics
            .iter()
            .any(|d| d.severity == Severity::Error)
    }
}

/// Parses FML source text. Never fails; see [`Parse::diagnostics`].
pub fn parse(text: &str) -> Parse {
    let (green, diagnostics) = parser::parse_source(text);
    Parse { green, diagnostics }
}

pub use ast::AstNode;
