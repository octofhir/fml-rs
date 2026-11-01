//! Typed views over the syntax tree.
//!
//! Each type wraps a [`SyntaxNode`] of one kind. Accessors return `None` when the source is
//! incomplete, so they work on trees with errors too.

use crate::{SyntaxKind, SyntaxNode, SyntaxToken};

/// A typed view of a syntax node.
pub trait AstNode: Sized {
    fn can_cast(kind: SyntaxKind) -> bool;
    fn cast(node: SyntaxNode) -> Option<Self>;
    fn syntax(&self) -> &SyntaxNode;
}

macro_rules! ast_node {
    ($(#[$meta:meta])* $name:ident, $kind:ident) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq, Eq, Hash)]
        pub struct $name(SyntaxNode);

        impl AstNode for $name {
            fn can_cast(kind: SyntaxKind) -> bool {
                kind == SyntaxKind::$kind
            }

            fn cast(node: SyntaxNode) -> Option<Self> {
                Self::can_cast(node.kind()).then(|| Self(node))
            }

            fn syntax(&self) -> &SyntaxNode {
                &self.0
            }
        }
    };
}

fn child<N: AstNode>(node: &SyntaxNode) -> Option<N> {
    node.children().find_map(N::cast)
}

fn children<N: AstNode>(node: &SyntaxNode) -> impl Iterator<Item = N> + use<N> {
    node.children().filter_map(N::cast)
}

fn tokens(node: &SyntaxNode) -> impl Iterator<Item = SyntaxToken> + use<> {
    node.children_with_tokens()
        .filter_map(|element| element.into_token())
        .filter(|token| !token.kind().is_trivia())
}

/// The text of the first name token directly under `node` that is not one of `skip`.
fn name_after(node: &SyntaxNode, skip: &[&str]) -> Option<String> {
    tokens(node)
        .filter(|t| matches!(t.kind(), SyntaxKind::Ident | SyntaxKind::DelimitedIdent))
        .find(|t| !skip.contains(&t.text()))
        .map(|t| name_text(&t))
}

/// The name a token spells: the identifier, or the content of a `` `delimited` `` identifier.
pub fn name_text(token: &SyntaxToken) -> String {
    match token.kind() {
        SyntaxKind::DelimitedIdent => unescape(token.text()),
        _ => token.text().to_owned(),
    }
}

/// Removes the quotes from a string or delimited identifier and resolves escape sequences.
pub fn unescape(text: &str) -> String {
    // Markdown between triple quotes is taken verbatim, line breaks included.
    let inner = if text.starts_with("\"\"\"") && text.len() >= 6 {
        return text[3..text.len() - 3].to_owned();
    } else if text.len() >= 2 {
        &text[1..text.len() - 1]
    } else {
        text
    };
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('t') => out.push('\t'),
            Some('f') => out.push('\u{c}'),
            Some('u') => {
                let hex: String = chars.by_ref().take(4).collect();
                match u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
                    Some(decoded) => out.push(decoded),
                    None => {
                        out.push_str("\\u");
                        out.push_str(&hex);
                    }
                }
            }
            Some(other) => out.push(other),
            None => out.push('\\'),
        }
    }
    out
}

// ---- File level -------------------------------------------------------------------------------

ast_node!(
    /// A whole FML document.
    SourceFile,
    SourceFile
);

impl SourceFile {
    pub fn metadata(&self) -> impl Iterator<Item = Metadata> + use<> {
        children(&self.0)
    }

    pub fn concept_maps(&self) -> impl Iterator<Item = ConceptMap> + use<> {
        children(&self.0)
    }

    pub fn map_declaration(&self) -> Option<MapDeclaration> {
        child(&self.0)
    }

    pub fn uses(&self) -> impl Iterator<Item = UsesDeclaration> + use<> {
        children(&self.0)
    }

    pub fn imports(&self) -> impl Iterator<Item = ImportsDeclaration> + use<> {
        children(&self.0)
    }

