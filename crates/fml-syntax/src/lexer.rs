//! Converts FML source text into tokens, trivia included.

use rowan::{TextRange, TextSize};

use crate::SyntaxKind;
use crate::diagnostic::Diagnostic;

/// A token: its kind and where it is in the source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Token {
    pub kind: SyntaxKind,
    pub range: TextRange,
}

/// Splits `source` into tokens. Every byte of the source belongs to exactly one token, so the
/// concatenated token texts always equal the source. Malformed input produces
/// [`SyntaxKind::Unknown`] tokens and a diagnostic for each.
pub fn tokenize(source: &str) -> (Vec<Token>, Vec<Diagnostic>) {
    let mut lexer = Lexer {
        source,
        pos: 0,
        tokens: Vec::new(),
        diagnostics: Vec::new(),
    };
    while lexer.pos < source.len() {
        lexer.next_token();
    }
    (lexer.tokens, lexer.diagnostics)
}

struct Lexer<'a> {
    source: &'a str,
    pos: usize,
    tokens: Vec<Token>,
    diagnostics: Vec<Diagnostic>,
}

impl Lexer<'_> {
    fn rest(&self) -> &str {
        &self.source[self.pos..]
    }

    fn peek(&self) -> Option<char> {
        self.rest().chars().next()
    }

    fn peek_nth(&self, n: usize) -> Option<char> {
        self.rest().chars().nth(n)
    }

    fn next_token(&mut self) {
        let start = self.pos;
        let c = self.peek().expect("lexer called at end of input");
        let kind = match c {
            c if is_whitespace(c) => {
                self.eat_while(is_whitespace);
                SyntaxKind::Whitespace
            }
            '/' => self.slash(),
            '\'' => self.quoted('\'', SyntaxKind::SingleQuotedString, "string"),
            '`' => self.quoted('`', SyntaxKind::DelimitedIdent, "delimited identifier"),
            '"' if self.rest().starts_with("\"\"\"") => self.triple_quoted(),
            '"' => self.quoted('"', SyntaxKind::DoubleQuotedString, "string"),
            '0'..='9' => self.number(),
            '@' => self.date_time(),
            '$' if self.peek_nth(1).is_some_and(is_ident_start) => {
                self.bump();
                self.eat_while(is_ident_continue);
                SyntaxKind::DollarIdent
            }
            c if is_ident_start(c) => {
                self.eat_while(is_ident_continue);
                SyntaxKind::Ident
            }
            _ => self.punctuation(c),
        };
        self.push(kind, start);
    }

    fn push(&mut self, kind: SyntaxKind, start: usize) {
        self.tokens.push(Token {
            kind,
            range: range(start, self.pos),
        });
    }

    fn error(&mut self, start: usize, message: impl Into<String>) {
        self.diagnostics
            .push(Diagnostic::error(range(start, self.pos), message));
    }

    fn bump(&mut self) {
        if let Some(c) = self.peek() {
            self.pos += c.len_utf8();
        }
    }

    fn bump_n(&mut self, n: usize) {
        for _ in 0..n {
            self.bump();
        }
    }

    fn eat_while(&mut self, predicate: impl Fn(char) -> bool) {
        while self.peek().is_some_and(&predicate) {
            self.bump();
        }
    }

    fn eat_digits(&mut self) -> usize {
        let start = self.pos;
        self.eat_while(|c| c.is_ascii_digit());
        self.pos - start
    }

    fn slash(&mut self) -> SyntaxKind {
        let rest = self.rest();
        if rest.starts_with("///") && !rest.starts_with("////") && is_metadata_line(&rest[3..]) {
            self.bump_n(3);
            SyntaxKind::MetadataPrefix
        } else if rest.starts_with("//") {
            self.eat_while(|c| c != '\n' && c != '\r');
            SyntaxKind::LineComment
        } else if let Some(body) = rest.strip_prefix("/*") {
            let start = self.pos;
            match body.find("*/") {
                Some(end) => {
                    self.pos += 2 + end + 2;
                    SyntaxKind::BlockComment
                }
                None => {
                    self.pos = self.source.len();
                    self.error(start, "unterminated block comment");
                    SyntaxKind::BlockComment
                }
            }
        } else {
            self.bump();
            SyntaxKind::Slash
        }
    }

    fn quoted(&mut self, quote: char, kind: SyntaxKind, what: &str) -> SyntaxKind {
        let start = self.pos;
        self.bump();
        loop {
            match self.peek() {
                None => {
                    self.error(start, format!("unterminated {what}"));
                    return SyntaxKind::Unknown;
                }
                Some('\\') => {
                    self.bump();
                    self.bump();
                }
                Some(c) if c == quote => {
                    self.bump();
                    return kind;
                }
                Some(_) => self.bump(),
            }
        }
    }

    fn triple_quoted(&mut self) -> SyntaxKind {
        let start = self.pos;
        match self.rest()[3..].find("\"\"\"") {
            Some(end) => {
                self.pos += 3 + end + 3;
                SyntaxKind::TripleQuotedString
            }
            None => {
                self.pos = self.source.len();
                self.error(start, "unterminated triple-quoted string");
                SyntaxKind::Unknown
            }
        }
    }

    fn number(&mut self) -> SyntaxKind {
        self.eat_digits();
        if self.peek() == Some('.') && self.peek_nth(1).is_some_and(|c| c.is_ascii_digit()) {
            self.bump();
            self.eat_digits();
            return SyntaxKind::Decimal;
        }
        if self.peek() == Some('L') && !self.peek_nth(1).is_some_and(is_ident_continue) {
            self.bump();
            return SyntaxKind::LongNumber;
        }
        SyntaxKind::Integer
    }

    /// `@YYYY[-MM[-DD]][T[hh[:mm[:ss[.fff]]]][tz]]` or `@Thh[:mm[:ss[.fff]]]`.
    fn date_time(&mut self) -> SyntaxKind {
        let start = self.pos;
        self.bump();
        if self.peek() == Some('T') {
            self.bump();
            self.time();
            return SyntaxKind::Time;
        }
        if self.eat_digits() != 4 {
            self.error(start, "malformed date literal");
            return SyntaxKind::Unknown;
        }
        for _ in 0..2 {
            if self.peek() == Some('-') && self.peek_nth(1).is_some_and(|c| c.is_ascii_digit()) {
                self.bump();
                self.eat_digits();
            }
        }
        if self.peek() != Some('T') {
            return SyntaxKind::Date;
        }
        self.bump();
        if self.peek().is_some_and(|c| c.is_ascii_digit()) {
            self.time();
            match self.peek() {
                Some('Z') => self.bump(),
                Some('+' | '-') if self.peek_nth(1).is_some_and(|c| c.is_ascii_digit()) => {
                    self.bump();
                    self.eat_digits();
                    if self.peek() == Some(':') {
                        self.bump();
                        self.eat_digits();
                    }
                }
                _ => {}
            }
        }
        SyntaxKind::DateTime
    }

    fn time(&mut self) {
        self.eat_digits();
        for _ in 0..2 {
            if self.peek() == Some(':') && self.peek_nth(1).is_some_and(|c| c.is_ascii_digit()) {
                self.bump();
                self.eat_digits();
            }
        }
        if self.peek() == Some('.') && self.peek_nth(1).is_some_and(|c| c.is_ascii_digit()) {
            self.bump();
            self.eat_digits();
        }
    }

    fn punctuation(&mut self, c: char) -> SyntaxKind {
        let next = self.peek_nth(1);
        let (kind, len) = match (c, next) {
            ('-', Some('>')) => (SyntaxKind::Arrow, 2),
            ('.', Some('.')) => (SyntaxKind::DotDot, 2),
            ('=', Some('=')) => (SyntaxKind::EqEq, 2),
            ('!', Some('=')) => (SyntaxKind::Neq, 2),
            ('!', Some('~')) => (SyntaxKind::NotTilde, 2),
            ('<', Some('<')) => (SyntaxKind::Shl, 2),
            ('<', Some('=')) => (SyntaxKind::Le, 2),
            ('>', Some('>')) => (SyntaxKind::Shr, 2),
            ('>', Some('=')) => (SyntaxKind::Ge, 2),
            ('(', _) => (SyntaxKind::LParen, 1),
            (')', _) => (SyntaxKind::RParen, 1),
            ('{', _) => (SyntaxKind::LBrace, 1),
            ('}', _) => (SyntaxKind::RBrace, 1),
            ('[', _) => (SyntaxKind::LBracket, 1),
            (']', _) => (SyntaxKind::RBracket, 1),
            (',', _) => (SyntaxKind::Comma, 1),
            (';', _) => (SyntaxKind::Semicolon, 1),
            (':', _) => (SyntaxKind::Colon, 1),
            ('.', _) => (SyntaxKind::Dot, 1),
            ('=', _) => (SyntaxKind::Eq, 1),
            ('~', _) => (SyntaxKind::Tilde, 1),
            ('<', _) => (SyntaxKind::Lt, 1),
            ('>', _) => (SyntaxKind::Gt, 1),
            ('+', _) => (SyntaxKind::Plus, 1),
            ('-', _) => (SyntaxKind::Minus, 1),
            ('*', _) => (SyntaxKind::Star, 1),
            ('&', _) => (SyntaxKind::Amp, 1),
            ('|', _) => (SyntaxKind::Pipe, 1),
            ('%', _) => (SyntaxKind::Percent, 1),
            _ => (SyntaxKind::Unknown, 1),
        };
        let start = self.pos;
        self.bump_n(len);
        if kind == SyntaxKind::Unknown {
            self.error(start, format!("unexpected character {c:?}"));
        }
        kind
    }
}

