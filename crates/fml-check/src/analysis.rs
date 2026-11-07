//! What the checker learns about a map besides its problems: the variables, where each is
//! declared and visible, and what each name in the map refers to. Editors use it for hover,
//! completion, go to definition, find references and rename.

use fml_syntax::{Diagnostic, TextRange, TextSize};

use crate::types::{ElementRef, Type};

/// The result of [`analyze`](crate::analyze).
#[derive(Debug, Clone, Default)]
pub struct Analysis {
    /// The problems [`check`](crate::check) reports.
    pub diagnostics: Vec<Diagnostic>,
    /// Constants, group parameters and rule aliases, in the order they are declared.
    pub bindings: Vec<Binding>,
    /// The names that refer to something, ordered by position.
    pub references: Vec<Reference>,
}

/// How a variable came to be.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BindingKind {
    /// `let name = …;`
    Constant,
    /// `source src : Type` or `target tgt : Type` in a group's parameters.
    SourceParameter,
    TargetParameter,
    /// `src.name as n`
    Source,
    /// `tgt.name as n`
    Target,
}

impl BindingKind {
    /// Whether the variable is a target, which rules write into.
    pub fn is_target(self) -> bool {
        matches!(self, BindingKind::TargetParameter | BindingKind::Target)
    }
}

/// A variable.
#[derive(Debug, Clone)]
pub struct Binding {
    pub name: String,
    pub kind: BindingKind,
    /// The name where it is declared.
    pub definition: TextRange,
    /// Where it can be used: the group of a parameter, the rest of the rule of an alias, the
    /// whole map for a constant.
    pub visible: TextRange,
    /// The type, where known. An untyped group parameter has the type of the first call that
    /// gives one.
    pub ty: Type,
}

/// What a name refers to.
#[derive(Debug, Clone)]
pub enum Referent {
    /// A variable: an index into [`Analysis::bindings`].
    Binding(usize),
    /// An element of a type.
    Element(Box<ElementRef>),
    /// A group, by name.
    Group(String),
    /// A transform, such as `create` or `translate`.
    Transform(String),
    /// A type: a parameter or source type, or what `create('Type')` makes.
    Type(Type),
}

/// A name in the map and what it refers to.
#[derive(Debug, Clone)]
pub struct Reference {
    pub range: TextRange,
    pub referent: Referent,
    /// The type of the value the name gives, where known.
    pub ty: Type,
}

impl Analysis {
    /// The reference whose name covers `offset`, or ends at it (the cursor right after a name).
    pub fn reference_at(&self, offset: TextSize) -> Option<&Reference> {
        let index = self.references.partition_point(|r| r.range.end() < offset);
        self.references[index..]
            .iter()
            .take_while(|r| r.range.start() <= offset)
            .find(|r| r.range.contains_inclusive(offset))
    }

    /// The variable declared at `offset`.
    pub fn binding_at(&self, offset: TextSize) -> Option<(usize, &Binding)> {
        self.bindings
            .iter()
            .enumerate()
            .find(|(_, b)| b.definition.contains_inclusive(offset))
    }

    /// The variables that can be used at `offset`, the innermost one for each name.
    pub fn bindings_at(&self, offset: TextSize) -> Vec<&Binding> {
        let mut visible: Vec<&Binding> = Vec::new();
        for binding in self
            .bindings
            .iter()
            .filter(|b| b.visible.contains_inclusive(offset))
        {
            match visible.iter_mut().find(|b| b.name == binding.name) {
                Some(seen) if binding.visible.len() < seen.visible.len() => *seen = binding,
                Some(_) => {}
                None => visible.push(binding),
            }
        }
        visible
    }

    /// The references to the variable `binding`.
    pub fn references_to(&self, binding: usize) -> impl Iterator<Item = &Reference> {
        self.references
            .iter()
            .filter(move |r| matches!(r.referent, Referent::Binding(b) if b == binding))
    }
}
