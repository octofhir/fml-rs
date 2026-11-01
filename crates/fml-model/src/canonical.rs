//! FHIRPath text in the form the HL7 reference compiler stores in a StructureMap.
//!
//! The reference FHIRPath parser (`FHIRPathEngine` in org.hl7.fhir.core) does not build an
//! operator tree. It reads an expression as a flat chain of terms and operators, then groups the
//! chain by precedence level (`organisePrecedence`): wherever operators of one level are mixed with
//! others, each run of that level becomes a parenthesised group. A unary `-` or `+` is an empty
//! term carrying the operator. Writing the chain back gives texts such as
//! `(code = 'x') and (system = 'y')` or `v = ( - 1)`; this module reproduces them, so a compiled
//! StructureMap matches the reference compiler's byte for byte.

use fml_syntax::ast::{self, AstNode, Expr};
use fml_syntax::{NodeOrToken, SyntaxKind, SyntaxNode, expression_text, quote_fhirpath_string};

/// Operator precedence levels, highest first, as `organisePrecedence` gathers them. `implies`, the
/// lowest, is never grouped.
const LEVELS: &[&[&str]] = &[
    &["*", "/", "div", "mod"],
    &["+", "-", "&"],
    &["is", "as"],
    &["|"],
    &["<", ">", "<=", ">="],
    &["=", "~", "!=", "!~"],
    &["in", "contains", "memberOf"],
    &["and"],
    &["xor", "or"],
];

/// The canonical text of an expression, without redundant outer parentheses.
pub fn canonical(expression: &Expr) -> String {
    root(&expression.unparenthesized())
}

/// A full ("proximal") expression: a chain, organised by precedence.
fn root(expression: &Expr) -> String {
    let node = expression.syntax();
    if !is_chain(node.kind()) {
        return term(node);
    }
    let mut chain = Chain::default();
    flatten(node, &mut chain);
    // A unary operator at the start of an expression applies to the whole chain after it.
    if matches!(chain.terms.first(), Some(Term::Empty)) && !chain.operators.is_empty() {
        let operator = chain.operators.remove(0);
        chain.terms.remove(0);
        return format!(" {operator} {}", organise(chain).render());
    }
    organise(chain).render()
}

#[derive(Debug, Clone)]
enum Term {
    Text(String),
    /// The operand-less term of a unary operator.
    Empty,
}

impl Term {
    fn text(&self) -> &str {
        match self {
            Term::Text(text) => text,
            Term::Empty => "",
        }
    }
}

/// Terms joined by operators: `terms.len() == operators.len() + 1`.
#[derive(Debug, Clone, Default)]
struct Chain {
    terms: Vec<Term>,
    operators: Vec<String>,
}

impl Chain {
    fn render(&self) -> String {
        let mut out = self.terms[0].text().to_owned();
        for (operator, term) in self.operators.iter().zip(&self.terms[1..]) {
            out.push(' ');
            out.push_str(operator);
            out.push(' ');
            out.push_str(term.text());
        }
        out
    }
}

fn is_chain(kind: SyntaxKind) -> bool {
    matches!(
        kind,
        SyntaxKind::BinaryExpr | SyntaxKind::TypeExpr | SyntaxKind::PrefixExpr
    )
}

fn flatten(node: &SyntaxNode, chain: &mut Chain) {
    match node.kind() {
        SyntaxKind::BinaryExpr | SyntaxKind::TypeExpr => {
            let mut operands = node.children();
            let left = operands.next();
            let right = operands.next();
            if let Some(left) = &left {
                flatten(left, chain);
            }
            chain.operators.push(operator_text(node));
            match (node.kind(), right) {
                (SyntaxKind::TypeExpr, Some(type_name)) => {
                    chain.terms.push(Term::Text(term(&type_name)))
                }
                (_, Some(right)) => flatten(&right, chain),
                (_, None) => chain.terms.push(Term::Text(String::new())),
            }
        }
        SyntaxKind::PrefixExpr => {
            chain.terms.push(Term::Empty);
            chain.operators.push(operator_text(node));
            match node.children().next() {
                Some(operand) => flatten(&operand, chain),
                None => chain.terms.push(Term::Text(String::new())),
            }
        }
        _ => chain.terms.push(Term::Text(term(node))),
    }
}

/// The operator token directly under a binary, type or prefix expression.
fn operator_text(node: &SyntaxNode) -> String {
    node.children_with_tokens()
        .filter_map(|e| e.into_token())
        .find(|t| !t.kind().is_trivia())
        .map(|t| t.text().to_owned())
        .unwrap_or_default()
}

/// `gatherPrecedence` for every level: where a level's operators are mixed with others, each run
/// of that level's operators and their terms becomes one parenthesised term.
fn organise(mut chain: Chain) -> Chain {
    for level in LEVELS {
        let in_level = |operator: &String| level.contains(&operator.as_str());
        let mixed = chain.operators.iter().any(in_level) && !chain.operators.iter().all(in_level);
        if !mixed {
            continue;
        }
        let mut grouped = Chain::default();
        let mut run = Chain {
            terms: vec![chain.terms[0].clone()],
            operators: Vec::new(),
        };
        for (operator, next) in chain.operators.iter().zip(&chain.terms[1..]) {
            if in_level(operator) {
                run.operators.push(operator.clone());
                run.terms.push(next.clone());
            } else {
                grouped.terms.push(close_run(run));
                grouped.operators.push(operator.clone());
                run = Chain {
                    terms: vec![next.clone()],
                    operators: Vec::new(),
                };
            }
        }
        grouped.terms.push(close_run(run));
        chain = grouped;
    }
    chain
}