/// Whether the text after `///` reads `key =` on the same line, which makes it metadata rather
/// than a comment that starts with three slashes.
fn is_metadata_line(after: &str) -> bool {
    let line = after.split(['\n', '\r']).next().unwrap_or("");
    let line = line.trim_start_matches([' ', '\t']);
    let key_len = line
        .find(|c: char| !(is_ident_continue(c) || c == '.'))
        .unwrap_or(line.len());
    key_len > 0
        && line.starts_with(is_ident_start)
        && line[key_len..]
            .trim_start_matches([' ', '\t'])
            .starts_with('=')
}

fn is_whitespace(c: char) -> bool {
    c.is_whitespace() || c == '\u{feff}'
}

fn is_ident_start(c: char) -> bool {
    c.is_alphabetic() || c == '_'
}

fn is_ident_continue(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn range(start: usize, end: usize) -> TextRange {
    TextRange::new(TextSize::from(start as u32), TextSize::from(end as u32))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(source: &str) -> Vec<(SyntaxKind, &str)> {
        let (tokens, diagnostics) = tokenize(source);
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        tokens
            .into_iter()
            .filter(|t| t.kind != SyntaxKind::Whitespace)
            .map(|t| (t.kind, &source[t.range]))
            .collect()
    }

    #[test]
    fn lossless() {
        let source =
            "/// url = 'x'\ngroup G(source s, target t) { s.a as v -> t.b = v \"r\"; } // c\n";
        let (tokens, _) = tokenize(source);
        let text: String = tokens.iter().map(|t| &source[t.range]).collect();
        assert_eq!(text, source);
    }

    #[test]
    fn metadata_prefix_needs_key_and_equals() {
        assert_eq!(
            kinds("/// url = 'x'")[0],
            (SyntaxKind::MetadataPrefix, "///")
        );
        assert_eq!(
            kinds("/// just a comment")[0],
            (SyntaxKind::LineComment, "/// just a comment")
        );
        assert_eq!(kinds("//// x = 1")[0].0, SyntaxKind::LineComment);
    }

    #[test]
    fn numbers_and_ranges() {
        assert_eq!(
            kinds("0..* 1.5 10L"),
            vec![
                (SyntaxKind::Integer, "0"),
                (SyntaxKind::DotDot, ".."),
                (SyntaxKind::Star, "*"),
                (SyntaxKind::Decimal, "1.5"),
                (SyntaxKind::LongNumber, "10L"),
            ]
        );
    }

    #[test]
    fn dates() {
        assert_eq!(
            kinds("@2024-01-02 @2024-01-02T10:11:12.5+02:00 @T10:30"),
            vec![
                (SyntaxKind::Date, "@2024-01-02"),
                (SyntaxKind::DateTime, "@2024-01-02T10:11:12.5+02:00"),
                (SyntaxKind::Time, "@T10:30"),
            ]
        );
    }

    #[test]
    fn strings() {
        assert_eq!(
            kinds(
                r#"'a\'b' "c" """
md
""" `x y`"#
            ),
            vec![
                (SyntaxKind::SingleQuotedString, r"'a\'b'"),
                (SyntaxKind::DoubleQuotedString, "\"c\""),
                (SyntaxKind::TripleQuotedString, "\"\"\"\nmd\n\"\"\""),
                (SyntaxKind::DelimitedIdent, "`x y`"),
            ]
        );
    }

    #[test]
    fn operators() {
        assert_eq!(
            kinds("-> <<type+>> != <= =="),
            vec![
                (SyntaxKind::Arrow, "->"),
                (SyntaxKind::Shl, "<<"),
                (SyntaxKind::Ident, "type"),
                (SyntaxKind::Plus, "+"),
                (SyntaxKind::Shr, ">>"),
                (SyntaxKind::Neq, "!="),
                (SyntaxKind::Le, "<="),
                (SyntaxKind::EqEq, "=="),
            ]
        );
    }

    #[test]
    fn unterminated_string_reports() {
        let (_, diagnostics) = tokenize("'abc");
        assert_eq!(diagnostics.len(), 1);
    }
}
