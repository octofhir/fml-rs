//! Navigating types: the type of a variable, its elements, and the types of those.
//!
//! Shared by the checker and the engine, so both see the same elements: inherited ones from base
//! structures, backbone elements declared inline, content references (`Questionnaire.item.item`),
//! a primitive's `value`, and the type a choice element takes.

use std::sync::Arc;

use crate::definitions::{Definitions, Element, Kind, Structure, same_primitive, type_url};

/// The type of a value: a structure, or a backbone element inside one.
#[derive(Debug, Clone)]
pub enum Type {
    /// Not known: no definition, a choice without a chosen type, an expression result.
    Unknown,
    Node {
        structure: Arc<Structure>,
        /// The element path to a backbone element; empty for the structure itself.
        path: Vec<String>,
    },
}

impl Type {
    /// A key that identifies the type, for caching and comparisons.
    pub fn key(&self) -> String {
        match self {
            Type::Unknown => "?".to_owned(),
            Type::Node { structure, path } => {
                let mut key = structure.url.clone();
                for segment in path {
                    key.push('.');
                    key.push_str(segment);
                }
                key
            }
        }
    }

    /// `Patient` or `Patient.contact`, for messages.
    pub fn display(&self) -> String {
        match self {
            Type::Unknown => "?".to_owned(),
            Type::Node { structure, path } => {
                let mut name = structure.type_name.clone();
                for segment in path {
                    name.push('.');
                    name.push_str(segment);
                }
                name
            }
        }
    }

    /// The FHIR type name, as `fhirType()` gives it: `Patient`, `string`, or `BackboneElement` for
    /// an element declared inline.
    pub fn fhir_type(&self) -> Option<String> {
        match self {
            Type::Unknown => None,
            Type::Node { structure, path } if path.is_empty() => Some(structure.type_name.clone()),
            Type::Node { .. } => Some("BackboneElement".to_owned()),
        }
    }

    pub fn structure(&self) -> Option<&Arc<Structure>> {
        match self {
            Type::Unknown => None,
            Type::Node { structure, .. } => Some(structure),
        }
    }

    /// Whether this is a resource (a resource structure itself, not an element of one).
    pub fn is_resource(&self) -> bool {
        matches!(self, Type::Node { structure, path } if path.is_empty() && structure.kind == Kind::Resource)
    }

    pub fn is_primitive(&self) -> bool {
        matches!(self, Type::Node { structure, path } if path.is_empty() && structure.kind == Kind::PrimitiveType)
    }
}

/// An element found on a type.
#[derive(Debug, Clone)]
pub struct ElementRef {
    /// The element's name, as looked up.
    pub name: String,
    pub element: Element,
    /// The structure that defines the element.
    pub owner: Arc<Structure>,
    /// The path to the element inside `owner`.
    pub path: Vec<String>,
}

/// The result of looking up an element.
#[derive(Debug, Clone)]
pub enum Lookup {
    Found(Box<ElementRef>),
    /// The type is known and has no such element.
    Missing,
    /// The type, or part of the chain needed to answer, is not known.
    Unknown,
}

/// Navigates types through a set of definitions.
#[derive(Clone, Copy)]
pub struct Types<'a> {
    pub definitions: &'a dyn Definitions,
}

impl<'a> Types<'a> {
    pub fn new(definitions: &'a dyn Definitions) -> Self {
        Self { definitions }
    }

    /// The type a canonical URL names.
    pub fn structure(&self, url: &str) -> Type {
        match self.definitions.structure(url) {
            Some(structure) => Type::Node {
                structure,
                path: Vec::new(),
            },
            None => Type::Unknown,
        }
    }

