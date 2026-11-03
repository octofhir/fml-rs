//! Formatter: canonical layout for FML, keeping every comment.
//!
//! Declarations and rules go one per line; rule blocks and concept maps are indented. Spacing
//! inside a statement follows the style of the Mapping Language IG examples (`source src : Type`,
//! `a -> b`, `where (x)`, `create('X')`). Blank lines between statements are kept, collapsed to
//! one, and sections of the file are separated by one blank line.
//!
//! A statement that has a comment in its middle is written as it was, so no comment ever moves
//! away from the code it describes. Output is re-parsed and compared token by token with the
//! input; the formatter returns an error instead of output that would change the map.

use std::fmt;

use rowan::NodeOrToken;

use crate::SyntaxKind::{self, *};
use crate::ast::{AstNode, SourceFile, unescape};
use crate::{Diagnostic, SyntaxNode, SyntaxToken, parse};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormatOptions {
    /// Spaces per indentation level.
    pub indent_width: usize,
    /// Indent with tabs instead of spaces.
    pub use_tabs: bool,
}

impl Default for FormatOptions {
    fn default() -> Self {
        Self {
            indent_width: 2,
            use_tabs: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FormatError {
    /// The source has syntax errors; it is not formatted so nothing is lost.
    Syntax(Vec<Diagnostic>),
    /// The formatted text would not parse back to the same map. This is a formatter bug.
    Unstable(String),
}

impl fmt::Display for FormatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FormatError::Syntax(diagnostics) => {
                write!(f, "cannot format: {} syntax error(s)", diagnostics.len())
            }
            FormatError::Unstable(reason) => {
                write!(f, "formatter produced different code: {reason}")
            }
        }
    }
}

impl std::error::Error for FormatError {}

/// Formats FML source text.
pub fn format(text: &str, options: &FormatOptions) -> Result<String, FormatError> {
    let parse = parse(text);
    if parse.has_errors() {
        return Err(FormatError::Syntax(parse.diagnostics().to_vec()));
    }
    let mut output = format_tree(&parse.tree(), options);
    if uses_crlf(text) {
        output = output.replace("\r\n", "\n").replace('\n', "\r\n");
    }
    verify(&parse.syntax(), &output)?;
    Ok(output)
}

/// Whether the text's first line ends with `\r\n`; the formatter keeps that line ending.
fn uses_crlf(text: &str) -> bool {
    text.find('\n')
        .is_some_and(|index| text[..index].ends_with('\r'))
}

/// Formats a parsed tree without checking it first. Prefer [`format()`], which refuses trees with
/// syntax errors and verifies the result.
pub fn format_tree(file: &SourceFile, options: &FormatOptions) -> String {
    let mut formatter = Formatter {
        options,
        out: String::new(),
        depth: 0,
        last: None,
    };
    formatter.items(file.syntax(), Container::File);
    let mut out = formatter.out;
    let trimmed = out.trim_end().len();
    out.truncate(trimmed);
    if !out.is_empty() {
        out.push('\n');
    }
    out
}

/// Checks that `output` holds the same tokens and comments as `original`.
fn verify(original: &SyntaxNode, output: &str) -> Result<(), FormatError> {
    let reparsed = parse(output);
    if reparsed.has_errors() {
        return Err(FormatError::Unstable("output has syntax errors".into()));
    }
    let tokens = |node: &SyntaxNode| -> Vec<(SyntaxKind, String)> {
        node.descendants_with_tokens()
            .filter_map(NodeOrToken::into_token)
            .filter(|t| t.kind() != Whitespace)
            .map(|t| (t.kind(), t.text().trim_end().to_owned()))
            .collect()
    };
    let before = tokens(original);
    let after = tokens(&reparsed.syntax());
    if before.len() != after.len() {
        return Err(FormatError::Unstable(format!(
            "{} tokens became {}",
            before.len(),
            after.len()
        )));
    }
    if let Some((a, b)) = before.iter().zip(&after).find(|(a, b)| a != b) {
        return Err(FormatError::Unstable(format!("`{}` became `{}`", a.1, b.1)));
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Container {
    File,
    Block,
}

/// Where a statement belongs in the file, for blank lines between sections.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Section {
    Metadata,
    ConceptMap,
    Map,
    Uses,
    Imports,
    Constants,
    Group,
    Other,
}

impl Section {
    fn of(kind: SyntaxKind) -> Section {
        match kind {
            Metadata => Section::Metadata,
            ConceptMap => Section::ConceptMap,
            MapDeclaration => Section::Map,
            UsesDeclaration => Section::Uses,
            ImportsDeclaration => Section::Imports,
            ConstantDeclaration => Section::Constants,
            Group => Section::Group,
            _ => Section::Other,
        }
    }

    /// Two statements of this section are separated by a blank line even if written together.
    fn spaced(self) -> bool {
        matches!(self, Section::Group | Section::ConceptMap)
    }
}

enum Item {
    Node(SyntaxNode),
    Comment(SyntaxToken),
}

struct Entry {
    item: Item,
    /// Line breaks in the source between the previous element and this one.
    newlines: usize,
}

struct Formatter<'a> {
    options: &'a FormatOptions,
    out: String,
    depth: usize,
    /// The last token written on the current line, for spacing decisions.
    last: Option<SyntaxToken>,
}

