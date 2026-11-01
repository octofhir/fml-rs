//! The checker: walks a map with the types of its variables and reports what cannot work.

use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use fml_syntax::ast::{
    self, AstNode, Expr, Invocation, ParameterMode, QualifiedName, TransformValue,
};
use fml_syntax::{Diagnostic, Severity, SyntaxKind, SyntaxNode, TextRange};

use crate::definitions::{Definitions, Element, Kind, Structure, same_primitive, type_url};

/// Checks a parsed map. Syntax errors are not repeated; see [`fml_syntax::Parse::diagnostics`].
pub fn check(file: &ast::SourceFile, definitions: &dyn Definitions) -> Vec<Diagnostic> {
    let mut checker = Checker::new(file, definitions);
    checker.run();
    let mut diagnostics = checker.diagnostics;
    diagnostics.sort_by_key(|d| (d.range.start(), d.range.end()));
    diagnostics
}

/// How deep untyped groups are followed into the groups they call.
const MAX_CALL_DEPTH: usize = 32;

/// The type of a variable or element.
#[derive(Debug, Clone)]
enum Ty {
    /// Not known: no definition, a choice without a type, an expression result.
    Unknown,
    /// A structure, or a backbone element inside it (the element path).
    Node {
        structure: Arc<Structure>,
        path: Vec<String>,
    },
}