    /// The element `name` of `ty`, declared or inherited.
    pub fn lookup(&self, ty: &Type, name: &str) -> Lookup {
        let Type::Node { structure, path } = ty else {
            return Lookup::Unknown;
        };
        if path.is_empty() {
            return self.lookup_in_structure(structure.clone(), name, 0);
        }
        let Some(node) = self.element_at(structure, path) else {
            return Lookup::Unknown;
        };
        if let Some(element) = node.children.get(name) {
            let mut element_path = path.clone();
            element_path.push(name.to_owned());
            return Lookup::Found(Box::new(ElementRef {
                name: name.to_owned(),
                element: element.clone(),
                owner: structure.clone(),
                path: element_path,
            }));
        }
        // Not declared inline: inherited from the backbone's own type (BackboneElement, Element).
        match node.types.first() {
            Some(code) => match self.definitions.structure(&type_url(code, &structure.url)) {
                Some(base) => self.lookup_in_structure(base, name, 0),
                None => Lookup::Unknown,
            },
            None => Lookup::Unknown,
        }
    }

    /// Every element of `ty`, declared or inherited, own elements first and each in definition
    /// order. Empty when the type is not known.
    pub fn elements(&self, ty: &Type) -> Vec<ElementRef> {
        let Type::Node { structure, path } = ty else {
            return Vec::new();
        };
        let mut found = Vec::new();
        if path.is_empty() {
            self.collect_elements(structure.clone(), &mut found);
            return found;
        }
        let Some(node) = self.element_at(structure, path) else {
            return found;
        };
        let mut children: Vec<(&String, &Element)> = node.children.iter().collect();
        children.sort_by_key(|(_, element)| element.index);
        for (name, element) in children {
            let mut element_path = path.clone();
            element_path.push(name.clone());
            found.push(ElementRef {
                name: name.clone(),
                element: element.clone(),
                owner: structure.clone(),
                path: element_path,
            });
        }
        if let Some(base) = node
            .types
            .first()
            .and_then(|code| self.definitions.structure(&type_url(code, &structure.url)))
        {
            self.collect_elements(base, &mut found);
        }
        found
    }

    fn collect_elements(&self, structure: Arc<Structure>, found: &mut Vec<ElementRef>) {
        let mut current = Some(structure);
        let mut depth = 0;
        while let Some(structure) = current.take() {
            if structure.kind == Kind::PrimitiveType && !found.iter().any(|e| e.name == "value") {
                found.push(ElementRef {
                    name: "value".to_owned(),
                    element: Element::default(),
                    owner: structure.clone(),
                    path: vec!["value".to_owned()],
                });
            }
            let mut elements: Vec<(&String, &Element)> = structure.elements.iter().collect();
            elements.sort_by_key(|(_, element)| element.index);
            for (name, element) in elements {
                if found.iter().any(|e| &e.name == name) {
                    continue;
                }
                found.push(ElementRef {
                    name: name.clone(),
                    element: element.clone(),
                    owner: structure.clone(),
                    path: vec![name.clone()],
                });
            }
            depth += 1;
            if depth > 16 {
                break;
            }
            current = structure
                .base
                .as_deref()
                .and_then(|base| self.definitions.structure(base));
        }
    }

    fn lookup_in_structure(&self, structure: Arc<Structure>, name: &str, depth: usize) -> Lookup {
        // A primitive's value: `string.value` in the StructureDefinition, not a separate element
        // in schemas that store primitives as plain JSON values.
        if structure.kind == Kind::PrimitiveType && name == "value" {
            return Lookup::Found(Box::new(ElementRef {
                name: name.to_owned(),
                element: Element::default(),
                owner: structure.clone(),
                path: vec![name.to_owned()],
            }));
        }
        if let Some(element) = structure.elements.get(name) {
            return Lookup::Found(Box::new(ElementRef {
                name: name.to_owned(),
                element: element.clone(),
                owner: structure.clone(),
                path: vec![name.to_owned()],
            }));
        }
        let Some(base) = &structure.base else {
            return Lookup::Missing;
        };
        if depth > 16 {
            return Lookup::Unknown;
        }
        match self.definitions.structure(base) {
            Some(base) => self.lookup_in_structure(base, name, depth + 1),
            None => Lookup::Unknown,
        }
    }

    /// The element at `path` inside `structure`, following content references.
    fn element_at(&self, structure: &Structure, path: &[String]) -> Option<Element> {
        let (first, rest) = path.split_first()?;
        let mut current = self.resolve_reference(structure.elements.get(first)?)?;
        for segment in rest {
            current = self.resolve_reference(current.children.get(segment)?)?;
        }
        Some(current)
    }