    pub fn constants(&self) -> impl Iterator<Item = ConstantDeclaration> + use<> {
        children(&self.0)
    }

    pub fn groups(&self) -> impl Iterator<Item = Group> + use<> {
        children(&self.0)
    }
}

ast_node!(
    /// `/// key = value`
    Metadata,
    Metadata
);

impl Metadata {
    /// The key, such as `url` or `jurisdiction.coding.system`.
    pub fn key(&self) -> Option<QualifiedName> {
        child(&self.0)
    }

    /// The value. `None` when the key is given without one, which removes an inherited value.
    pub fn value(&self) -> Option<Literal> {
        child(&self.0)
    }
}

ast_node!(
    /// A literal token: string, number, boolean, date or time.
    Literal,
    Literal
);

impl Literal {
    pub fn token(&self) -> Option<SyntaxToken> {
        tokens(&self.0).next()
    }

    /// The value as written, with quotes removed and escapes resolved for strings.
    pub fn value(&self) -> Option<String> {
        let token = self.token()?;
        Some(match token.kind() {
            SyntaxKind::SingleQuotedString
            | SyntaxKind::DoubleQuotedString
            | SyntaxKind::TripleQuotedString
            | SyntaxKind::DelimitedIdent => unescape(token.text()),
            _ => token.text().to_owned(),
        })
    }

    pub fn kind(&self) -> Option<SyntaxKind> {
        self.token().map(|t| t.kind())
    }
}

ast_node!(
    /// `conceptmap "url" { … }`
    ConceptMap,
    ConceptMap
);

impl ConceptMap {
    pub fn url(&self) -> Option<Literal> {
        child(&self.0)
    }

    pub fn prefixes(&self) -> impl Iterator<Item = ConceptMapPrefix> + use<> {
        children(&self.0)
    }

    pub fn entries(&self) -> impl Iterator<Item = ConceptMapEntry> + use<> {
        children(&self.0)
    }

    pub fn unmapped(&self) -> impl Iterator<Item = ConceptMapUnmapped> + use<> {
        children(&self.0)
    }
}

ast_node!(
    /// `unmapped for s = provided`
    ConceptMapUnmapped,
    ConceptMapUnmapped
);

impl ConceptMapUnmapped {
    pub fn prefix(&self) -> Option<String> {
        name_after(&self.0, &["unmapped", "for"])
    }

    /// The unmapped mode, such as `provided`.
    pub fn mode(&self) -> Option<String> {
        tokens(&self.0)
            .filter(|t| t.kind() == SyntaxKind::Ident)
            .last()
            .map(|t| t.text().to_owned())
    }
}

ast_node!(
    /// `prefix s = "url"`
    ConceptMapPrefix,
    ConceptMapPrefix
);

impl ConceptMapPrefix {
    pub fn name(&self) -> Option<String> {
        name_after(&self.0, &["prefix"])
    }

    pub fn url(&self) -> Option<Literal> {
        child(&self.0)
    }
}

ast_node!(
    /// `s:male - t:M`
    ConceptMapEntry,
    ConceptMapEntry
);

impl ConceptMapEntry {
    pub fn source(&self) -> Option<ConceptMapCode> {
        children(&self.0).next()
    }

    pub fn target(&self) -> Option<ConceptMapCode> {
        children(&self.0).nth(1)
    }

    /// The relationship operator, such as `-`, `==` or `<=`.
    pub fn relation(&self) -> Option<String> {
        self.0
            .children()
            .find(|n| n.kind() == SyntaxKind::ConceptMapRelation)
            .map(|n| tokens(&n).map(|t| t.text().to_owned()).collect())
    }
}

ast_node!(
    /// `s:male`
    ConceptMapCode,
    ConceptMapCode
);

impl ConceptMapCode {
    pub fn prefix(&self) -> Option<String> {
        tokens(&self.0).next().map(|t| t.text().to_owned())
    }

