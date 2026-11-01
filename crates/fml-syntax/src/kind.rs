//! Token and node kinds of the FML syntax tree.

/// Every token and node kind in an FML syntax tree.
///
/// FML keywords are contextual: `source`, `group`, `where` and the rest are also valid element
/// names. The lexer therefore produces [`SyntaxKind::Ident`] for every word and the parser decides
/// what a word means from where it appears.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(u16)]
pub enum SyntaxKind {
    // Trivia.
    Whitespace = 0,
    LineComment,
    BlockComment,

    // Literals and names.
    Ident,
    /// `` `name` ``
    DelimitedIdent,
    /// `'text'`
    SingleQuotedString,
    /// `"text"`
    DoubleQuotedString,
    /// `"""` … `"""`, used for markdown metadata values.
    TripleQuotedString,
    Integer,
    Decimal,
    /// `10L`
    LongNumber,
    /// `@2024-01-01`
    Date,
    /// `@2024-01-01T10:00:00Z`
    DateTime,
    /// `@T10:00`
    Time,
    /// `$this`, `$index`, `$total`
    DollarIdent,

    // Punctuation.
    /// `///` at the start of a metadata line.
    MetadataPrefix,
    LParen,
    RParen,
    LBrace,
    RBrace,
    LBracket,
    RBracket,
    Comma,
    Semicolon,
    Colon,
    Dot,
    /// `..`
    DotDot,
    /// `->`
    Arrow,
    /// `=`
    Eq,
    /// `==`
    EqEq,
    /// `!=`
    Neq,
    /// `~`
    Tilde,
    /// `!~`
    NotTilde,
    Lt,
    Le,
    Gt,
    Ge,
    /// `<<`
    Shl,
    /// `>>`
    Shr,
    Plus,
    Minus,
    Star,
    Slash,
    Amp,
    Pipe,
    Percent,
    /// A character the lexer does not recognise, or an unterminated literal.
    Unknown,

    // Nodes: map structure.
    SourceFile,
    /// `/// key = value`
    Metadata,
    /// `conceptmap "url" { … }`
    ConceptMap,
    /// `prefix s = "url"`
    ConceptMapPrefix,
    /// `unmapped for s = provided`
    ConceptMapUnmapped,
    /// `s:code - t:code`
    ConceptMapEntry,
    /// `s:code`
    ConceptMapCode,
    /// The relationship operator between two concept map codes.
    ConceptMapRelation,
    /// `map "url" = "name"`
    MapDeclaration,
    /// `uses "url" alias Name as source`
    UsesDeclaration,
    /// `alias Name`
    UsesAlias,
    /// `imports "url"`
    ImportsDeclaration,
    /// `let name = expression;`
    ConstantDeclaration,
    /// `group Name(…) extends Base <<types>> { … }`
    Group,
    /// `(source src : Type, target tgt : Type)`
    ParameterList,
    /// `source src : Type`
    Parameter,
    /// `: Type`
    TypeAnnotation,
    /// `extends Base`
    ExtendsClause,
    /// `<<types>>` or `<<type+>>`
    TypeMode,
    /// `{ rule; rule; }`
    RuleBlock,
    Rule,
    /// The sources of a rule, separated by commas.
    SourceList,
    Source,
    /// `0..*`
    Cardinality,
    /// `default (expression)`
    DefaultClause,
    /// `first`, `not_first`, `last`, `not_last`, `only_one`
    SourceListMode,
    /// `as name`
    Alias,
    /// `where (expression)`
    WhereClause,
    /// `check (expression)`
    CheckClause,
    /// `log (expression)`
    LogClause,
    /// The targets of a rule, separated by commas.
    TargetList,
    Target,
    /// `= transform` on a target.
    Transform,
    /// `first`, `last`, `single`, `share id`, `collate`
    TargetListMode,
    /// `name(arguments)`, a transform or a group invocation.
    Invocation,
    /// `(argument, argument)`
    ArgumentList,
    /// `then Group(a, b) { … }`
    DependentClause,
    /// `: field, field`, in the batch identity rule form.
    IdentityList,
    /// `"name"` after a rule.
    RuleName,
    /// `a.b.c`
    QualifiedName,

    // Nodes: FHIRPath expressions.
    /// A literal value.
    Literal,
    /// `5 'mg'`, `3 days`
    Quantity,
    /// `name` in an expression.
    NameRef,
    /// `%name`
    ExternalConstant,
    /// `(expression)`
    ParenExpr,
    /// `expression.member` or `expression.function(…)`
    MemberExpr,
    /// `function(arguments)`
    CallExpr,
    /// `expression[index]`
    IndexExpr,
    /// `-expression`
    PrefixExpr,
    /// `left op right`
    BinaryExpr,
    /// `expression is Type`, `expression as Type`
    TypeExpr,
    /// `{}`
    NullLiteral,

    /// Tokens the parser could not place.
    Error,
}

impl SyntaxKind {
    /// Whitespace and comments.
    pub fn is_trivia(self) -> bool {
        matches!(
            self,
            SyntaxKind::Whitespace | SyntaxKind::LineComment | SyntaxKind::BlockComment
        )
    }

    pub fn is_comment(self) -> bool {
        matches!(self, SyntaxKind::LineComment | SyntaxKind::BlockComment)
    }

    fn from_raw(raw: u16) -> SyntaxKind {
        assert!(raw <= SyntaxKind::Error as u16, "invalid syntax kind {raw}");
        // SAFETY: `SyntaxKind` is `repr(u16)` with contiguous discriminants from 0 to `Error`.
        unsafe { std::mem::transmute::<u16, SyntaxKind>(raw) }
    }
}

impl From<SyntaxKind> for rowan::SyntaxKind {
    fn from(kind: SyntaxKind) -> Self {
        rowan::SyntaxKind(kind as u16)
    }
}

/// The rowan language tag for FML trees.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum FmlLanguage {}

impl rowan::Language for FmlLanguage {
    type Kind = SyntaxKind;

    fn kind_from_raw(raw: rowan::SyntaxKind) -> SyntaxKind {
        SyntaxKind::from_raw(raw.0)
    }

    fn kind_to_raw(kind: SyntaxKind) -> rowan::SyntaxKind {
        kind.into()
    }
}

pub type SyntaxNode = rowan::SyntaxNode<FmlLanguage>;
pub type SyntaxToken = rowan::SyntaxToken<FmlLanguage>;
pub type SyntaxElement = rowan::SyntaxElement<FmlLanguage>;