impl Ty {
    fn key(&self) -> String {
        match self {
            Ty::Unknown => "?".to_owned(),
            Ty::Node { structure, path } => {
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
    fn display(&self) -> String {
        match self {
            Ty::Unknown => "?".to_owned(),
            Ty::Node { structure, path } => {
                let mut name = structure.type_name.clone();
                for segment in path {
                    name.push('.');
                    name.push_str(segment);
                }
                name
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Source,
    Target,
}

#[derive(Debug, Clone)]
struct Variable {
    ty: Ty,
    mode: Mode,
}

/// The result of looking up an element.
enum Lookup {
    Found {
        element: Element,
        /// The structure that defines the element, and the path to the element itself.
        owner: Arc<Structure>,
        path: Vec<String>,
    },
    Missing,
    Unknown,
}

struct Checker<'a> {
    file: &'a ast::SourceFile,
    definitions: &'a dyn Definitions,
    /// `uses` aliases and the names of unaliased structures, to their URLs.
    type_names: HashMap<String, String>,
    groups: HashMap<String, ast::Group>,
    has_imports: bool,
    /// Groups already checked with given argument types.
    checked: HashSet<(String, Vec<String>)>,
    reported: HashSet<(TextRange, String)>,
    diagnostics: Vec<Diagnostic>,
}

impl<'a> Checker<'a> {
    fn new(file: &'a ast::SourceFile, definitions: &'a dyn Definitions) -> Self {
        Self {
            file,
            definitions,
            type_names: HashMap::new(),
            groups: HashMap::new(),
            has_imports: file.imports().next().is_some(),
            checked: HashSet::new(),
            reported: HashSet::new(),
            diagnostics: Vec::new(),
        }
    }

    fn report(&mut self, severity: Severity, range: TextRange, message: String) {
        if self.reported.insert((range, message.clone())) {
            self.diagnostics.push(Diagnostic {
                severity,
                range,
                message,
            });
        }
    }

    fn error(&mut self, node: &SyntaxNode, message: impl Into<String>) {
        self.report(Severity::Error, trimmed_range(node), message.into());
    }

    fn error_at(&mut self, range: TextRange, message: impl Into<String>) {
        self.report(Severity::Error, range, message.into());
    }

    fn run(&mut self) {
        self.declarations();
        let groups: Vec<ast::Group> = self.file.groups().collect();
        for group in &groups {
            let types = group
                .parameters()
                .map(|parameter| self.parameter_type(&parameter))
                .collect();
            self.check_group(group, types, 0);
        }
    }

    // ---- Declarations -----------------------------------------------------------------------

    fn declarations(&mut self) {
        for uses in self.file.uses() {
            let Some(url) = uses.url().and_then(|u| u.value()) else {
                continue;
            };
            match self.definitions.structure(&url) {
                Some(structure) => {
                    let name = uses.alias().unwrap_or_else(|| structure.name.clone());
                    self.type_names.insert(name, url);
                }
                None => {
                    if self.definitions.covers(&url) {
                        let at = uses.url().unwrap();
                        self.error(at.syntax(), format!("unknown structure `{url}`"));
                    }
                    if let Some(alias) = uses.alias() {
                        self.type_names.insert(alias, url);
                    }
                }
            }
            if uses.mode().is_none() {
                self.error(
                    uses.syntax(),
                    "`uses` needs a mode: `as source` or `as target`",
                );
            }
        }
        for group in self.file.groups() {
            let Some(name) = group.name() else { continue };
            match self.groups.entry(name) {
                Entry::Occupied(entry) => {
                    let message = format!("group `{}` is declared twice", entry.key());
                    self.error(group.syntax(), message);
                }
                Entry::Vacant(entry) => {
                    entry.insert(group);
                }
            }
        }
        for group in self.file.groups() {
            if let Some(extends) = group.extends()
                && !self.groups.contains_key(&extends)
                && !self.has_imports
            {
                self.error(
                    group.syntax(),
                    format!("group `{extends}` is not declared and nothing is imported"),
                );
            }
        }
    }

    /// The declared type of a group parameter: a `uses` alias, the name of a used structure, or a
    /// core type name.
    fn parameter_type(&mut self, parameter: &ast::Parameter) -> Option<Ty> {
        let annotation = parameter.type_annotation()?;
        let name = annotation.type_name()?;
        if let Some(url) = self.type_names.get(&name).cloned() {
            return Some(self.structure_ty(&url));
        }
        let url = type_url(&name, "http://hl7.org/fhir/StructureDefinition/");
        if let Some(structure) = self.definitions.structure(&url) {
            return Some(Ty::Node {
                structure,
                path: Vec::new(),
            });
        }
        if self.definitions.covers(&url) {
            self.error(annotation.syntax(), format!("unknown type `{name}`"));
        }
        Some(Ty::Unknown)
    }

    fn structure_ty(&self, url: &str) -> Ty {
        match self.definitions.structure(url) {
            Some(structure) => Ty::Node {
                structure,
                path: Vec::new(),
            },
            None => Ty::Unknown,
        }
    }

    // ---- Groups -----------------------------------------------------------------------------

    /// Checks a group with argument types: declared types, or for untyped parameters the types
    /// inferred at a call site (`None` when not known).
    fn check_group(&mut self, group: &ast::Group, arguments: Vec<Option<Ty>>, depth: usize) {
        let name = group.name().unwrap_or_default();
        let parameters: Vec<ast::Parameter> = group.parameters().collect();
        let mut scope = Scope::default();
        for constant in self.file.constants() {
            if let Some(constant_name) = constant.name() {
                scope.define(
                    constant_name,
                    Variable {
                        ty: Ty::Unknown,
                        mode: Mode::Source,
                    },
                );
            }
        }
        scope.push();
        let mut key = Vec::new();
        for (index, parameter) in parameters.iter().enumerate() {
            let declared = self.parameter_type(parameter);
            let ty = declared
                .or_else(|| arguments.get(index).cloned().flatten())
                .unwrap_or(Ty::Unknown);
            key.push(ty.key());
            let mode = match parameter.mode() {
                Some(ParameterMode::Target) => Mode::Target,
                _ => Mode::Source,
            };
            if let Some(parameter_name) = parameter.name() {
                scope.define(parameter_name, Variable { ty, mode });
            }
        }
        if !self.checked.insert((name, key)) || depth > MAX_CALL_DEPTH {
            return;
        }
        if let Some(body) = group.body() {
            for rule in body.rules() {
                self.check_rule(&rule, &mut scope, depth);
            }
        }
    }

    // ---- Rules ------------------------------------------------------------------------------

    fn check_rule(&mut self, rule: &ast::Rule, outer: &mut Scope, depth: usize) {
        outer.push();
        for source in rule.sources() {
            self.check_source(&source, outer);
        }
        let fields = rule.identity_fields();
        let targets: Vec<ast::Target> = rule.targets().collect();
        if !fields.is_empty() {
            self.check_identity(rule, &fields, outer);
        }
        for target in &targets {
            self.check_target(target, outer);
        }
        if let Some(dependent) = rule.dependent() {
            for invocation in dependent.invocations() {
                self.check_call(&invocation, outer, depth);
            }
            if let Some(block) = dependent.rules() {
                for nested in block.rules() {
                    self.check_rule(&nested, outer, depth);
                }
            }
        }
        outer.pop();
    }

    fn check_source(&mut self, source: &ast::Source, scope: &mut Scope) {
        let Some(path) = source.path() else { return };
        let segments = path.segments();
        let Some(context) = segments.first() else {
            return;
        };
        let Some(variable) = scope.get(context).cloned() else {
            self.error_at(
                segment_range(&path, 0),
                format!("unknown variable `{context}`"),
            );
            return;
        };
        let annotation = source.type_annotation();
        let annotated = annotation.as_ref().and_then(|a| a.type_name());
        let ty = self.walk(
            &path,
            variable.ty,
            annotated.as_deref(),
            annotation.as_ref(),
        );
        if let Some(alias) = source.alias() {
            scope.define(
                alias,
                Variable {
                    ty,
                    mode: Mode::Source,
                },
            );
        }
    }

    /// `src -> tgt : a, b;`: each field exists on both sides.
    fn check_identity(&mut self, rule: &ast::Rule, fields: &[String], scope: &Scope) {
        let source = rule.sources().next().and_then(|s| s.path());
        let target = rule.targets().next().and_then(|t| t.path());
        for path in [source, target].into_iter().flatten() {
            let Some(variable) = path.segments().first().and_then(|c| scope.get(c)).cloned() else {
                continue;
            };
            for field in fields {
                if let Lookup::Missing = self.lookup(&variable.ty, field) {
                    let message = format!("`{}` has no element `{field}`", variable.ty.display());
                    self.error(rule.syntax(), message);
                }
            }
        }
    }

    fn check_target(&mut self, target: &ast::Target, scope: &mut Scope) {
        let mut ty = Ty::Unknown;
        if let Some(path) = target.path() {
            let segments = path.segments();
            let context = &segments[0];
            match scope.get(context).cloned() {
                None => {
                    self.error_at(
                        segment_range(&path, 0),
                        format!("unknown variable `{context}`"),
                    );
                }
                Some(variable) => {
                    if variable.mode == Mode::Source {
                        self.error_at(
                            segment_range(&path, 0),
                            format!("`{context}` is a source variable; a target writes into a target variable"),
                        );
                    }
                    ty = self.walk(&path, variable.ty, None, None);
                }
            }
        }
        if let Some(transform) = target.transform() {
            ty = self.check_transform(&transform, ty, scope);
        }
        if let Some(alias) = target.alias() {
            scope.define(
                alias,
                Variable {
                    ty,
                    mode: Mode::Target,
                },
            );
        }
    }

    /// Checks a transform; returns the type of the value it produces where that is known.
    fn check_transform(&mut self, transform: &TransformValue, element: Ty, scope: &Scope) -> Ty {
        match transform {
            TransformValue::Name(name) => {
                let segments = name.segments();
                match scope.get(&segments[0]).cloned() {
                    Some(variable) if segments.len() == 1 => {
                        if matches!(element, Ty::Unknown) {
                            variable.ty
                        } else {
                            element
                        }
                    }
                    Some(variable) => {
                        self.walk(name, variable.ty, None, None);
                        element
                    }
                    None => {
                        self.error_at(
                            segment_range(name, 0),
                            format!("unknown variable `{}`", segments[0]),
                        );
                        element
                    }
                }
            }
            TransformValue::Invocation(invocation) => {
                self.check_invocation(invocation, element, scope)
            }
            TransformValue::Literal(_) | TransformValue::Expression(_) => element,
        }
    }

    fn check_invocation(&mut self, invocation: &Invocation, element: Ty, scope: &Scope) -> Ty {
        let name = invocation.name().unwrap_or_default();
        let arguments: Vec<Expr> = invocation.arguments().collect();
        let Some(&(_, min, max)) = TRANSFORMS.iter().find(|(n, _, _)| *n == name) else {
            self.error(invocation.syntax(), format!("unknown transform `{name}`"));
            return element;
        };
        if arguments.len() < min || max.is_some_and(|max| arguments.len() > max) {
            let expected = match max {
                Some(max) if max == min => format!("{min}"),
                Some(max) => format!("{min} to {max}"),
                None => format!("at least {min}"),
            };
            self.error(
                invocation.syntax(),
                format!(
                    "`{name}` takes {expected} argument(s), found {}",
                    arguments.len()
                ),
            );
        }
        for (index, argument) in arguments.iter().enumerate() {
            let is_expression = name == "evaluate" && index == 1;
            if !is_expression && argument.syntax().kind() == SyntaxKind::NameRef {
                let variable = argument.text();
                if scope.get(&variable).is_none() {
                    self.error(argument.syntax(), format!("unknown variable `{variable}`"));
                }
            }
        }
        if name == "create"
            && let Some(type_name) = arguments.first().and_then(string_literal)
        {
            return self.created_type(&type_name, &element, &arguments[0]);
        }
        element
    }

    /// The type `create('Type')` makes, checked against the element it is assigned to.
    fn created_type(&mut self, type_name: &str, element: &Ty, at: &Expr) -> Ty {
        let url = match (self.type_names.get(type_name), element) {
            (Some(url), _) => url.clone(),
            (None, Ty::Node { structure, .. }) => type_url(type_name, &structure.url),
            // The element's type, and so the FHIR release the created type belongs to, is unknown.
            (None, Ty::Unknown) => return Ty::Unknown,
        };
        match self.definitions.structure(&url) {
            Some(structure) => {
                if structure.is_abstract {
                    self.error(
                        at.syntax(),
                        format!("`{type_name}` is abstract and cannot be created"),
                    );
                }
                Ty::Node {
                    structure,
                    path: Vec::new(),
                }
            }
            None => {
                if self.definitions.covers(&url) {
                    self.error(at.syntax(), format!("unknown type `{type_name}`"));
                }
                Ty::Unknown
            }
        }
    }

    /// `then Group(a, b)`: the group exists, takes that many arguments, and is checked with the
    /// argument types when its parameters are untyped.
    fn check_call(&mut self, invocation: &Invocation, scope: &Scope, depth: usize) {
        let name = invocation.name().unwrap_or_default();
        let arguments: Vec<Expr> = invocation.arguments().collect();
        let mut types = Vec::new();
        for argument in &arguments {
            if argument.syntax().kind() == SyntaxKind::NameRef {
                let variable = argument.text();
                match scope.get(&variable) {
                    Some(v) => types.push(Some(v.ty.clone())),
                    None => {
                        self.error(argument.syntax(), format!("unknown variable `{variable}`"));
                        types.push(None);
                    }
                }
            } else {
                types.push(None);
            }
        }
        let Some(group) = self.groups.get(&name).cloned() else {
            if !self.has_imports {
                self.error(
                    invocation.syntax(),
                    format!("group `{name}` is not declared and nothing is imported"),
                );
            }
            return;
        };
        let parameters = group.parameters().count();
        if parameters != arguments.len() {
            self.error(
                invocation.syntax(),
                format!(
                    "group `{name}` takes {parameters} argument(s), found {}",
                    arguments.len()
                ),
            );
            return;
        }
        self.check_group(&group, types, depth + 1);
    }

    // ---- Element paths ----------------------------------------------------------------------

    /// Follows `context.a.b` from the context variable's type, reporting missing elements, and
    /// returns the type at the end. `annotation` is the source's `: Type`, which chooses a type of
    /// a choice element.
    fn walk(
        &mut self,
        path: &QualifiedName,
        mut ty: Ty,
        annotation: Option<&str>,
        annotation_node: Option<&ast::TypeAnnotation>,
    ) -> Ty {
        let segments = path.segments();
        for (index, segment) in segments.iter().enumerate().skip(1) {
            let last = index == segments.len() - 1;
            let segment = segment.trim_end_matches("[x]");
            ty = match self.lookup(&ty, segment) {
                Lookup::Unknown => return Ty::Unknown,
                Lookup::Missing => {
                    let message = format!("`{}` has no element `{segment}`", ty.display());
                    self.error_at(segment_range(path, index), message);
                    return Ty::Unknown;
                }
                Lookup::Found {
                    element,
                    owner,
                    path: element_path,
                } => {
                    let chosen = if last { annotation } else { None };
                    if let (Some(chosen), Some(node)) = (chosen, annotation_node)
                        && !element.types.is_empty()
                        && !self.is_one_of(chosen, &element.types, &owner.url)
                    {
                        self.error(
                            node.syntax(),
                            format!(
                                "`{chosen}` is not a type of `{}.{segment}`; it is {}",
                                ty.display(),
                                element.types.join(", ")
                            ),
                        );
                    }
                    self.element_ty(&element, &owner, element_path, chosen)
                }
            };
        }
        ty
    }

    /// Whether `chosen` is one of `types`, or derives from one of them (`OperationOutcome` for an
    /// element of type `Resource`).
    fn is_one_of(&self, chosen: &str, types: &[String], context: &str) -> bool {
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

    fn lookup(&self, ty: &Ty, name: &str) -> Lookup {
        let Ty::Node { structure, path } = ty else {
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
            return Lookup::Found {
                element: element.clone(),
                owner: structure.clone(),
                path: element_path,
            };
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

    fn lookup_in_structure(&self, structure: Arc<Structure>, name: &str, depth: usize) -> Lookup {
        // A primitive's value: `string.value` in the StructureDefinition, not a separate element
        // in schemas that store primitives as plain JSON values.
        if structure.kind == Kind::PrimitiveType && name == "value" {
            return Lookup::Found {
                element: Element::default(),
                owner: structure.clone(),
                path: vec![name.to_owned()],
            };
        }
        if let Some(element) = structure.elements.get(name) {
            return Lookup::Found {
                element: element.clone(),
                owner: structure.clone(),
                path: vec![name.to_owned()],
            };
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
        let mut elements = structure.elements.clone();
        let mut current: Option<Element> = None;
        for segment in path {
            let element = elements.get(segment)?.clone();
            let element = match &element.content_reference {
                Some((url, target)) => {
                    let referenced = self.definitions.structure(url)?;
                    self.element_at(&referenced, target)?
                }
                None => element,
            };
            elements = element.children.clone();
            current = Some(element);
        }
        current
    }

    /// The type of an element: its backbone, the content it references, or its (chosen) type.
    fn element_ty(
        &self,
        element: &Element,
        owner: &Arc<Structure>,
        path: Vec<String>,
        chosen: Option<&str>,
    ) -> Ty {
        if let Some((url, target)) = &element.content_reference {
            return match self.definitions.structure(url) {
                Some(structure) => Ty::Node {
                    structure,
                    path: target.clone(),
                },
                None => Ty::Unknown,
            };
        }
        if !element.children.is_empty() {
            return Ty::Node {
                structure: owner.clone(),
                path,
            };
        }
        let code = match (chosen, element.types.as_slice()) {
            (Some(chosen), _) => chosen.to_owned(),
            (None, [single]) => single.clone(),
            _ => return Ty::Unknown,
        };
        self.structure_ty(&type_url(&code, &owner.url))
    }
}

/// Transforms with their argument counts (minimum, maximum).
const TRANSFORMS: &[(&str, usize, Option<usize>)] = &[
    ("create", 0, Some(1)),
    ("copy", 1, Some(1)),
    ("truncate", 2, Some(2)),
    ("escape", 2, Some(3)),
    ("cast", 1, Some(2)),
    ("append", 1, None),
    ("translate", 3, Some(3)),
    ("reference", 1, Some(1)),
    ("dateOp", 2, Some(3)),
    ("uuid", 0, Some(0)),
    ("pointer", 1, Some(1)),
    ("evaluate", 1, Some(2)),
    ("cc", 1, Some(3)),
    ("c", 2, Some(3)),
    ("qty", 1, Some(4)),
    ("id", 2, Some(3)),
    ("cp", 1, Some(2)),
];

/// Variables by name, in nested frames: group parameters, then one frame per rule.
#[derive(Debug, Default)]
struct Scope {
    frames: Vec<HashMap<String, Variable>>,
}

impl Scope {
    fn push(&mut self) {
        self.frames.push(HashMap::new());
    }

    fn pop(&mut self) {
        self.frames.pop();
    }

    fn define(&mut self, name: String, variable: Variable) {
        if self.frames.is_empty() {
            self.frames.push(HashMap::new());
        }
        self.frames.last_mut().unwrap().insert(name, variable);
    }

    fn get(&self, name: &str) -> Option<&Variable> {
        self.frames.iter().rev().find_map(|frame| frame.get(name))
    }
}

/// A string literal argument's value.
fn string_literal(expression: &Expr) -> Option<String> {
    let literal = ast::Literal::cast(expression.syntax().clone())?;
    matches!(
        literal.kind()?,
        SyntaxKind::SingleQuotedString | SyntaxKind::DoubleQuotedString
    )
    .then(|| literal.value())
    .flatten()
}

/// The range of the `index`-th name in `a.b.c`.
fn segment_range(path: &QualifiedName, index: usize) -> TextRange {
    path.syntax()
        .children_with_tokens()
        .filter_map(|e| e.into_token())
        .filter(|t| matches!(t.kind(), SyntaxKind::Ident | SyntaxKind::DelimitedIdent))
        .nth(index)
        .map(|t| t.text_range())
        .unwrap_or_else(|| trimmed_range(path.syntax()))
}

fn trimmed_range(node: &SyntaxNode) -> TextRange {
    let mut tokens = node
        .descendants_with_tokens()
        .filter_map(|e| e.into_token())
        .filter(|t| !t.kind().is_trivia());
    match tokens.next() {
        Some(first) => {
            let end = tokens
                .last()
                .map_or(first.text_range().end(), |t| t.text_range().end());
            TextRange::new(first.text_range().start(), end)
        }
        None => node.text_range(),
    }
}
