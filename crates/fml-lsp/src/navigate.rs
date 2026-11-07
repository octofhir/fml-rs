//! Go to definition, find references and rename, for variables and groups.

use std::collections::HashMap;

use fml_check::Referent;
use fml_syntax::{TextRange, TextSize};
use lsp_types::{TextEdit, Url, WorkspaceEdit};

use crate::document::Document;

/// A name that can be followed and renamed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Symbol {
    /// A variable: an index into the analysis bindings.
    Binding(usize),
    Group(String),
}

/// The symbol at `offset`, and the range of the name there.
pub(crate) fn symbol_at(document: &Document, offset: TextSize) -> Option<(Symbol, TextRange)> {
    let analysis = &document.analysis;
    if let Some(reference) = analysis.reference_at(offset) {
        match &reference.referent {
            Referent::Binding(id) => return Some((Symbol::Binding(*id), reference.range)),
            Referent::Group(name) => return Some((Symbol::Group(name.clone()), reference.range)),
            _ => {}
        }
    }
    if let Some((id, binding)) = analysis.binding_at(offset) {
        return Some((Symbol::Binding(id), binding.definition));
    }
    for group in document.tree().groups() {
        for token in [group.name_token(), group.extends_token()]
            .into_iter()
            .flatten()
        {
            if token.text_range().contains_inclusive(offset) {
                let name = fml_syntax::ast::name_text(&token);
                return Some((Symbol::Group(name), token.text_range()));
            }
        }
    }
    None
}

/// Where the symbol is declared.
fn declaration(document: &Document, symbol: &Symbol) -> Option<TextRange> {
    match symbol {
        Symbol::Binding(id) => Some(document.analysis.bindings[*id].definition),
        Symbol::Group(name) => document
            .tree()
            .groups()
            .find(|g| g.name().as_deref() == Some(name))
            .and_then(|g| g.name_token())
            .map(|t| t.text_range()),
    }
}

/// Every occurrence of the symbol, the declaration first.
pub(crate) fn occurrences(document: &Document, symbol: &Symbol) -> Vec<TextRange> {
    let mut ranges: Vec<TextRange> = declaration(document, symbol).into_iter().collect();
    match symbol {
        Symbol::Binding(id) => {
            ranges.extend(document.analysis.references_to(*id).map(|r| r.range));
        }
        Symbol::Group(name) => {
            ranges.extend(
                document
                    .analysis
                    .references
                    .iter()
                    .filter(|r| matches!(&r.referent, Referent::Group(n) if n == name))
                    .map(|r| r.range),
            );
            ranges.extend(
                document
                    .tree()
                    .groups()
                    .filter_map(|g| g.extends_token())
                    .filter(|t| &fml_syntax::ast::name_text(t) == name)
                    .map(|t| t.text_range()),
            );
        }
    }
    let first = ranges.first().copied();
    ranges.sort_by_key(|r| r.start());
    ranges.dedup();
    // Keep the declaration first.
    if let Some(first) = first
        && let Some(index) = ranges.iter().position(|r| *r == first)
    {
        ranges.remove(index);
        ranges.insert(0, first);
    }
    ranges
}

/// The declaration of the name at `offset`: a variable, a group, or the `uses` alias a type
/// annotation names.
pub(crate) fn definition(document: &Document, offset: TextSize) -> Option<TextRange> {
    if let Some((symbol, _)) = symbol_at(document, offset) {
        return declaration(document, &symbol);
    }
    let reference = document.analysis.reference_at(offset)?;
    let Referent::Type(_) = reference.referent else {
        return None;
    };
    let written = document.text[reference.range].trim_matches(['"', '\'', '`']);
    document
        .tree()
        .uses()
        .find(|u| u.alias().as_deref() == Some(written))
        .and_then(|u| u.alias_token())
        .map(|t| t.text_range())
}

/// Why a name cannot be used, if it cannot.
pub(crate) fn invalid_name(name: &str) -> Option<String> {
    let mut chars = name.chars();
    let valid = chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_');
    if !valid {
        return Some(format!(
            "`{name}` is not a name: use letters, digits and `_`"
        ));
    }
    if crate::reference::KEYWORDS.contains(&name)
        || ["true", "false", "type", "types"].contains(&name)
    {
        return Some(format!("`{name}` is a keyword"));
    }
    None
}

pub(crate) fn rename(
    document: &Document,
    uri: &Url,
    offset: TextSize,
    new_name: &str,
) -> Result<Option<WorkspaceEdit>, String> {
    if let Some(problem) = invalid_name(new_name) {
        return Err(problem);
    }
    let Some((symbol, _)) = symbol_at(document, offset) else {
        return Ok(None);
    };
    let edits = occurrences(document, &symbol)
        .into_iter()
        .map(|range| TextEdit::new(document.range(range), new_name.to_owned()))
        .collect();
    Ok(Some(WorkspaceEdit {
        changes: Some(HashMap::from([(uri.clone(), edits)])),
        ..WorkspaceEdit::default()
    }))
}
