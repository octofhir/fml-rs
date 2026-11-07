//! Semantic tokens: what each name is, for colouring beyond the grammar. Variables, parameters,
//! elements, groups, transforms and types each get their own token type.

use fml_check::{BindingKind, Referent};
use fml_syntax::TextRange;
use lsp_types::{
    SemanticToken, SemanticTokenModifier, SemanticTokenType, SemanticTokens, SemanticTokensLegend,
};

use crate::document::Document;

const TYPES: &[SemanticTokenType] = &[
    SemanticTokenType::VARIABLE,
    SemanticTokenType::PARAMETER,
    SemanticTokenType::PROPERTY,
    SemanticTokenType::FUNCTION,
    SemanticTokenType::METHOD,
    SemanticTokenType::TYPE,
];

const MODIFIERS: &[SemanticTokenModifier] = &[
    SemanticTokenModifier::DECLARATION,
    SemanticTokenModifier::READONLY,
    SemanticTokenModifier::DEFAULT_LIBRARY,
];

const VARIABLE: u32 = 0;
const PARAMETER: u32 = 1;
const PROPERTY: u32 = 2;
const FUNCTION: u32 = 3;
const METHOD: u32 = 4;
const TYPE: u32 = 5;

const DECLARATION: u32 = 1;
const READONLY: u32 = 2;
const DEFAULT_LIBRARY: u32 = 4;

pub(crate) fn legend() -> SemanticTokensLegend {
    SemanticTokensLegend {
        token_types: TYPES.to_vec(),
        token_modifiers: MODIFIERS.to_vec(),
    }
}

fn binding_type(kind: BindingKind) -> (u32, u32) {
    match kind {
        BindingKind::Constant => (VARIABLE, READONLY),
        BindingKind::SourceParameter | BindingKind::TargetParameter => (PARAMETER, 0),
        BindingKind::Source | BindingKind::Target => (VARIABLE, 0),
    }
}

pub(crate) fn semantic_tokens(document: &Document) -> SemanticTokens {
    let analysis = &document.analysis;
    let mut tokens: Vec<(TextRange, u32, u32)> = Vec::new();
    for binding in &analysis.bindings {
        let (ty, modifiers) = binding_type(binding.kind);
        tokens.push((binding.definition, ty, modifiers | DECLARATION));
    }
    for reference in &analysis.references {
        let (ty, modifiers) = match &reference.referent {
            Referent::Binding(id) => binding_type(analysis.bindings[*id].kind),
            Referent::Element(_) => (PROPERTY, 0),
            Referent::Group(_) => (FUNCTION, 0),
            Referent::Transform(_) => (METHOD, DEFAULT_LIBRARY),
            Referent::Type(_) => (TYPE, 0),
        };
        tokens.push((reference.range, ty, modifiers));
    }
    let tree = document.tree();
    for group in tree.groups() {
        if let Some(token) = group.name_token() {
            tokens.push((token.text_range(), FUNCTION, DECLARATION));
        }
        if let Some(token) = group.extends_token() {
            tokens.push((token.text_range(), FUNCTION, 0));
        }
    }
    for uses in tree.uses() {
        if let Some(token) = uses.alias_token() {
            tokens.push((token.text_range(), TYPE, DECLARATION));
        }
    }
    tokens.sort_by_key(|(range, _, _)| (range.start(), range.end()));
    tokens.dedup_by(|a, b| a.0 == b.0);

    let mut data = Vec::with_capacity(tokens.len());
    let (mut line, mut column) = (0, 0);
    for (range, ty, modifiers) in tokens {
        let start = document.position(range.start());
        let end = document.position(range.end());
        // Names are on one line; anything else is not a name to colour.
        if start.line != end.line || end.character <= start.character {
            continue;
        }
        let delta_line = start.line - line;
        let delta_start = if delta_line == 0 {
            start.character - column
        } else {
            start.character
        };
        data.push(SemanticToken {
            delta_line,
            delta_start,
            length: end.character - start.character,
            token_type: ty,
            token_modifiers_bitset: modifiers,
        });
        line = start.line;
        column = start.character;
    }
    SemanticTokens {
        result_id: None,
        data,
    }
}
