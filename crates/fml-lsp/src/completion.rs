//! Completion: elements after `.`, variables, groups after `then`, transforms after `=`, types
//! after `:`, and keywords.

use fml_check::types::{Lookup, Type, Types};
use fml_check::{BindingKind, Definitions, Referent};
use fml_syntax::{SyntaxKind, TextSize};
use lsp_types::{
    CompletionItem, CompletionItemKind, CompletionList, Documentation, InsertTextFormat,
    MarkupContent, MarkupKind,
};

use crate::document::Document;
use crate::hover::{element_types, group_signature};
use crate::reference::{FHIRPATH_FUNCTIONS, KEYWORDS, TRANSFORMS, TRANSFORMS_PAGE};

pub(crate) fn completion(
    document: &Document,
    definitions: &dyn Definitions,
    offset: TextSize,
) -> Option<CompletionList> {
    if let Some(token) = document.token_at(offset)
        && token.text_range().start() < offset
        && (token.kind().is_comment()
            || matches!(
                token.kind(),
                SyntaxKind::SingleQuotedString | SyntaxKind::DoubleQuotedString
            ))
    {
        return None;
    }
    let line = document.line_before(offset);
    let word_start = line
        .rfind(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .map_or(0, |i| i + 1);
    let before = &line[..word_start];
    let word_offset = offset - TextSize::from((line.len() - word_start) as u32);
    let types = Types::new(definitions);

    let items = if let Some(chain) = before.strip_suffix('.') {
        let ty = chain_type(document, &types, chain, word_offset)?;
        let mut items = elements(&types, &ty);
        if in_expression(document, offset) {
            items.extend(functions());
        }
        items
    } else if after_then(before) {
        groups(document)
    } else if let Some(rest) = before.trim_end().strip_suffix(':')
        && !rest.ends_with(':')
    {
        annotation_types(document, rest.trim_end(), word_offset)
    } else if transform_position(before) && !in_expression(document, offset) {
        let mut items = transforms();
        items.extend(variables(document, offset));
        items
    } else {
        let mut items = variables(document, offset);
        if !in_expression(document, offset) {
            items.extend(keywords());
        }
        items
    };
    Some(CompletionList {
        is_incomplete: false,
        items,
    })
}

/// The type of `a.b.c` ending just before `end`: a variable followed through its elements, or
/// an element of the expression focus.
fn chain_type(document: &Document, types: &Types, chain: &str, end: TextSize) -> Option<Type> {
    let start = chain
        .rfind(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '.'))
        .map_or(0, |i| i + 1);
    let segments: Vec<&str> = chain[start..].split('.').collect();
    let first = *segments.first()?;
    if first.is_empty() {
        return None;
    }
    let chain_start = end - TextSize::from((chain.len() - start + 1) as u32);
    let mut ty = match document
        .analysis
        .bindings_at(chain_start)
        .into_iter()
        .find(|b| b.name == first)
    {
        Some(binding) => binding.ty.clone(),
        None => document.analysis.reference_at(chain_start)?.ty.clone(),
    };
    for segment in &segments[1..] {
        ty = match types.lookup(&ty, segment) {
            Lookup::Found(found) => types.element_type(&found, None),
            _ => return None,
        };
    }
    Some(ty)
}

/// Inside a FHIRPath expression: a `where`, `check` or `log` clause, a default, a constant, or a
/// parenthesised transform.
fn in_expression(document: &Document, offset: TextSize) -> bool {
    let Some(token) = document.token_at(offset) else {
        return false;
    };
    token.parent_ancestors().any(|node| {
        matches!(
            node.kind(),
            SyntaxKind::WhereClause
                | SyntaxKind::CheckClause
                | SyntaxKind::LogClause
                | SyntaxKind::DefaultClause
                | SyntaxKind::ConstantDeclaration
                | SyntaxKind::ParenExpr
        )
    })
}

/// `then `, or `then A(x), ` for the next group call.
fn after_then(before: &str) -> bool {
    let trimmed = before.trim_end();
    if trimmed.ends_with("then") {
        return trimmed.len() == 4 || !is_name_char(trimmed.as_bytes()[trimmed.len() - 5]);
    }
    if let Some(rest) = trimmed.strip_suffix(',')
        && let Some(index) = rest.rfind("then")
    {
        let between = &rest[index + 4..];
        return between.starts_with(char::is_whitespace)
            && between.chars().filter(|&c| c == '(').count()
                == between.chars().filter(|&c| c == ')').count()
            && !between.contains(['{', ';']);
    }
    false
}

/// After a target's `=`, where a transform or a variable goes.
fn transform_position(before: &str) -> bool {
    let trimmed = before.trim_end();
    trimmed.ends_with('=')
        && !trimmed.ends_with("==")
        && !trimmed.ends_with("!=")
        && !trimmed.ends_with("<=")
        && !trimmed.ends_with(">=")
        && trimmed.contains("->")
}