impl Formatter<'_> {
    fn newline(&mut self, blank: bool) {
        let trimmed = self.out.trim_end_matches([' ', '\t']).len();
        self.out.truncate(trimmed);
        if self.out.is_empty() {
            return;
        }
        self.out.push('\n');
        if blank {
            self.out.push('\n');
        }
        for _ in 0..self.depth {
            if self.options.use_tabs {
                self.out.push('\t');
            } else {
                self.out
                    .extend(std::iter::repeat_n(' ', self.options.indent_width));
            }
        }
        self.last = None;
    }

    /// Writes the statements and comments that are direct children of `node`: the whole file, or
    /// the part of a block between its braces.
    fn items(&mut self, node: &SyntaxNode, container: Container) {
        let entries = collect_entries(node, container);
        let sections: Vec<Option<Section>> = (0..entries.len())
            .map(|i| {
                entries[i..].iter().find_map(|e| match &e.item {
                    Item::Node(n) => Some(Section::of(n.kind())),
                    Item::Comment(_) => None,
                })
            })
            .collect();
        let mut previous: Option<(Section, bool)> = None;
        for (index, entry) in entries.iter().enumerate() {
            let first = index == 0;
            let same_line = entry.newlines == 0 && (!first || container == Container::Block);
            if let (Item::Comment(comment), true) = (&entry.item, same_line) {
                self.out.push(' ');
                self.out.push_str(comment.text());
                self.last = None;
                continue;
            }
            let mut blank = entry.newlines >= 2 && !first;
            if container == Container::File
                && let (Some((previous_section, previous_was_node)), Some(section)) =
                    (previous, sections[index])
                && (previous_section != section || (section.spaced() && previous_was_node))
            {
                blank = true;
            }
            self.newline(blank);
            match &entry.item {
                Item::Comment(comment) => {
                    self.out.push_str(comment.text().trim_end());
                    self.last = None;
                    if let Some(section) = sections[index] {
                        previous = Some((section, false));
                    }
                }
                Item::Node(node) => {
                    self.node(node);
                    previous = Some((Section::of(node.kind()), true));
                }
            }
        }
    }

    fn node(&mut self, node: &SyntaxNode) {
        if node.kind() == Error || has_loose_comments(node) {
            return self.verbatim(node);
        }
        if matches!(node.kind(), RuleBlock | ConceptMap) {
            return self.block(node);
        }
        for element in node.children_with_tokens() {
            match element {
                NodeOrToken::Node(child) => self.node(&child),
                NodeOrToken::Token(token) if !token.kind().is_trivia() => self.token(&token),
                NodeOrToken::Token(_) => {}
            }
        }
    }

    /// A node with `{ … }`: tokens before the brace, then one item per line, indented.
    fn block(&mut self, node: &SyntaxNode) {
        let mut inside = false;
        for element in node.children_with_tokens() {
            match element {
                NodeOrToken::Token(token) if token.kind() == LBrace && !inside => {
                    inside = true;
                    self.token(&token);
                    let empty = !node
                        .children_with_tokens()
                        .skip_while(|e| e.kind() != LBrace)
                        .skip(1)
                        .any(|e| e.kind() != RBrace && e.kind() != Whitespace);
                    if empty {
                        continue;
                    }
                    self.depth += 1;
                    self.items(node, Container::Block);
                    self.depth -= 1;
                    self.newline(false);
                }
                NodeOrToken::Token(token) if token.kind() == RBrace => {
                    self.out.push('}');
                    self.last = Some(token);
                    inside = false;
                }
                _ if inside => {}
                NodeOrToken::Node(child) => self.node(&child),
                NodeOrToken::Token(token) if !token.kind().is_trivia() => self.token(&token),
                NodeOrToken::Token(_) => {}
            }
        }
    }

    fn token(&mut self, token: &SyntaxToken) {
        if let Some(last) = &self.last
            && space_between(last, token)
        {
            self.out.push(' ');
        }
        self.out.push_str(token.text());
        self.last = Some(token.clone());
    }

    /// Writes a node exactly as it appears in the source.
    fn verbatim(&mut self, node: &SyntaxNode) {
        let first = node
            .descendants_with_tokens()
            .filter_map(NodeOrToken::into_token)
            .find(|t| !t.kind().is_trivia());
        if let (Some(last), Some(first)) = (&self.last, &first)
            && space_between(last, first)
        {
            self.out.push(' ');
        }
        self.out.push_str(node.text().to_string().trim());
        self.last = node
            .descendants_with_tokens()
            .filter_map(NodeOrToken::into_token)
            .filter(|t| !t.kind().is_trivia())
            .last();
    }
}