    pub fn code(&self) -> Option<String> {
        let token = tokens(&self.0).nth(2)?;
        Some(match token.kind() {
            SyntaxKind::Ident | SyntaxKind::Integer | SyntaxKind::Decimal => {
                token.text().to_owned()
            }
            _ => unescape(token.text()),
        })
    }
}

ast_node!(
    /// `map "url" = "name"`
    MapDeclaration,
    MapDeclaration
);

impl MapDeclaration {
    pub fn url(&self) -> Option<Literal> {
        children(&self.0).next()
    }

    pub fn name(&self) -> Option<Literal> {
        children(&self.0).nth(1)
    }
}

/// How a map uses a structure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ModelMode {
    Source,
    Queried,
    Target,
    Produced,
}

impl ModelMode {
    pub fn as_str(self) -> &'static str {
        match self {
            ModelMode::Source => "source",
            ModelMode::Queried => "queried",
            ModelMode::Target => "target",
            ModelMode::Produced => "produced",
        }
    }
}

ast_node!(
    /// `uses "url" alias Name as source`
    UsesDeclaration,
    UsesDeclaration
);

impl UsesDeclaration {
    pub fn url(&self) -> Option<Literal> {
        child(&self.0)
    }

    pub fn alias(&self) -> Option<String> {
        let alias = self
            .0
            .children()
            .find(|n| n.kind() == SyntaxKind::UsesAlias)?;
        name_after(&alias, &["alias"])
    }

    pub fn mode(&self) -> Option<ModelMode> {
        let mut words = tokens(&self.0).map(|t| t.text().to_owned());
        words.find(|w| w == "as")?;
        match words.next()?.as_str() {
            "source" => Some(ModelMode::Source),
            "queried" => Some(ModelMode::Queried),
            "target" => Some(ModelMode::Target),
            "produced" => Some(ModelMode::Produced),
            _ => None,
        }
    }
}

ast_node!(
    /// `imports "url"`
    ImportsDeclaration,
    ImportsDeclaration
);

impl ImportsDeclaration {
    pub fn url(&self) -> Option<Literal> {
        child(&self.0)
    }
}

ast_node!(
    /// `let name = expression;`
    ConstantDeclaration,
    ConstantDeclaration
);

impl ConstantDeclaration {
    pub fn name(&self) -> Option<String> {
        name_after(&self.0, &["let"])
    }

    pub fn expression(&self) -> Option<Expr> {
        child(&self.0)
    }
}

// ---- Groups -----------------------------------------------------------------------------------

ast_node!(
    /// `group Name(…) extends Base <<types>> { … }`
    Group,
    Group
);

/// The type mode of a group: whether it is the default mapping for its types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GroupTypeMode {
    /// `<<types>>`
    Types,
    /// `<<type+>>`
    TypeAndTypes,
}

impl Group {
    pub fn name(&self) -> Option<String> {
        name_after(&self.0, &["group"])
    }

    pub fn parameters(&self) -> impl Iterator<Item = Parameter> + use<> {
        child::<ParameterList>(&self.0)
            .into_iter()
            .flat_map(|list| children::<Parameter>(&list.0).collect::<Vec<_>>())
    }

    pub fn extends(&self) -> Option<String> {
        let clause = self
            .0
            .children()
            .find(|n| n.kind() == SyntaxKind::ExtendsClause)?;
        name_after(&clause, &["extends"])
    }

    pub fn type_mode(&self) -> Option<GroupTypeMode> {
        let mode = self
            .0
            .children()
            .find(|n| n.kind() == SyntaxKind::TypeMode)?;
        let text: String = tokens(&mode).map(|t| t.text().to_owned()).collect();
        match text.as_str() {
            "<<types>>" => Some(GroupTypeMode::Types),
            "<<type+>>" => Some(GroupTypeMode::TypeAndTypes),
            _ => None,
        }
    }

    pub fn body(&self) -> Option<RuleBlock> {
        child(&self.0)
    }

    pub fn rules(&self) -> impl Iterator<Item = Rule> + use<> {
        self.body()
            .into_iter()
            .flat_map(|block| block.rules().collect::<Vec<_>>())
    }
}

