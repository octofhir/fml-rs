//! The checker: walks a map with the types of its variables and reports what cannot work.

use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet};

use fml_syntax::ast::{
    self, AstNode, Expr, Invocation, ParameterMode, QualifiedName, TransformValue,
};
use fml_syntax::{Diagnostic, Severity, SyntaxKind, SyntaxNode, TextRange};

use crate::definitions::{Definitions, type_url};
use crate::types::{Lookup, Type, Types};

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Source,
    Target,
}

#[derive(Debug, Clone)]
struct Variable {
    ty: Type,
    mode: Mode,
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
    fn types(&self) -> Types<'a> {
        Types::new(self.definitions)
    }

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
    fn parameter_type(&mut self, parameter: &ast::Parameter) -> Option<Type> {
        let annotation = parameter.type_annotation()?;
        let name = annotation.type_name()?;
        if let Some(url) = self.type_names.get(&name).cloned() {
            return Some(self.types().structure(&url));
        }
        let url = type_url(&name, "http://hl7.org/fhir/StructureDefinition/");
        if let Some(structure) = self.definitions.structure(&url) {
            return Some(Type::Node {
                structure,
                path: Vec::new(),
            });
        }
        if self.definitions.covers(&url) {
            self.error(annotation.syntax(), format!("unknown type `{name}`"));
        }
        Some(Type::Unknown)
    }

    // ---- Groups -----------------------------------------------------------------------------

    /// Checks a group with argument types: declared types, or for untyped parameters the types
    /// inferred at a call site (`None` when not known).
    fn check_group(&mut self, group: &ast::Group, arguments: Vec<Option<Type>>, depth: usize) {
        let name = group.name().unwrap_or_default();
        let parameters: Vec<ast::Parameter> = group.parameters().collect();
        let mut scope = Scope::default();
        for constant in self.file.constants() {
            if let Some(constant_name) = constant.name() {
                scope.define(
                    constant_name,
                    Variable {
                        ty: Type::Unknown,
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
                .unwrap_or(Type::Unknown);
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
        if let Some(default) = source.default_value().and_then(|d| d.expression()) {
            self.check_expression(&default, &Type::Unknown, scope);
        }
        if let Some(alias) = source.alias() {
            scope.define(
                alias,
                Variable {
                    ty: ty.clone(),
                    mode: Mode::Source,
                },
            );
        }
        // `where`, `check` and `log` see each item as the focus, and its variable.
        for clause in [
            source.where_clause(),
            source.check_clause(),
            source.log_clause(),
        ]
        .into_iter()
        .flatten()
        {
            self.check_expression(&clause, &ty, scope);
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
                if let Lookup::Missing = self.types().lookup(&variable.ty, field) {
                    let message = format!("`{}` has no element `{field}`", variable.ty.display());
                    self.error(rule.syntax(), message);
                }
            }
        }
    }

    fn check_target(&mut self, target: &ast::Target, scope: &mut Scope) {
        let mut ty = Type::Unknown;
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
    fn check_transform(
        &mut self,
        transform: &TransformValue,
        element: Type,
        scope: &Scope,
    ) -> Type {
        match transform {
            TransformValue::Name(name) => {
                let segments = name.segments();
                match scope.get(&segments[0]).cloned() {
                    Some(variable) if segments.len() == 1 => {
                        if matches!(element, Type::Unknown) {
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
            TransformValue::Expression(expression) => {
                self.check_expression(expression, &Type::Unknown, scope);
                element
            }
            TransformValue::Literal(_) => element,
        }
    }

    fn check_invocation(&mut self, invocation: &Invocation, element: Type, scope: &Scope) -> Type {
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
        if name == "evaluate"
            && let [focus, expression] = arguments.as_slice()
        {
            let focus = scope
                .get(&focus.text())
                .map(|v| v.ty.clone())
                .unwrap_or(Type::Unknown);
            self.check_expression(expression, &focus, scope);
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
    fn created_type(&mut self, type_name: &str, element: &Type, at: &Expr) -> Type {
        let url = match (self.type_names.get(type_name), element) {
            (Some(url), _) => url.clone(),
            (None, Type::Node { structure, .. }) => type_url(type_name, &structure.url),
            // The element's type, and so the FHIR release the created type belongs to, is unknown.
            (None, Type::Unknown) => return Type::Unknown,
        };
        match self.definitions.structure(&url) {
            Some(structure) => {
                if structure.is_abstract {
                    self.error(
                        at.syntax(),
                        format!("`{type_name}` is abstract and cannot be created"),
                    );
                }
                Type::Node {
                    structure,
                    path: Vec::new(),
                }
            }
            None => {
                if self.definitions.covers(&url) {
                    self.error(at.syntax(), format!("unknown type `{type_name}`"));
                }
                Type::Unknown
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

    // ---- FHIRPath ---------------------------------------------------------------------------

    /// Checks the element paths of a FHIRPath expression: a name that is not a variable is an
    /// element of `focus`. Problems are warnings, since a wrong path gives an empty result rather
    /// than an error, but it is nearly always a mistake.
    fn check_expression(&mut self, expression: &Expr, focus: &Type, scope: &Scope) {
        self.expression_type(expression.syntax(), focus, scope);
    }

    /// The type an expression gives, where it is known, reporting missing elements on the way.
    fn expression_type(&mut self, node: &SyntaxNode, focus: &Type, scope: &Scope) -> Type {
        match node.kind() {
            SyntaxKind::NameRef => {
                let Some(token) = node.first_token() else {
                    return Type::Unknown;
                };
                if token.kind() == SyntaxKind::DollarIdent {
                    return if token.text() == "$this" {
                        focus.clone()
                    } else {
                        Type::Unknown
                    };
                }
                let name = ast::name_text(&token);
                if let Some(variable) = scope.get(&name) {
                    return variable.ty.clone();
                }
                // `Patient.name` on a Patient: the type name selects the focus itself.
                if focus.fhir_type().as_deref() == Some(name.as_str()) {
                    return focus.clone();
                }
                self.element_of(focus, &name, node)
            }
            SyntaxKind::MemberExpr => {
                let mut parts = node.children();
                let (Some(left), Some(right)) = (parts.next(), parts.next()) else {
                    return Type::Unknown;
                };
                let input = self.expression_type(&left, focus, scope);
                match right.kind() {
                    SyntaxKind::NameRef => match right.first_token() {
                        Some(token) if token.kind() != SyntaxKind::DollarIdent => {
                            self.element_of(&input, &ast::name_text(&token), &right)
                        }
                        _ => Type::Unknown,
                    },
                    SyntaxKind::CallExpr => self.call_type(&right, &input, scope),
                    _ => Type::Unknown,
                }
            }
            SyntaxKind::CallExpr => self.call_type(node, focus, scope),
            SyntaxKind::ParenExpr => match node.children().next() {
                Some(inner) => self.expression_type(&inner, focus, scope),
                None => Type::Unknown,
            },
            SyntaxKind::IndexExpr => {
                let mut parts = node.children();
                let input = match parts.next() {
                    Some(left) => self.expression_type(&left, focus, scope),
                    None => Type::Unknown,
                };
                if let Some(index) = parts.next() {
                    self.expression_type(&index, focus, scope);
                }
                input
            }
            SyntaxKind::BinaryExpr | SyntaxKind::PrefixExpr | SyntaxKind::TypeExpr => {
                for child in node.children().filter(|c| Expr::can_cast(c.kind())) {
                    self.expression_type(&child, focus, scope);
                }
                Type::Unknown
            }
            _ => Type::Unknown,
        }
    }

    fn element_of(&mut self, ty: &Type, name: &str, at: &SyntaxNode) -> Type {
        match self.types().lookup(ty, name) {
            Lookup::Found(found) => self.types().element_type(&found, None),
            Lookup::Missing => {
                let message = format!("`{}` has no element `{name}`", ty.display());
                self.report(Severity::Warning, trimmed_range(at), message);
                Type::Unknown
            }
            Lookup::Unknown => Type::Unknown,
        }
    }

    /// A function call on `input`. The iterating functions see each input item as the focus of
    /// their argument; the arguments of the others are left unchecked.
    fn call_type(&mut self, call: &SyntaxNode, input: &Type, scope: &Scope) -> Type {
        let name = call
            .first_token()
            .map(|t| t.text().to_owned())
            .unwrap_or_default();
        let arguments: Vec<SyntaxNode> = call
            .children()
            .find(|n| n.kind() == SyntaxKind::ArgumentList)
            .map(|list| {
                list.children()
                    .filter(|c| Expr::can_cast(c.kind()))
                    .collect()
            })
            .unwrap_or_default();
        let iterating = matches!(
            name.as_str(),
            "where" | "select" | "exists" | "all" | "repeat" | "sort" | "aggregate"
        );
        let argument_focus = if iterating {
            input.clone()
        } else {
            Type::Unknown
        };
        let mut last = Type::Unknown;
        for argument in &arguments {
            last = self.expression_type(argument, &argument_focus, scope);
        }
        match name.as_str() {
            "where" | "first" | "last" | "tail" | "skip" | "take" | "single" | "distinct"
            | "trace" | "repeat" | "sort" => input.clone(),
            "select" => last,
            _ => Type::Unknown,
        }
    }

    /// Follows `context.a.b` from the context variable's type, reporting missing elements, and
    /// returns the type at the end. `annotation` is the source's `: Type`, which chooses a type of
    /// a choice element.
    fn walk(
        &mut self,
        path: &QualifiedName,
        mut ty: Type,
        annotation: Option<&str>,
        annotation_node: Option<&ast::TypeAnnotation>,
    ) -> Type {
        let segments = path.segments();
        for (index, segment) in segments.iter().enumerate().skip(1) {
            let last = index == segments.len() - 1;
            let segment = segment.trim_end_matches("[x]");
            ty = match self.types().lookup(&ty, segment) {
                Lookup::Unknown => return Type::Unknown,
                Lookup::Missing => {
                    let message = format!("`{}` has no element `{segment}`", ty.display());
                    self.error_at(segment_range(path, index), message);
                    return Type::Unknown;
                }
                Lookup::Found(found) => {
                    let chosen = if last { annotation } else { None };
                    if let (Some(chosen), Some(node)) = (chosen, annotation_node)
                        && !found.element.types.is_empty()
                        && !self
                            .types()
                            .is_one_of(chosen, &found.element.types, &found.owner.url)
                    {
                        self.error(
                            node.syntax(),
                            format!(
                                "`{chosen}` is not a type of `{}.{segment}`; it is {}",
                                ty.display(),
                                found.element.types.join(", ")
                            ),
                        );
                    }
                    self.types().element_type(&found, chosen)
                }
            };
        }
        ty
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
    ("toDateTime", 1, Some(2)),
    ("unixToDateTime", 1, Some(2)),
    ("toDate", 1, Some(2)),
    ("unixToDate", 1, Some(2)),
    ("toTime", 1, Some(2)),
    ("unixToTime", 1, Some(2)),
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