fn collect_entries(node: &SyntaxNode, container: Container) -> Vec<Entry> {
    let mut entries = Vec::new();
    let mut newlines = 0;
    let mut inside = container == Container::File;
    for element in node.children_with_tokens() {
        match element {
            NodeOrToken::Token(token) if token.kind() == LBrace && !inside => inside = true,
            NodeOrToken::Token(token) if token.kind() == RBrace && inside => break,
            _ if !inside => {}
            NodeOrToken::Token(token) if token.kind() == Whitespace => {
                newlines += line_breaks(token.text());
            }
            NodeOrToken::Token(token) if token.kind().is_comment() => entries.push(Entry {
                item: Item::Comment(token),
                newlines: std::mem::take(&mut newlines),
            }),
            NodeOrToken::Token(_) => {}
            NodeOrToken::Node(child) => entries.push(Entry {
                item: Item::Node(child),
                newlines: std::mem::take(&mut newlines),
            }),
        }
    }
    entries
}

fn line_breaks(text: &str) -> usize {
    text.matches('\n').count() + text.matches('\r').count() - text.matches("\r\n").count()
}

/// Whether `node` has a comment that the item layout does not place: one inside a statement
/// rather than between the statements of a block. Nested blocks place their own comments, so
/// their contents are not searched.
fn has_loose_comments(node: &SyntaxNode) -> bool {
    let is_block = matches!(node.kind(), RuleBlock | ConceptMap);
    let mut after_brace = false;
    for element in node.children_with_tokens() {
        match element {
            NodeOrToken::Token(token) => {
                after_brace |= token.kind() == LBrace;
                if token.kind().is_comment() && !(is_block && after_brace) {
                    return true;
                }
            }
            NodeOrToken::Node(child) => {
                // Statements inside a block are laid out, and checked, one by one.
                if !(is_block && after_brace) && contains_comment_outside_blocks(&child) {
                    return true;
                }
            }
        }
    }
    false
}

fn contains_comment_outside_blocks(node: &SyntaxNode) -> bool {
    if matches!(node.kind(), RuleBlock | ConceptMap) {
        return false;
    }
    node.children_with_tokens().any(|element| match element {
        NodeOrToken::Token(token) => token.kind().is_comment(),
        NodeOrToken::Node(child) => contains_comment_outside_blocks(&child),
    })
}

/// Renders a FHIRPath expression in canonical form: one space around binary operators, none
/// inside paths and calls, comments dropped, strings single-quoted. This is the form StructureMap
/// stores expressions in, and the one the HL7 reference compiler produces.
pub fn expression_text(node: &SyntaxNode) -> String {
    let mut out = String::new();
    let mut last: Option<SyntaxToken> = None;
    for token in node
        .descendants_with_tokens()
        .filter_map(NodeOrToken::into_token)
        .filter(|t| !t.kind().is_trivia())
    {
        if let Some(previous) = &last
            && space_between(previous, &token)
        {
            out.push(' ');
        }
        if token.kind() == DoubleQuotedString {
            out.push_str(&quote_fhirpath_string(&unescape(token.text())));
        } else {
            out.push_str(token.text());
        }
        last = Some(token);
    }
    out
}