ast_node!(ParameterList, ParameterList);

ast_node!(
    /// `source src : Type`
    Parameter,
    Parameter
);

/// Whether a group parameter is an input or an output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ParameterMode {
    Source,
    Target,
}

impl Parameter {
    pub fn mode(&self) -> Option<ParameterMode> {
        match tokens(&self.0).next()?.text() {
            "source" => Some(ParameterMode::Source),
            "target" => Some(ParameterMode::Target),
            _ => None,
        }
    }

    pub fn name(&self) -> Option<String> {
        tokens(&self.0)
            .nth(1)
            .filter(|t| matches!(t.kind(), SyntaxKind::Ident | SyntaxKind::DelimitedIdent))
            .map(|t| name_text(&t))
    }

    pub fn type_annotation(&self) -> Option<TypeAnnotation> {
        child(&self.0)
    }
}

ast_node!(
    /// `: Type`
    TypeAnnotation,
    TypeAnnotation
);

impl TypeAnnotation {
    /// The type name, unquoted when it was written as a string.
    pub fn type_name(&self) -> Option<String> {
        if let Some(name) = child::<QualifiedName>(&self.0) {
            return Some(name.text());
        }
        child::<Literal>(&self.0).and_then(|l| l.value())
    }
}

ast_node!(
    /// `{ rule; … }`
    RuleBlock,
    RuleBlock
);

impl RuleBlock {
    pub fn rules(&self) -> impl Iterator<Item = Rule> + use<> {
        children(&self.0)
    }
}

// ---- Rules ------------------------------------------------------------------------------------

ast_node!(
    /// `sources -> targets then … "name";`
    Rule,
    Rule
);

impl Rule {
    pub fn sources(&self) -> impl Iterator<Item = Source> + use<> {
        self.0
            .children()
            .find(|n| n.kind() == SyntaxKind::SourceList)
            .into_iter()
            .flat_map(|list| children::<Source>(&list).collect::<Vec<_>>())
    }

    pub fn targets(&self) -> impl Iterator<Item = Target> + use<> {
        self.0
            .children()
            .find(|n| n.kind() == SyntaxKind::TargetList)
            .into_iter()
            .flat_map(|list| children::<Target>(&list).collect::<Vec<_>>())
    }

    pub fn dependent(&self) -> Option<DependentClause> {
        child(&self.0)
    }

