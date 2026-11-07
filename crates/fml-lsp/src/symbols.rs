//! The outline: declarations, groups with their parameters and rules.

use fml_syntax::{TextRange, ast, ast::AstNode};
use lsp_types::{DocumentSymbol, SymbolKind};

use crate::document::Document;
use crate::hover::group_signature;

#[allow(deprecated)]
fn symbol(
    document: &Document,
    name: String,
    detail: Option<String>,
    kind: SymbolKind,
    range: TextRange,
    selection: TextRange,
    children: Vec<DocumentSymbol>,
) -> DocumentSymbol {
    DocumentSymbol {
        name,
        detail,
        kind,
        tags: None,
        deprecated: None,
        range: document.range(range),
        selection_range: document.range(selection),
        children: (!children.is_empty()).then_some(children),
    }
}

fn node_range(node: &impl AstNode) -> TextRange {
    node.syntax().text_range()
}

pub(crate) fn document_symbols(document: &Document) -> Vec<DocumentSymbol> {
    let tree = document.tree();
    let mut symbols = Vec::new();
    for uses in tree.uses() {
        let url = uses.url().and_then(|u| u.value()).unwrap_or_default();
        let name = uses.alias().unwrap_or_else(|| url.clone());
        let selection = uses
            .alias_token()
            .map(|t| t.text_range())
            .unwrap_or_else(|| node_range(&uses));
        let mode = uses.mode().map(|m| m.as_str().to_owned());
        symbols.push(symbol(
            document,
            name,
            mode.map(|m| format!("{m} {url}")),
            SymbolKind::MODULE,
            node_range(&uses),
            selection,
            Vec::new(),
        ));
    }
    for constant in tree.constants() {
        let Some(token) = constant.name_token() else {
            continue;
        };
        symbols.push(symbol(
            document,
            ast::name_text(&token),
            constant.expression().map(|e| e.text()),
            SymbolKind::CONSTANT,
            node_range(&constant),
            token.text_range(),
            Vec::new(),
        ));
    }
    for concept_map in tree.concept_maps() {
        let name = concept_map
            .url()
            .and_then(|u| u.value())
            .unwrap_or_else(|| "conceptmap".to_owned());
        symbols.push(symbol(
            document,
            name,
            None,
            SymbolKind::OBJECT,
            node_range(&concept_map),
            node_range(&concept_map),
            Vec::new(),
        ));
    }
    for group in tree.groups() {
        let Some(token) = group.name_token() else {
            continue;
        };
        let mut children = Vec::new();
        for parameter in group.parameters() {
            if let Some(name) = parameter.name_token() {
                children.push(symbol(
                    document,
                    ast::name_text(&name),
                    parameter.type_annotation().and_then(|t| t.type_name()),
                    SymbolKind::VARIABLE,
                    node_range(&parameter),
                    name.text_range(),
                    Vec::new(),
                ));
            }
        }
        children.extend(group.rules().map(|rule| rule_symbol(document, &rule)));
        symbols.push(symbol(
            document,
            ast::name_text(&token),
            Some(group_signature(&group)),
            SymbolKind::FUNCTION,
            node_range(&group),
            token.text_range(),
            children,
        ));
    }
    symbols
}

fn rule_symbol(document: &Document, rule: &ast::Rule) -> DocumentSymbol {
    let text = rule.syntax().text().to_string();
    let first_line = text.trim().lines().next().unwrap_or_default().trim();
    let name = rule.name().unwrap_or_else(|| {
        let mut short: String = first_line.chars().take(60).collect();
        if short.len() < first_line.len() {
            short.push('…');
        }
        short
    });
    let children = rule
        .dependent()
        .and_then(|d| d.rules())
        .map(|block| {
            block
                .rules()
                .map(|nested| rule_symbol(document, &nested))
                .collect()
        })
        .unwrap_or_default();
    let range = node_range(rule);
    symbol(
        document,
        name,
        None,
        SymbolKind::EVENT,
        range,
        range,
        children,
    )
}
