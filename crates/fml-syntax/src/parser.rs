//! Error-tolerant recursive-descent parser producing a lossless rowan tree.
//!
//! Trivia (whitespace and comments) is attached lazily: it is emitted right before the next
//! significant token or the next node that starts. Trivia between two statements therefore sits in
//! the block that contains them, never inside the statements, which is what the formatter relies
//! on to keep comments in place.

use rowan::{Checkpoint, GreenNode, GreenNodeBuilder, TextRange};

use crate::SyntaxKind::{self, *};
use crate::diagnostic::Diagnostic;
use crate::lexer::{Token, tokenize};

/// Parses an FML document. Never fails: problems are reported as diagnostics and the offending
/// tokens end up in [`SyntaxKind::Error`] nodes, so the tree always covers the whole input.
pub(crate) fn parse_source(source: &str) -> (GreenNode, Vec<Diagnostic>) {
    let (tokens, mut diagnostics) = tokenize(source);
    let mut parser = Parser::new(source, tokens);
    parser.source_file();
    diagnostics.append(&mut parser.diagnostics);
    diagnostics.sort_by_key(|d| (d.range.start(), d.range.end()));
    (parser.builder.finish(), diagnostics)
}

/// Words that start a top-level declaration.
const DECLARATION_KEYWORDS: &[&str] = &["conceptmap", "map", "uses", "imports", "let", "group"];
const SOURCE_LIST_MODES: &[&str] = &["first", "not_first", "last", "not_last", "only_one"];
const TARGET_LIST_MODES: &[&str] = &["first", "last", "single", "collate"];
const CALENDAR_UNITS: &[&str] = &[
    "year",
    "month",
    "week",
    "day",
    "hour",
    "minute",
    "second",
    "millisecond",
    "years",
    "months",
    "weeks",
    "days",
    "hours",
    "minutes",
    "seconds",
    "milliseconds",
];

struct Parser<'a> {
    source: &'a str,
    tokens: Vec<Token>,
    /// Indices into `tokens` of the significant (non-trivia) tokens.
    significant: Vec<usize>,
    /// Index into `significant` of the current token.
    cursor: usize,
    /// Index into `tokens` of the first token not yet added to the tree.
    emitted: usize,
    builder: GreenNodeBuilder<'static>,
    diagnostics: Vec<Diagnostic>,
}

impl<'a> Parser<'a> {
    fn new(source: &'a str, tokens: Vec<Token>) -> Self {
        let significant = tokens
            .iter()
            .enumerate()
            .filter(|(_, t)| !t.kind.is_trivia())
            .map(|(i, _)| i)
            .collect();
        Self {
            source,
            tokens,
            significant,
            cursor: 0,
            emitted: 0,
            builder: GreenNodeBuilder::new(),
            diagnostics: Vec::new(),
        }
    }

    // ---- Token access -------------------------------------------------------------------------

    fn nth_token(&self, n: usize) -> Option<&Token> {
        self.significant
            .get(self.cursor + n)
            .map(|&index| &self.tokens[index])
    }

    fn nth(&self, n: usize) -> Option<SyntaxKind> {
        self.nth_token(n).map(|t| t.kind)
    }

    fn current(&self) -> Option<SyntaxKind> {
        self.nth(0)
    }