/// Writes `value` as a single-quoted FHIRPath string literal.
pub fn quote_fhirpath_string(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('\'');
    for c in value.chars() {
        match c {
            '\'' => out.push_str("\\'"),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{c}' => out.push_str("\\f"),
            c => out.push(c),
        }
    }
    out.push('\'');
    out
}

fn space_between(previous: &SyntaxToken, next: &SyntaxToken) -> bool {
    let previous_parent = previous.parent().map(|p| p.kind());
    let next_parent = next.parent().map(|p| p.kind());
    let same_parent = |kind: SyntaxKind| previous_parent == Some(kind) && next_parent == Some(kind);
    match (previous.kind(), next.kind()) {
        (_, Comma | Semicolon | RParen | RBracket | Dot | DotDot) => false,
        (LParen | LBracket | Dot | DotDot | Percent, _) => false,
        (_, LBracket) => false,
        (Plus | Minus, _) if previous_parent == Some(PrefixExpr) => false,
        (LBrace, RBrace) => false,
        (_, Colon) if next_parent == Some(IdentityList) => false,
        (_, LParen) => !matches!(next_parent, Some(ArgumentList | ParameterList)),
        _ if same_parent(TypeMode) || same_parent(ConceptMapCode) => false,
        _ if same_parent(ConceptMapRelation) => false,
        _ if same_parent(QualifiedName) => false,
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fmt(text: &str) -> String {
        format(text, &FormatOptions::default()).unwrap_or_else(|e| panic!("{e}: {e:?}"))
    }

    #[test]
    fn normalizes_layout() {
        let source = r#"/// url='http://x/Map'
uses "http://hl7.org/fhir/StructureDefinition/Patient" alias P as source
uses "http://hl7.org/fhir/StructureDefinition/Patient" as target
group G(source src:P,target tgt) extends Base <<type+>> {
src.name as n where (n.use='official')->tgt.name as t then {
n.family->t.family;
}"named";
src.a->tgt.b=create('Coding') as c,c.code=(src.x+1);
}
group H(source s, target t){}
"#;
        insta::assert_snapshot!(fmt(source), @r#"
        /// url = 'http://x/Map'

        uses "http://hl7.org/fhir/StructureDefinition/Patient" alias P as source
        uses "http://hl7.org/fhir/StructureDefinition/Patient" as target

        group G(source src : P, target tgt) extends Base <<type+>> {
          src.name as n where (n.use = 'official') -> tgt.name as t then {
            n.family -> t.family;
          } "named";
          src.a -> tgt.b = create('Coding') as c, c.code = (src.x + 1);
        }

        group H(source s, target t) {}
        "#);
    }

    #[test]
    fn keeps_comments() {
        let source = "// header\n\nuses \"u\" as source // why\n\n// group doc\ngroup G(source s, target t) { // opens\n  // first\n  s.a -> t.a; // trailing\n\n\n  s.b /* inline */ -> t.b;\n  // last\n}\n";
        insta::assert_snapshot!(fmt(source), @r#"
        // header

        uses "u" as source // why

        // group doc
        group G(source s, target t) { // opens
          // first
          s.a -> t.a; // trailing

          s.b /* inline */ -> t.b;
          // last
        }
        "#);
    }

    #[test]
    fn idempotent() {
        let source =
            "group G(source s, target t) {\n  s.a as v -> t.a = v, t.b = 'x' share k \"r\";\n}\n";
        let once = fmt(source);
        assert_eq!(fmt(&once), once);
    }

    #[test]
    fn keeps_crlf_line_endings() {
        let source =
            "uses \"u\" as source\r\n\r\n\r\ngroup G(source s, target t) {\r\n  s.a->t.a;\r\n}\r\n";
        assert_eq!(
            fmt(source),
            "uses \"u\" as source\r\n\r\ngroup G(source s, target t) {\r\n  s.a -> t.a;\r\n}\r\n"
        );
    }

    #[test]
    fn refuses_broken_input() {
        assert!(matches!(
            format("group G(source s { s.a -> ", &FormatOptions::default()),
            Err(FormatError::Syntax(_))
        ));
    }
}