fn close_run(run: Chain) -> Term {
    if run.operators.is_empty() {
        run.terms.into_iter().next().expect("a run has a term")
    } else {
        Term::Text(format!("({})", run.render()))
    }
}

/// A term: everything that is not a binary chain.
fn term(node: &SyntaxNode) -> String {
    let expr = |node: &SyntaxNode| Expr::cast(node.clone());
    match node.kind() {
        SyntaxKind::ParenExpr => match node.children().find_map(Expr::cast) {
            Some(inner) => format!("({})", root(&inner)),
            None => "()".to_owned(),
        },
        SyntaxKind::Literal => literal(node),
        SyntaxKind::NullLiteral => "{}".to_owned(),
        SyntaxKind::QualifiedName => ast::QualifiedName::cast(node.clone())
            .map(|n| n.syntax().text().to_string().trim().to_owned())
            .unwrap_or_default(),
        SyntaxKind::CallExpr => {
            let name = node
                .first_token()
                .map(|t| t.text().to_owned())
                .unwrap_or_default();
            format!("{name}({})", arguments(node))
        }
        SyntaxKind::MemberExpr => {
            let mut parts = node.children();
            let left = parts.next().map(|n| term(&n)).unwrap_or_default();
            let right = parts.next().map(|n| term(&n)).unwrap_or_default();
            format!("{left}.{right}")
        }
        SyntaxKind::IndexExpr => {
            let mut parts = node.children();
            let left = parts.next().map(|n| term(&n)).unwrap_or_default();
            let index = parts
                .next()
                .and_then(|n| expr(&n))
                .map(|e| root(&e))
                .unwrap_or_default();
            format!("{left}[{index}]")
        }
        _ if is_chain(node.kind()) => match expr(node) {
            Some(e) => root(&e),
            None => expression_text(node),
        },
        _ => expression_text(node),
    }
}

fn arguments(call: &SyntaxNode) -> String {
    let Some(list) = call
        .children()
        .find(|n| n.kind() == SyntaxKind::ArgumentList)
    else {
        return String::new();
    };
    let mut out = Vec::new();
    let mut current: Option<String> = None;
    for element in list.children_with_tokens() {
        match element {
            NodeOrToken::Node(node) => {
                if let Some(e) = Expr::cast(node) {
                    current = Some(root(&e));
                }
            }
            NodeOrToken::Token(token) => match token.text() {
                "," => out.extend(current.take()),
                "asc" | "desc" => {
                    if let Some(argument) = &mut current {
                        argument.push(' ');
                        argument.push_str(token.text());
                    }
                }
                _ => {}
            },
        }
    }
    out.extend(current);
    out.join(", ")
}

fn literal(node: &SyntaxNode) -> String {
    let Some(token) = node
        .children_with_tokens()
        .filter_map(|e| e.into_token())
        .find(|t| !t.kind().is_trivia())
    else {
        return String::new();
    };
    match token.kind() {
        SyntaxKind::SingleQuotedString | SyntaxKind::DoubleQuotedString => {
            quote_fhirpath_string(&ast::unescape(token.text()))
        }
        _ => token.text().to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use fml_syntax::parse;

    use super::*;

    fn canon(expression: &str) -> String {
        let source = format!("let x = {expression};");
        let parse = parse(&source);
        assert!(parse.diagnostics().is_empty(), "{:?}", parse.diagnostics());
        let constant = parse.tree().constants().next().unwrap();
        canonical(&constant.expression().unwrap())
    }

    #[test]
    fn groups_mixed_precedence_like_the_reference_parser() {
        assert_eq!(
            canon("coding.exists(code = 'a' and system = \"b\")"),
            "coding.exists((code = 'a') and (system = 'b'))"
        );
        assert_eq!(canon("a + b * c"), "a + (b * c)");
        assert_eq!(canon("a * b + c * d = e"), "((a * b) + (c * d)) = e");
        assert_eq!(canon("a - b + c"), "a - b + c");
        assert_eq!(canon("a and b and c"), "a and b and c");
        assert_eq!(canon("(a)"), "a");
        assert_eq!(canon("(a or b).not()"), "(a or b).not()");
        assert_eq!(canon("x is Quantity and y"), "(x is Quantity) and y");
    }

    #[test]
    fn unary_operators_are_empty_terms() {
        assert_eq!(canon("v = (-1)"), "v = ( - 1)");
        assert_eq!(canon("v = -1"), "v = ( - 1)");
        assert_eq!(canon("-a + b"), " - a + b");
    }

    #[test]
    fn terms() {
        assert_eq!(canon("%resource.item[0].value"), "%resource.item[0].value");
        assert_eq!(
            canon("a.where(b > 1).select(c)"),
            "a.where(b > 1).select(c)"
        );
        assert_eq!(canon("{}"), "{}");
        assert_eq!(canon("'it\\'s'"), "'it\\'s'");
    }
}
