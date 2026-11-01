//! FHIRPath, supplied by the host.
//!
//! The engine evaluates `where`, `check`, `log`, `default` and `evaluate` through [`Expressions`],
//! so it builds without a FHIRPath implementation; the `fhirpath` feature provides one.

use std::collections::HashSet;
use std::future::Future;

use fml_syntax::{AstNode, SyntaxKind, SyntaxNode, TextSize, parse};
use serde_json::Value;

/// A value passed to or returned by FHIRPath: FHIR JSON and its FHIR type where known
/// (`Patient`, `HumanName`, `string`).
#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    pub json: Value,
    pub type_name: Option<String>,
}

/// Evaluates FHIRPath expressions.
pub trait Expressions {
    /// Evaluates `expression` against `focus` (none for expressions that only use variables).
    /// Variables are available as `%name`.
    fn evaluate(
        &self,
        expression: &str,
        focus: Option<&Item>,
        variables: &[(String, Item)],
    ) -> impl Future<Output = Result<Vec<Item>, String>>;
}

/// No FHIRPath: maps that use expressions fail with a clear error.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoExpressions;

impl Expressions for NoExpressions {
    async fn evaluate(
        &self,
        expression: &str,
        _focus: Option<&Item>,
        _variables: &[(String, Item)],
    ) -> Result<Vec<Item>, String> {
        Err(format!(
            "`{expression}` needs FHIRPath; build fml-engine with the `fhirpath` feature or supply an evaluator"
        ))
    }
}

/// Rewrites the map's variables to FHIRPath environment variables: in a StructureMap, a name
/// that is a variable refers to it (the reference engine resolves such names before the focus),
/// so `item.answer` with a variable `item` becomes `%item.answer`.
pub fn bind_variables(expression: &str, variables: &HashSet<String>) -> String {
    const PREFIX: &str = "let _ = ";
    let source = format!("{PREFIX}{expression};");
    let parse = parse(&source);
    let Some(constant) = parse.tree().constants().next() else {
        return expression.to_owned();
    };
    let Some(root) = constant.expression() else {
        return expression.to_owned();
    };
    let mut positions: Vec<TextSize> = root
        .syntax()
        .descendants()
        .filter(|node| node.kind() == SyntaxKind::NameRef && is_path_start(node))
        .filter_map(|node| {
            let token = node.first_token()?;
            (token.kind() == SyntaxKind::Ident && variables.contains(token.text()))
                .then(|| token.text_range().start())
        })
        .collect();
    positions.sort();
    let mut out = String::with_capacity(expression.len() + positions.len());
    let mut last = PREFIX.len();
    for position in positions {
        let position = usize::from(position);
        out.push_str(&source[last..position]);
        out.push('%');
        last = position;
    }
    out.push_str(&source[last..source.len() - 1]);
    out
}

/// Whether a name starts a path, rather than following a `.`.
fn is_path_start(name: &SyntaxNode) -> bool {
    match name.parent() {
        Some(parent) if parent.kind() == SyntaxKind::MemberExpr => {
            parent.children().next().as_ref() == Some(name)
        }
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binds_variables_at_path_starts() {
        let variables: HashSet<String> = ["item", "src"].into_iter().map(String::from).collect();
        assert_eq!(
            bind_variables("item.answer.valueString", &variables),
            "%item.answer.valueString"
        );
        assert_eq!(
            bind_variables("linkId.value = 'x' and src.item.exists()", &variables),
            "linkId.value = 'x' and %src.item.exists()"
        );
        assert_eq!(
            bind_variables("answer.where(item = 1).item", &variables),
            "answer.where(%item = 1).item"
        );
        assert_eq!(bind_variables("%item", &variables), "%item");
    }
}
