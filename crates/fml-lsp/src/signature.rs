//! Signature help for transforms and group calls.

use fml_syntax::{SyntaxKind, TextSize, ast, ast::AstNode};
use lsp_types::{
    Documentation, MarkupContent, MarkupKind, ParameterInformation, ParameterLabel, SignatureHelp,
    SignatureInformation,
};

use crate::document::Document;
use crate::hover::{group_parameters, group_signature};
use crate::reference::transform;

pub(crate) fn signature_help(document: &Document, offset: TextSize) -> Option<SignatureHelp> {
    let token = document.token_at(offset)?;
    let invocation = token
        .parent_ancestors()
        .find_map(ast::Invocation::cast)
        .filter(|i| {
            // Inside the parentheses, not on the name.
            i.syntax()
                .children()
                .find(|n| n.kind() == SyntaxKind::ArgumentList)
                .is_some_and(|list| {
                    list.text_range().start() < offset && offset <= list.text_range().end()
                })
        })?;
    let name = invocation.name()?;
    let list = invocation
        .syntax()
        .children()
        .find(|n| n.kind() == SyntaxKind::ArgumentList)?;
    let active = list
        .children_with_tokens()
        .filter_map(|e| e.into_token())
        .filter(|t| t.kind() == SyntaxKind::Comma && t.text_range().end() <= offset)
        .count() as u32;
    let in_dependent = invocation
        .syntax()
        .parent()
        .is_some_and(|p| p.kind() == SyntaxKind::DependentClause);
    let (label, parameters, documentation) = match (in_dependent, transform(&name)) {
        (false, Some(transform)) => (
            transform.signature(),
            transform
                .parameters
                .iter()
                .map(|p| (*p).to_owned())
                .collect::<Vec<_>>(),
            Some(transform.description.to_owned()),
        ),
        _ => {
            let group = document
                .tree()
                .groups()
                .find(|g| g.name().as_deref() == Some(name.as_str()))?;
            (group_signature(&group), group_parameters(&group), None)
        }
    };
    let repeats = parameters.last().is_some_and(|p| p.ends_with("..."));
    let active = if repeats {
        active.min(parameters.len().saturating_sub(1) as u32)
    } else {
        active
    };
    Some(SignatureHelp {
        signatures: vec![SignatureInformation {
            label,
            documentation: documentation.map(|value| {
                Documentation::MarkupContent(MarkupContent {
                    kind: MarkupKind::Markdown,
                    value,
                })
            }),
            parameters: Some(
                parameters
                    .into_iter()
                    .map(|p| ParameterInformation {
                        label: ParameterLabel::Simple(p),
                        documentation: None,
                    })
                    .collect(),
            ),
            active_parameter: Some(active),
        }],
        active_signature: Some(0),
        active_parameter: Some(active),
    })
}