    /// The fields of the batch identity form `src.a -> tgt.a : f1, f2;`.
    pub fn identity_fields(&self) -> Vec<String> {
        self.0
            .children()
            .find(|n| n.kind() == SyntaxKind::IdentityList)
            .map(|list| {
                tokens(&list)
                    .filter(|t| matches!(t.kind(), SyntaxKind::Ident | SyntaxKind::DelimitedIdent))
                    .map(|t| name_text(&t))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The rule name, without quotes.
    pub fn name(&self) -> Option<String> {
        let name = self
            .0
            .children()
            .find(|n| n.kind() == SyntaxKind::RuleName)?;
        tokens(&name).next().map(|t| unescape(t.text()))
    }
}

/// A source or target list mode.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ListMode {
    First,
    NotFirst,
    Last,
    NotLast,
    OnlyOne,
    Single,
    Collate,
    /// `share id`
    Share(String),
}

impl ListMode {
    fn from_word(word: &str) -> Option<Self> {
        Some(match word {
            "first" => ListMode::First,
            "not_first" => ListMode::NotFirst,
            "last" => ListMode::Last,
            "not_last" => ListMode::NotLast,
            "only_one" => ListMode::OnlyOne,
            "single" => ListMode::Single,
            "collate" => ListMode::Collate,
            _ => return None,
        })
    }
}

ast_node!(
    /// One source of a rule.
    Source,
    Source
);

impl Source {
    /// The context variable and optional element, as in `src.name`.
    pub fn path(&self) -> Option<QualifiedName> {
        child(&self.0)
    }

    pub fn type_annotation(&self) -> Option<TypeAnnotation> {
        child(&self.0)
    }

    /// `(min, max)` where `max` is `*` or a number.
    pub fn cardinality(&self) -> Option<(String, String)> {
        let node = self
            .0
            .children()
            .find(|n| n.kind() == SyntaxKind::Cardinality)?;
        let mut parts = tokens(&node);
        let min = parts.next()?.text().to_owned();
        parts.next()?;
        let max = parts.next()?.text().to_owned();
        Some((min, max))
    }

    pub fn default_value(&self) -> Option<DefaultClause> {
        child(&self.0)
    }

    pub fn list_mode(&self) -> Option<ListMode> {
        let node = self
            .0
            .children()
            .find(|n| n.kind() == SyntaxKind::SourceListMode)?;
        ListMode::from_word(tokens(&node).next()?.text())
    }

    pub fn alias(&self) -> Option<String> {
        child::<Alias>(&self.0).and_then(|a| a.name())
    }

    pub fn where_clause(&self) -> Option<Expr> {
        self.clause(SyntaxKind::WhereClause)
    }

    pub fn check_clause(&self) -> Option<Expr> {
        self.clause(SyntaxKind::CheckClause)
    }

    pub fn log_clause(&self) -> Option<Expr> {
        self.clause(SyntaxKind::LogClause)
    }

    fn clause(&self, kind: SyntaxKind) -> Option<Expr> {
        let node = self.0.children().find(|n| n.kind() == kind)?;
        child(&node)
    }
}

ast_node!(
    /// `default (expression)`, or the legacy `default "value"`.
    DefaultClause,
    DefaultClause
);

impl DefaultClause {
    pub fn expression(&self) -> Option<Expr> {
        child(&self.0)
    }
}

ast_node!(
    /// `as name`
    Alias,
    Alias
);

impl Alias {
    pub fn name(&self) -> Option<String> {
        name_after(&self.0, &["as"])
    }
}

ast_node!(
    /// One target of a rule.
    Target,
    Target
);

impl Target {
    /// The context variable and element, as in `tgt.name`. `None` for targets that are only a
    /// transform (`create('X') as x`) or an expression.
    pub fn path(&self) -> Option<QualifiedName> {
        child(&self.0)
    }

    /// The transform: the value after `=`, or the target itself when it is an invocation or a
    /// parenthesised expression.
    pub fn transform(&self) -> Option<TransformValue> {
        if let Some(transform) = self
            .0
            .children()
            .find(|n| n.kind() == SyntaxKind::Transform)
        {
            return transform.children().find_map(TransformValue::cast);
        }
        self.0
            .children()
            .filter(|n| matches!(n.kind(), SyntaxKind::Invocation | SyntaxKind::ParenExpr))
            .find_map(TransformValue::cast)
    }

    pub fn alias(&self) -> Option<String> {
        child::<Alias>(&self.0).and_then(|a| a.name())
    }

    pub fn list_mode(&self) -> Option<ListMode> {
        let node = self
            .0
            .children()
            .find(|n| n.kind() == SyntaxKind::TargetListMode)?;
        let mut words = tokens(&node);
        let word = words.next()?;
        if word.text() == "share" {
            return Some(ListMode::Share(name_text(&words.next()?)));
        }
        ListMode::from_word(word.text())
    }
}

/// The value a target is set to.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TransformValue {
    /// A constant.
    Literal(Literal),
    /// A variable or a path: the copy transform.
    Name(QualifiedName),
    /// A named transform such as `create('Coding')`, or a group call.
    Invocation(Invocation),
    /// `(fhirpath)`: the evaluate transform.
    Expression(Expr),
}

impl TransformValue {
    fn cast(node: SyntaxNode) -> Option<Self> {
        Some(match node.kind() {
            SyntaxKind::Literal => TransformValue::Literal(Literal(node)),
            SyntaxKind::QualifiedName => TransformValue::Name(QualifiedName(node)),
            SyntaxKind::Invocation => TransformValue::Invocation(Invocation(node)),
            SyntaxKind::ParenExpr => TransformValue::Expression(Expr(node)),
            _ => return None,
        })
    }