    fn nth_text(&self, n: usize) -> &'a str {
        self.nth_token(n).map_or("", |t| &self.source[t.range])
    }

    fn at(&self, kind: SyntaxKind) -> bool {
        self.current() == Some(kind)
    }

    fn at_eof(&self) -> bool {
        self.current().is_none()
    }

    fn nth_is_kw(&self, n: usize, keyword: &str) -> bool {
        self.nth(n) == Some(Ident) && self.nth_text(n) == keyword
    }

    fn at_kw(&self, keyword: &str) -> bool {
        self.nth_is_kw(0, keyword)
    }

    fn at_any_kw(&self, keywords: &[&str]) -> bool {
        self.current() == Some(Ident) && keywords.contains(&self.nth_text(0))
    }

    /// Whether a line break separates the current token from the previous significant one. The
    /// start of the file counts as a line break.
    fn at_line_start(&self) -> bool {
        let end = self
            .significant
            .get(self.cursor)
            .copied()
            .unwrap_or(self.tokens.len());
        let start = match self.cursor {
            0 => return true,
            n => self.significant[n - 1] + 1,
        };
        self.tokens[start..end]
            .iter()
            .any(|t| t.kind == Whitespace && self.source[t.range].contains(['\n', '\r']))
    }

    fn current_range(&self) -> TextRange {
        match self.nth_token(0) {
            Some(t) => t.range,
            None => {
                let end = rowan::TextSize::from(self.source.len() as u32);
                TextRange::empty(end)
            }
        }
    }

    // ---- Tree building ------------------------------------------------------------------------

    fn flush_trivia(&mut self) {
        let end = self
            .significant
            .get(self.cursor)
            .copied()
            .unwrap_or(self.tokens.len());
        while self.emitted < end {
            let token = self.tokens[self.emitted];
            self.builder
                .token(token.kind.into(), &self.source[token.range]);
            self.emitted += 1;
        }
    }

    fn bump(&mut self) {
        let Some(&index) = self.significant.get(self.cursor) else {
            return;
        };
        self.flush_trivia();
        let token = self.tokens[index];
        self.builder
            .token(token.kind.into(), &self.source[token.range]);
        self.emitted = index + 1;
        self.cursor += 1;
    }

    fn start(&mut self, kind: SyntaxKind) {
        self.flush_trivia();
        self.builder.start_node(kind.into());
    }

    fn finish(&mut self) {
        self.builder.finish_node();
    }

    fn checkpoint(&mut self) -> Checkpoint {
        self.flush_trivia();
        self.builder.checkpoint()
    }

    fn start_at(&mut self, checkpoint: Checkpoint, kind: SyntaxKind) {
        self.builder.start_node_at(checkpoint, kind.into());
    }

    fn error(&mut self, message: impl Into<String>) {
        let range = self.current_range();
        self.diagnostics.push(Diagnostic::error(range, message));
    }

    fn bump_error(&mut self) {
        self.start(Error);
        self.bump();
        self.finish();
    }

    /// Consumes `kind` or reports what was expected without consuming anything.
    fn expect(&mut self, kind: SyntaxKind, what: &str) -> bool {
        if self.at(kind) {
            self.bump();
            true
        } else {
            self.error_expected(what);
            false
        }
    }

    /// Reports a missing token. When the current token starts a new line, the problem is the end
    /// of the previous line (a missing `;`, an unfinished rule), so that is where it is reported.
    fn error_expected(&mut self, what: &str) {
        let found = match self.nth_token(0) {
            Some(_) => format!("`{}`", self.nth_text(0)),
            None => "end of file".to_owned(),
        };
        let range = match self.cursor.checked_sub(1) {
            Some(previous) if self.at_line_start() || self.at_eof() => {
                TextRange::empty(self.tokens[self.significant[previous]].range.end())
            }
            _ => self.current_range(),
        };
        self.diagnostics.push(Diagnostic::error(
            range,
            format!("expected {what}, found {found}"),
        ));
    }

    // ---- Declarations -------------------------------------------------------------------------

    fn source_file(&mut self) {
        self.builder.start_node(SourceFile.into());
        while !self.at_eof() {
            self.declaration();
        }
        self.flush_trivia();
        self.finish();
    }

    fn at_declaration_start(&self) -> bool {
        self.at(MetadataPrefix) || (self.at_any_kw(DECLARATION_KEYWORDS) && self.at_line_start())
    }

    fn declaration(&mut self) {
        if self.at(MetadataPrefix) {
            return self.metadata();
        }
        match self.nth_text(0) {
            "conceptmap" if self.at(Ident) => self.concept_map(),
            "map" if self.at(Ident) => self.map_declaration(),
            "uses" if self.at(Ident) => self.uses_declaration(),
            "imports" if self.at(Ident) => self.imports_declaration(),
            "let" if self.at(Ident) => self.constant_declaration(),
            "group" if self.at(Ident) => self.group(),
            _ => {
                self.error_expected("a declaration (`uses`, `imports`, `group`, …)");
                self.start(Error);
                self.bump();
                while !self.at_eof() && !self.at_declaration_start() {
                    self.bump();
                }
                self.finish();
            }
        }
    }

    /// `/// key = value`. The value is optional, and must sit on the same line as the key.
    fn metadata(&mut self) {
        self.start(Metadata);
        self.bump();
        self.qualified_name();
        self.expect(Eq, "`=`");
        let value_on_line = !self.at_eof() && !self.at_line_start();
        if value_on_line {
            if is_literal_token(self.current(), self.nth_text(0)) {
                self.start(Literal);
                self.bump();
                self.finish();
            }
            if !self.at_eof() && !self.at_line_start() {
                self.error("metadata value must be a single literal on the same line");
                self.start(Error);
                while !self.at_eof() && !self.at_line_start() {
                    self.bump();
                }
                self.finish();
            }
        }
        self.finish();
    }

    fn url(&mut self) {
        if self.at(SingleQuotedString) || self.at(DoubleQuotedString) {
            self.start(Literal);
            self.bump();
            self.finish();
        } else {
            self.error_expected("a quoted URL");
        }
    }

    /// `conceptmap "url" { prefix s = "url" … s:code - t:code … }`
    fn concept_map(&mut self) {
        self.start(ConceptMap);
        self.bump();
        self.url();
        if self.expect(LBrace, "`{`") {
            while !self.at_eof() && !self.at(RBrace) {
                if self.at_kw("prefix") {
                    self.start(ConceptMapPrefix);
                    self.bump();
                    self.name_token("a prefix name");
                    self.expect(Eq, "`=`");
                    self.url();
                    self.finish();
                } else if self.at_kw("unmapped") {
                    self.concept_map_unmapped();
                } else if self.at(Ident) && self.nth(1) == Some(Colon) {
                    self.concept_map_entry();
                } else {
                    self.error_expected("`prefix` or a concept mapping");
                    self.bump_error();
                }
            }
            self.expect(RBrace, "`}`");
        }
        self.finish();
    }

    /// `unmapped for s = provided`
    fn concept_map_unmapped(&mut self) {
        self.start(ConceptMapUnmapped);
        self.bump();
        if self.at_kw("for") {
            self.bump();
        } else {
            self.error_expected("`for`");
        }
        self.name_token("a prefix name");
        self.expect(Eq, "`=`");
        self.name_token("an unmapped mode such as `provided`");
        self.finish();
    }

    fn concept_map_entry(&mut self) {
        self.start(ConceptMapEntry);
        self.concept_map_code();
        self.start(ConceptMapRelation);
        let mut operators = 0;
        while matches!(
            self.current(),
            Some(Minus | Eq | EqEq | Neq | Lt | Le | Gt | Ge | Tilde | NotTilde)
        ) {
            self.bump();
            operators += 1;
        }
        self.finish();
        if operators == 0 {
            self.error_expected("a relationship such as `-` or `==`");
        }
        if self.at(Ident) && self.nth(1) == Some(Colon) && !self.at_line_start() {
            self.concept_map_code();
        }
        self.finish();
    }

    fn concept_map_code(&mut self) {
        self.start(ConceptMapCode);
        self.bump();
        self.bump();
        if matches!(
            self.current(),
            Some(
                Ident
                    | SingleQuotedString
                    | DoubleQuotedString
                    | Integer
                    | Decimal
                    | DelimitedIdent
            )
        ) {
            self.bump();
        } else {
            self.error_expected("a code");
        }
        self.finish();
    }

    /// `map "url" = "name"`
    fn map_declaration(&mut self) {
        self.start(MapDeclaration);
        self.bump();
        self.url();
        self.expect(Eq, "`=`");
        if matches!(
            self.current(),
            Some(SingleQuotedString | DoubleQuotedString | Ident | DelimitedIdent)
        ) {
            self.start(Literal);
            self.bump();
            self.finish();
        } else {
            self.error_expected("the map name");
        }
        self.finish();
    }

    /// `uses "url" alias Name as source`
    fn uses_declaration(&mut self) {
        self.start(UsesDeclaration);
        self.bump();
        self.url();
        loop {
            if self.at_kw("alias") {
                self.start(UsesAlias);
                self.bump();
                self.name_token("an alias");
                self.finish();
            } else if self.at_kw("as") {
                self.bump();
                if self.at_any_kw(&["source", "queried", "target", "produced"]) {
                    self.bump();
                } else {
                    self.error_expected("`source`, `queried`, `target` or `produced`");
                }
            } else {
                break;
            }
        }
        self.optional_semicolon();
        self.finish();
    }

    /// The reference compiler accepts a `;` after `uses`, `imports` and a group's `}`.
    fn optional_semicolon(&mut self) {
        if self.at(Semicolon) {
            self.bump();
        }
    }

    /// `imports "url"`
    fn imports_declaration(&mut self) {
        self.start(ImportsDeclaration);
        self.bump();
        self.url();
        self.optional_semicolon();
        self.finish();
    }

    /// `let name = expression;`
    fn constant_declaration(&mut self) {
        self.start(ConstantDeclaration);
        self.bump();
        self.name_token("a constant name");
        self.expect(Eq, "`=`");
        self.expression(true);
        self.expect(Semicolon, "`;`");
        self.finish();
    }

    /// `group Name(source src : Type, target tgt : Type) extends Base <<type+>> { … }`
    fn group(&mut self) {
        self.start(Group);
        self.bump();
        self.name_token("a group name");
        if self.at(LParen) {
            self.parameter_list();
        } else {
            self.error_expected("`(` and the group parameters");
        }
        loop {
            if self.at_kw("extends") {
                self.start(ExtendsClause);
                self.bump();
                self.name_token("the extended group");
                self.finish();
            } else if self.at(Shl) {
                self.start(TypeMode);
                self.bump();
                while !self.at_eof() && !self.at(Shr) && !self.at(LBrace) {
                    self.bump();
                }
                self.expect(Shr, "`>>`");
                self.finish();
            } else {
                break;
            }
        }
        if self.at(LBrace) {
            self.rule_block();
            self.optional_semicolon();
        } else {
            self.error_expected("`{` and the group rules");
        }
        self.finish();
    }

    fn parameter_list(&mut self) {
        self.start(ParameterList);
        self.bump();
        while !self.at_eof() && !self.at(RParen) {
            self.parameter();
            if !self.at(Comma) {
                break;
            }
            self.bump();
        }
        self.expect(RParen, "`)`");
        self.finish();
    }

    /// `source src : Type`
    fn parameter(&mut self) {
        self.start(Parameter);
        if self.at_any_kw(&["source", "target"]) {
            self.bump();
        } else {
            self.error_expected("`source` or `target`");
        }
        self.name_token("a parameter name");
        if self.at(Colon) {
            self.type_annotation();
        }
        self.finish();
    }

    fn type_annotation(&mut self) {
        self.start(TypeAnnotation);
        self.bump();
        if self.at(SingleQuotedString) || self.at(DoubleQuotedString) {
            self.start(Literal);
            self.bump();
            self.finish();
        } else {
            self.qualified_name();
        }
        self.finish();
    }

    // ---- Rules --------------------------------------------------------------------------------

    fn rule_block(&mut self) {
        self.start(RuleBlock);
        self.bump();
        while !self.at_eof() && !self.at(RBrace) {
            if self.at_kw("group") && self.at_line_start() && self.nth(1) == Some(Ident) {
                // A missing `}`: let the enclosing file parse the next group.
                break;
            }
            self.rule();
        }
        self.expect(RBrace, "`}`");
        self.finish();
    }

    fn rule(&mut self) {
        let before = self.cursor;
        self.start(Rule);
        self.source_list();
        if self.at(Arrow) {
            self.bump();
            self.target_list();
            if self.at(Colon) {
                self.identity_list();
            }
        }
        if self.at_kw("then") {
            self.dependent_clause();
        }
        if self.at(DoubleQuotedString) {
            self.start(RuleName);
            self.bump();
            self.finish();
        }
        if !self.at(Semicolon) {
            self.error_expected("`;`");
            self.recover_rule(before);
        }
        if self.at(Semicolon) {
            self.bump();
        }
        self.finish();
    }

    /// Skips to the end of a broken rule: up to `;`, a `}`, or the next line (a missing `;`).
    fn recover_rule(&mut self, rule_start: usize) {
        let made_progress = self.cursor > rule_start;
        if self.at_eof() || self.at(RBrace) || (made_progress && self.at_line_start()) {
            return;
        }
        self.start(Error);
        self.bump();
        while !self.at_eof() && !self.at(Semicolon) && !self.at(RBrace) && !self.at_line_start() {
            self.bump();
        }
        self.finish();
    }

    fn source_list(&mut self) {
        self.start(SourceList);
        self.source();
        while self.at(Comma) {
            self.bump();
            self.source();
        }
        self.finish();
    }

    /// `src.path : Type 0..1 default (x) first as v where (c) check (c) log (x)`
    fn source(&mut self) {
        self.start(Source);
        self.qualified_name();
        loop {
            if self.at(Colon) {
                self.type_annotation();
            } else if self.at(Integer) && self.nth(1) == Some(DotDot) {
                self.start(Cardinality);
                self.bump();
                self.bump();
                if self.at(Integer) || self.at(Star) {
                    self.bump();
                } else {
                    self.error_expected("an upper bound");
                }
                self.finish();
            } else if self.at_kw("default") {
                self.start(DefaultClause);
                self.bump();
                if self.at(LParen) {
                    self.paren_expr();
                } else if is_literal_token(self.current(), self.nth_text(0)) {
                    self.start(Literal);
                    self.bump();
                    self.finish();
                } else {
                    self.expression(false);
                }
                self.finish();
            } else if self.at_any_kw(SOURCE_LIST_MODES) {
                self.start(SourceListMode);
                self.bump();
                self.finish();
            } else if self.at_kw("as") {
                self.alias();
            } else if self.at_kw("where") {
                self.clause(WhereClause);
            } else if self.at_kw("check") {
                self.clause(CheckClause);
            } else if self.at_kw("log") {
                self.clause(LogClause);
            } else {
                break;
            }
        }
        self.finish();
    }

    /// `where (expression)`. The parentheses are optional in maps written for FHIR R4.
    fn clause(&mut self, kind: SyntaxKind) {
        self.start(kind);
        self.bump();
        self.expression(false);
        self.finish();
    }

    fn alias(&mut self) {
        self.start(Alias);
        self.bump();
        self.name_token("a variable name");
        self.finish();
    }

    fn target_list(&mut self) {
        self.start(TargetList);
        self.target();
        while self.at(Comma) {
            self.bump();
            self.target();
        }
        self.finish();
    }

    fn target(&mut self) {
        self.start(Target);
        if self.at(LParen) {
            self.paren_expr();
        } else if is_name_token(self.current()) && self.nth(1) == Some(LParen) {
            self.invocation();
        } else {
            self.qualified_name();
            if self.at(Eq) {
                self.start(Transform);
                self.bump();
                self.transform_value();
                self.finish();
            }
        }
        if self.at_kw("as") {
            self.alias();
        }
        if self.at_any_kw(TARGET_LIST_MODES) {
            self.start(TargetListMode);
            self.bump();
            self.finish();
        } else if self.at_kw("share") {
            self.start(TargetListMode);
            self.bump();
            self.name_token("a list rule id");
            self.finish();
        }
        self.finish();
    }

    /// The value after `=` in a target: a literal, a variable or path, an invocation such as
    /// `create('Coding')`, or a parenthesised FHIRPath expression.
    fn transform_value(&mut self) {
        if self.at(LParen) {
            self.paren_expr();
        } else if is_name_token(self.current()) && self.nth(1) == Some(LParen) {
            self.invocation();
        } else if is_literal_token(self.current(), self.nth_text(0)) {
            self.start(Literal);
            self.bump();
            self.finish();
        } else if is_name_token(self.current()) {
            self.qualified_name();
        } else {
            self.error_expected("a transform");
        }
    }

    /// `name(argument, …)`
    fn invocation(&mut self) {
        self.start(Invocation);
        self.bump();
        self.argument_list();
        self.finish();
    }

    fn argument_list(&mut self) {
        self.start(ArgumentList);
        self.bump();
        while !self.at_eof() && !self.at(RParen) {
            self.expression(true);
            if self.at_any_kw(&["asc", "desc"]) {
                self.bump();
            }
            if !self.at(Comma) {
                break;
            }
            self.bump();
        }
        self.expect(RParen, "`)`");
        self.finish();
    }

    /// `then Group(a, b), Other(c) { … }` or `then { … }`
    fn dependent_clause(&mut self) {
        self.start(DependentClause);
        self.bump();
        if is_name_token(self.current()) && self.nth(1) == Some(LParen) {
            self.invocation();
            while self.at(Comma) {
                self.bump();
                if is_name_token(self.current()) && self.nth(1) == Some(LParen) {
                    self.invocation();
                } else {
                    self.error_expected("a group invocation");
                    break;
                }
            }
        } else if !self.at(LBrace) {
            self.error_expected("a group invocation or `{`");
        }
        if self.at(LBrace) {
            self.rule_block();
        }
        self.finish();
    }

    /// `: field, field`
    fn identity_list(&mut self) {
        self.start(IdentityList);
        self.bump();
        self.name_token("a field name");
        while self.at(Comma) {
            self.bump();
            self.name_token("a field name");
        }
        self.finish();
    }

    fn name_token(&mut self, what: &str) {
        if is_name_token(self.current()) {
            self.bump();
        } else {
            self.error_expected(what);
        }
    }

    /// `a.b.c`, allowing reserved words and `` `quoted` `` segments, and a trailing `[x]`.
    fn qualified_name(&mut self) {
        self.start(QualifiedName);
        self.name_token("a name");
        loop {
            if self.at(Dot) && is_name_token(self.nth(1)) {
                self.bump();
                self.bump();
            } else if self.at(LBracket) && self.nth_is_kw(1, "x") && self.nth(2) == Some(RBracket) {
                self.bump();
                self.bump();
                self.bump();
            } else {
                break;
            }
        }
        self.finish();
    }

    // ---- FHIRPath -----------------------------------------------------------------------------

    fn paren_expr(&mut self) {
        self.start(ParenExpr);
        self.bump();
        self.expression(true);
        self.expect(RParen, "`)`");
        self.finish();
    }

    /// Parses a FHIRPath expression. Outside parentheses (`allow_as == false`), `as` ends the
    /// expression because it introduces a rule variable instead.
    fn expression(&mut self, allow_as: bool) {
        self.expression_bp(0, allow_as);
    }

    fn expression_bp(&mut self, min_bp: u8, allow_as: bool) {
        let checkpoint = self.checkpoint();
        if self.at(Plus) || self.at(Minus) {
            self.start(PrefixExpr);
            self.bump();
            self.expression_bp(PREFIX_BP, allow_as);
            self.finish();
        } else {
            self.term();
        }
        loop {
            if self.at(Dot) && self.nth(1).is_some_and(is_member_token) {
                self.start_at(checkpoint, MemberExpr);
                self.bump();
                self.member();
                self.finish();
                continue;
            }
            if self.at(LBracket) {
                self.start_at(checkpoint, IndexExpr);
                self.bump();
                self.expression(true);
                self.expect(RBracket, "`]`");
                self.finish();
                continue;
            }
            let Some((bp, kind)) = self.infix_operator(allow_as) else {
                break;
            };
            if bp < min_bp {
                break;
            }
            self.start_at(checkpoint, kind);
            self.bump();
            if kind == TypeExpr && self.at(SingleQuotedString) {
                // Some published maps quote the type (`$this is 'Reference'`).
                let range = self.current_range();
                self.diagnostics.push(Diagnostic::warning(
                    range,
                    "a type name is an identifier and should not be quoted",
                ));
                self.start(Literal);
                self.bump();
                self.finish();
            } else if kind == TypeExpr {
                self.qualified_name();
            } else {
                self.expression_bp(bp + 1, allow_as);
            }
            self.finish();
        }
    }

    fn infix_operator(&self, allow_as: bool) -> Option<(u8, SyntaxKind)> {
        let binary = |bp| Some((bp, BinaryExpr));
        match self.current()? {
            Star | Slash => binary(10),
            Plus | Minus | Amp => binary(9),
            Pipe => binary(7),
            Lt | Le | Gt | Ge => binary(6),
            Eq | Tilde | Neq | NotTilde => binary(5),
            Ident => match self.nth_text(0) {
                "div" | "mod" => binary(10),
                "is" => Some((8, TypeExpr)),
                "as" if allow_as => Some((8, TypeExpr)),
                "in" | "contains" => binary(4),
                "and" => binary(3),
                "or" | "xor" => binary(2),
                "implies" => binary(1),
                _ => None,
            },
            _ => None,
        }
    }

    fn member(&mut self) {
        if self.at(Ident) && self.nth(1) == Some(LParen) {
            self.call();
        } else {
            self.start(NameRef);
            self.bump();
            self.finish();
        }
    }

    fn call(&mut self) {
        self.start(CallExpr);
        self.bump();
        self.argument_list();
        self.finish();
    }

    fn term(&mut self) {
        match self.current() {
            Some(LParen) => self.paren_expr(),
            Some(LBrace) if self.nth(1) == Some(RBrace) => {
                self.start(NullLiteral);
                self.bump();
                self.bump();
                self.finish();
            }
            Some(Percent) => {
                self.start(ExternalConstant);
                self.bump();
                if is_name_token(self.current()) || self.at(SingleQuotedString) {
                    self.bump();
                } else {
                    self.error_expected("a constant name");
                }
                self.finish();
            }
            Some(Integer | Decimal) if self.quantity_unit_follows() => {
                self.start(Quantity);
                self.bump();
                self.bump();
                self.finish();
            }
            Some(Ident) if self.nth(1) == Some(LParen) => self.call(),
            kind if is_literal_token(kind, self.nth_text(0)) => {
                self.start(Literal);
                self.bump();
                self.finish();
            }
            Some(Ident | DelimitedIdent | DollarIdent) => {
                self.start(NameRef);
                self.bump();
                self.finish();
            }
            Some(RParen | Comma | Semicolon | RBrace | RBracket) | None => {
                self.error_expected("an expression");
            }
            Some(_) => {
                self.error_expected("an expression");
                self.bump_error();
            }
        }
    }

    fn quantity_unit_follows(&self) -> bool {
        match self.nth(1) {
            Some(SingleQuotedString) => true,
            Some(Ident) => CALENDAR_UNITS.contains(&self.nth_text(1)),
            _ => false,
        }
    }
}

const PREFIX_BP: u8 = 11;

fn is_name_token(kind: Option<SyntaxKind>) -> bool {
    matches!(kind, Some(Ident | DelimitedIdent))
}

fn is_member_token(kind: SyntaxKind) -> bool {
    matches!(kind, Ident | DelimitedIdent | DollarIdent)
}

fn is_literal_token(kind: Option<SyntaxKind>, text: &str) -> bool {
    match kind {
        Some(
            SingleQuotedString | DoubleQuotedString | TripleQuotedString | Integer | Decimal
            | LongNumber | Date | DateTime | Time,
        ) => true,
        Some(Ident) => matches!(text, "true" | "false"),
        _ => false,
    }
}