fn is_name_char(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

fn markdown(value: String) -> Documentation {
    Documentation::MarkupContent(MarkupContent {
        kind: MarkupKind::Markdown,
        value,
    })
}

fn elements(types: &Types, ty: &Type) -> Vec<CompletionItem> {
    types
        .elements(ty)
        .into_iter()
        // `deceased`, not `deceasedBoolean`: the type goes after `:`.
        .filter(|element| element.element.choice_of.is_none())
        .enumerate()
        .map(|(index, element)| CompletionItem {
            label: element.name.clone(),
            kind: Some(CompletionItemKind::FIELD),
            detail: Some(element_types(&element)),
            documentation: Some(markdown(format!(
                "{}.{}",
                element.owner.type_name,
                element.path.join(".")
            ))),
            // Definition order, not alphabetical.
            sort_text: Some(format!("{index:04}")),
            ..CompletionItem::default()
        })
        .collect()
}

fn functions() -> Vec<CompletionItem> {
    FHIRPATH_FUNCTIONS
        .iter()
        .map(|(name, signature)| CompletionItem {
            label: (*name).to_owned(),
            kind: Some(CompletionItemKind::FUNCTION),
            detail: Some((*signature).to_owned()),
            sort_text: Some(format!("z{name}")),
            insert_text: Some(format!("{name}($0)")),
            insert_text_format: Some(InsertTextFormat::SNIPPET),
            ..CompletionItem::default()
        })
        .collect()
}

fn variables(document: &Document, offset: TextSize) -> Vec<CompletionItem> {
    document
        .analysis
        .bindings_at(offset)
        .into_iter()
        .map(|binding| CompletionItem {
            label: binding.name.clone(),
            kind: Some(match binding.kind {
                BindingKind::Constant => CompletionItemKind::CONSTANT,
                _ => CompletionItemKind::VARIABLE,
            }),
            detail: Some(binding.ty.display()),
            sort_text: Some(format!("0{}", binding.name)),
            ..CompletionItem::default()
        })
        .collect()
}

fn groups(document: &Document) -> Vec<CompletionItem> {
    document
        .tree()
        .groups()
        .filter_map(|group| {
            let name = group.name()?;
            let arguments: Vec<String> = group
                .parameters()
                .enumerate()
                .map(|(i, p)| format!("${{{}:{}}}", i + 1, p.name().unwrap_or_default()))
                .collect();
            Some(CompletionItem {
                label: name.clone(),
                kind: Some(CompletionItemKind::FUNCTION),
                detail: Some(group_signature(&group)),
                insert_text: Some(format!("{name}({})", arguments.join(", "))),
                insert_text_format: Some(InsertTextFormat::SNIPPET),
                ..CompletionItem::default()
            })
        })
        .collect()
}

fn transforms() -> Vec<CompletionItem> {
    TRANSFORMS
        .iter()
        .map(|transform| CompletionItem {
            label: transform.name.to_owned(),
            kind: Some(CompletionItemKind::METHOD),
            detail: Some(transform.signature()),
            documentation: Some(markdown(format!(
                "{}\n\n[Transforms]({TRANSFORMS_PAGE})",
                transform.description
            ))),
            insert_text: Some(transform.snippet()),
            insert_text_format: Some(InsertTextFormat::SNIPPET),
            sort_text: Some(format!("1{}", transform.name)),
            ..CompletionItem::default()
        })
        .collect()
}

/// After `:`: the types of the element the source reads (for a choice element), and the `uses`
/// aliases.
fn annotation_types(document: &Document, before: &str, end: TextSize) -> Vec<CompletionItem> {
    let mut items = Vec::new();
    // The element right before the colon: nothing but whitespace and `:` in between.
    let element = document.analysis.references.iter().rev().find(|r| {
        r.range.end() <= end
            && document.text[usize::from(r.range.end())..usize::from(end)]
                .chars()
                .all(|c| c == ':' || c.is_whitespace())
    });
    if before.ends_with(|c: char| c.is_ascii_alphanumeric() || c == '_' || c == ']')
        && let Some(reference) = element
        && let Referent::Element(element) = &reference.referent
    {
        for code in &element.element.types {
            let name = code.rsplit('/').next().unwrap_or(code);
            items.push(CompletionItem {
                label: name.to_owned(),
                kind: Some(CompletionItemKind::CLASS),
                detail: Some(format!("a type of {}", element.name)),
                sort_text: Some(format!("0{name}")),
                ..CompletionItem::default()
            });
        }
    }
    for uses in document.tree().uses() {
        if let (Some(alias), Some(url)) = (uses.alias(), uses.url().and_then(|u| u.value())) {
            items.push(CompletionItem {
                label: alias,
                kind: Some(CompletionItemKind::CLASS),
                detail: Some(url),
                sort_text: Some("1".to_owned()),
                ..CompletionItem::default()
            });
        }
    }
    items
}

fn keywords() -> Vec<CompletionItem> {
    KEYWORDS
        .iter()
        .map(|keyword| CompletionItem {
            label: (*keyword).to_owned(),
            kind: Some(CompletionItemKind::KEYWORD),
            sort_text: Some(format!("2{keyword}")),
            ..CompletionItem::default()
        })
        .collect()
}
