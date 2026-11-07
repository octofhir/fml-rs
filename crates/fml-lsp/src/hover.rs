//! Hover: what a name is, with its type.

use fml_check::{Binding, BindingKind, Referent, types::ElementRef};
use fml_syntax::{TextSize, ast};
use lsp_types::{Hover, HoverContents, MarkupContent, MarkupKind};

use crate::document::Document;
use crate::reference::{TRANSFORMS_PAGE, transform};

pub(crate) fn hover(document: &Document, offset: TextSize) -> Option<Hover> {
    let analysis = &document.analysis;
    let (markdown, range) = if let Some(reference) = analysis.reference_at(offset) {
        let text = match &reference.referent {
            Referent::Binding(id) => binding(&analysis.bindings[*id]),
            Referent::Element(element) => self::element(element),
            Referent::Group(name) => group(&document.tree(), name)?,
            Referent::Transform(name) => {
                let transform = transform(name)?;
                format!(
                    "```fml\n{}\n```\n{}\n\n[Transforms]({TRANSFORMS_PAGE})",
                    transform.signature(),
                    transform.description
                )
            }
            Referent::Type(ty) => {
                let structure = ty.structure()?;
                format!("```fml\n{}\n```\n{}", ty.display(), structure.url)
            }
        };
        (text, reference.range)
    } else if let Some((_, binding)) = analysis.binding_at(offset) {
        (self::binding(binding), binding.definition)
    } else {
        let tree = document.tree();
        let group = tree.groups().find(|g| {
            g.name_token()
                .is_some_and(|t| t.text_range().contains_inclusive(offset))
        })?;
        let range = group.name_token()?.text_range();
        (self::group(&tree, &group.name()?)?, range)
    };
    Some(Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value: markdown,
        }),
        range: Some(document.range(range)),
    })
}

fn binding(binding: &Binding) -> String {
    let kind = match binding.kind {
        BindingKind::Constant => "constant",
        BindingKind::SourceParameter => "source parameter",
        BindingKind::TargetParameter => "target parameter",
        BindingKind::Source => "source variable",
        BindingKind::Target => "target variable",
    };
    format!(
        "```fml\n{} : {}\n```\n{kind}",
        binding.name,
        binding.ty.display()
    )
}

pub(crate) fn element_types(element: &ElementRef) -> String {
    let types = if element.element.types.is_empty() {
        match &element.element.content_reference {
            Some((_, path)) => format!("see {}", path.join(".")),
            None => element.owner.type_name.clone(),
        }
    } else {
        element
            .element
            .types
            .iter()
            .map(|t| t.rsplit('/').next().unwrap_or(t))
            .collect::<Vec<_>>()
            .join(" | ")
    };
    if element.element.array {
        format!("{types} [0..*]")
    } else {
        types
    }
}

fn element(element: &ElementRef) -> String {
    let path = std::iter::once(element.owner.type_name.as_str())
        .chain(element.path.iter().map(String::as_str))
        .collect::<Vec<_>>()
        .join(".");
    format!(
        "```fml\n{path} : {}\n```\n{}",
        element_types(element),
        element.owner.url
    )
}

/// `group Name(source src : Type, target tgt)`.
pub(crate) fn group_signature(group: &ast::Group) -> String {
    let parameters = group_parameters(group);
    let mut signature = format!(
        "group {}({})",
        group.name().unwrap_or_default(),
        parameters.join(", ")
    );
    if let Some(base) = group.extends() {
        signature.push_str(" extends ");
        signature.push_str(&base);
    }
    signature
}

/// `source src : Type` for each parameter, as the signature writes it.
pub(crate) fn group_parameters(group: &ast::Group) -> Vec<String> {
    group
        .parameters()
        .map(|p| {
            let mode = match p.mode() {
                Some(ast::ParameterMode::Target) => "target",
                _ => "source",
            };
            let name = p.name().unwrap_or_default();
            match p.type_annotation().and_then(|t| t.type_name()) {
                Some(ty) => format!("{mode} {name} : {ty}"),
                None => format!("{mode} {name}"),
            }
        })
        .collect()
}

fn group(tree: &ast::SourceFile, name: &str) -> Option<String> {
    let group = tree.groups().find(|g| g.name().as_deref() == Some(name))?;
    Some(format!("```fml\n{}\n```", group_signature(&group)))
}