    /// The element itself, or the element its content reference points to.
    fn resolve_reference(&self, element: &Element) -> Option<Element> {
        match &element.content_reference {
            Some((url, target)) => {
                let referenced = self.definitions.structure(url)?;
                self.element_at(&referenced, target)
            }
            None => Some(element.clone()),
        }
    }

    /// The type of an element: its backbone, the content it references, or its type. `chosen`
    /// picks the type of a choice element.
    pub fn element_type(&self, element: &ElementRef, chosen: Option<&str>) -> Type {
        if let Some((url, target)) = &element.element.content_reference {
            return match self.definitions.structure(url) {
                Some(structure) => Type::Node {
                    structure,
                    path: target.clone(),
                },
                None => Type::Unknown,
            };
        }
        if !element.element.children.is_empty() {
            // Children a snapshot lists for an element of a structure type (`typeId.root` under
            // an `II`) are that type's own: the element has the type. Children the type does not
            // have make it a backbone element.
            if let [code] = element.element.types.as_slice() {
                let typed = self.structure(&type_url(code, &element.owner.url));
                if matches!(&typed, Type::Node { structure, .. } if !structure.is_abstract)
                    && element
                        .element
                        .children
                        .keys()
                        .all(|name| matches!(self.lookup(&typed, name), Lookup::Found(_)))
                {
                    return typed;
                }
            }
            return Type::Node {
                structure: element.owner.clone(),
                path: element.path.clone(),
            };
        }
        let code = match (chosen, element.element.types.as_slice()) {
            (Some(chosen), _) => chosen.to_owned(),
            (None, [single]) => single.clone(),
            _ => return Type::Unknown,
        };
        self.structure(&type_url(&code, &element.owner.url))
    }

    /// The type `name` names for a value of `element`: one of the element's types, by structure
    /// name (`IVL_TS`), type name or URL, or a type next to them that derives from one (a `CE` in
    /// a `CD` element). This is how a logical model's value says its type (`xsi:type`).
    pub fn named_type(&self, element: &ElementRef, name: &str) -> Type {
        let candidates: Vec<String> = element
            .element
            .types
            .iter()
            .map(|code| type_url(code, &element.owner.url))
            .collect();
        for url in &candidates {
            if let Some(structure) = self.definitions.structure(url)
                && (structure.name == name
                    || structure.type_name == name
                    || url == name
                    || url.ends_with(&format!("/{name}")))
            {
                return Type::Node {
                    structure,
                    path: Vec::new(),
                };
            }
        }
        if name.contains("://") {
            return self.structure(name);
        }
        // Model URLs name types by id, where `IVL_TS` is `IVL-TS`.
        for url in &candidates {
            let Some((prefix, _)) = url.rsplit_once('/') else {
                continue;
            };
            for id in [name.to_owned(), name.replace('_', "-")] {
                if let Some(structure) = self.definitions.structure(&format!("{prefix}/{id}"))
                    && (structure.name == name || structure.type_name.ends_with(name))
                {
                    return Type::Node {
                        structure,
                        path: Vec::new(),
                    };
                }
            }
        }
        Type::Unknown
    }

    /// Whether `chosen` is one of `types`, or derives from one of them (`OperationOutcome` for an
    /// element of type `Resource`).
    pub fn is_one_of(&self, chosen: &str, types: &[String], context: &str) -> bool {
        let matches = |name: &str| {
            types
                .iter()
                .any(|t| same_primitive(t, name) || t == name || t.ends_with(&format!("/{name}")))
        };
        if matches(chosen) {
            return true;
        }
        let mut url = type_url(chosen, context);
        for _ in 0..16 {
            let Some(structure) = self.definitions.structure(&url) else {
                // Not a known structure: a type of another model, nothing to compare with.
                return !self.definitions.covers(&url);
            };
            if matches(&structure.type_name) {
                return true;
            }
            match &structure.base {
                Some(base) => url = base.clone(),
                None => return false,
            }
        }
        true
    }
}