    pub fn syntax(&self) -> &SyntaxNode {
        match self {
            TransformValue::Literal(n) => n.syntax(),
            TransformValue::Name(n) => n.syntax(),
            TransformValue::Invocation(n) => n.syntax(),
            TransformValue::Expression(n) => n.syntax(),
        }
    }
}

ast_node!(
    /// `name(arguments)`: a transform or a group call.
    Invocation,
    Invocation
);

impl Invocation {
    pub fn name(&self) -> Option<String> {
        tokens(&self.0).next().map(|t| name_text(&t))
    }

    pub fn arguments(&self) -> impl Iterator<Item = Expr> + use<> {
        child::<ArgumentList>(&self.0)
            .into_iter()
            .flat_map(|list| children::<Expr>(&list.0).collect::<Vec<_>>())
    }
}

ast_node!(ArgumentList, ArgumentList);

ast_node!(
    /// `then Group(a, b) { … }`
    DependentClause,
    DependentClause
);

impl DependentClause {
    pub fn invocations(&self) -> impl Iterator<Item = Invocation> + use<> {
        children(&self.0)
    }

    pub fn rules(&self) -> Option<RuleBlock> {
        child(&self.0)
    }
}

ast_node!(
    /// `a.b.c`
    QualifiedName,
    QualifiedName
);

impl QualifiedName {
    /// The segments, with delimited identifiers unquoted. A trailing `[x]` stays on its segment.
    pub fn segments(&self) -> Vec<String> {
        let mut segments: Vec<String> = Vec::new();
        let mut choice = false;
        for token in tokens(&self.0) {
            match token.kind() {
                SyntaxKind::Ident | SyntaxKind::DelimitedIdent if choice => {}
                SyntaxKind::Ident | SyntaxKind::DelimitedIdent => segments.push(name_text(&token)),
                SyntaxKind::LBracket => choice = true,
                SyntaxKind::RBracket => {
                    choice = false;
                    if let Some(last) = segments.last_mut() {
                        last.push_str("[x]");
                    }
                }
                _ => {}
            }
        }
        segments
    }

    /// The segments joined with `.`.
    pub fn text(&self) -> String {
        self.segments().join(".")
    }
}

// ---- Expressions ------------------------------------------------------------------------------

/// Any FHIRPath expression node.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Expr(SyntaxNode);

impl AstNode for Expr {
    fn can_cast(kind: SyntaxKind) -> bool {
        matches!(
            kind,
            SyntaxKind::Literal
                | SyntaxKind::Quantity
                | SyntaxKind::NameRef
                | SyntaxKind::ExternalConstant
                | SyntaxKind::ParenExpr
                | SyntaxKind::MemberExpr
                | SyntaxKind::CallExpr
                | SyntaxKind::IndexExpr
                | SyntaxKind::PrefixExpr
                | SyntaxKind::BinaryExpr
                | SyntaxKind::TypeExpr
                | SyntaxKind::NullLiteral
        )
    }

    fn cast(node: SyntaxNode) -> Option<Self> {
        Self::can_cast(node.kind()).then(|| Self(node))
    }

    fn syntax(&self) -> &SyntaxNode {
        &self.0
    }
}

impl Expr {
    /// The expression without the outer parentheses, if it has them.
    pub fn unparenthesized(&self) -> Expr {
        let mut expr = self.clone();
        while expr.0.kind() == SyntaxKind::ParenExpr {
            match child::<Expr>(&expr.0) {
                Some(inner) => expr = inner,
                None => break,
            }
        }
        expr
    }

    /// The expression in canonical form, as StructureMap stores it; see
    /// [`expression_text`](crate::expression_text).
    pub fn canonical_text(&self) -> String {
        crate::expression_text(&self.0)
    }

    /// The expression's source text, comments and whitespace kept as written.
    pub fn text(&self) -> String {
        self.0.text().to_string().trim().to_owned()
    }
}
