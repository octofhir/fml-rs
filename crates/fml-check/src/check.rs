//! The checker: walks a map with the types of its variables and reports what cannot work.

use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet};

use fml_syntax::ast::{
    self, AstNode, Expr, Invocation, ParameterMode, QualifiedName, TransformValue,
};
use fml_syntax::{Diagnostic, Severity, SyntaxKind, SyntaxNode, SyntaxToken, TextRange, TextSize};

use crate::analysis::{Analysis, Binding, BindingKind, Reference, Referent};
use crate::definitions::{Definitions, Kind, type_url};
use crate::types::{Lookup, Type, Types};

/// Checks a parsed map. Syntax errors are not repeated; see [`fml_syntax::Parse::diagnostics`].
pub fn check(file: &ast::SourceFile, definitions: &dyn Definitions) -> Vec<Diagnostic> {
    analyze(file, definitions).diagnostics
}

/// Checks a parsed map and returns, with the problems, its variables and what each name refers
/// to. Works on maps with syntax errors too, as far as their tree goes.
pub fn analyze(file: &ast::SourceFile, definitions: &dyn Definitions) -> Analysis {
    let mut checker = Checker::new(file, definitions);
    checker.run();
    let mut diagnostics = checker.diagnostics;
    diagnostics.sort_by_key(|d| (d.range.start(), d.range.end()));
    let mut references: Vec<Reference> = checker.references.into_values().collect();
    references.sort_by_key(|r| (r.range.start(), r.range.end()));
    Analysis {
        diagnostics,
        bindings: checker.bindings,
        references,
    }
}

/// The reserved keywords of the mapping language (fml.md, "Reserved keywords").
const RESERVED: &[&str] = &[
    "map",
    "uses",
    "as",
    "alias",
    "imports",
    "group",
    "extends",
    "default",
    "where",
    "check",
    "log",
    "then",
    "true",
    "false",
    "types",
    "type",
    "first",
    "not_first",
    "last",
    "not_last",
    "only_one",
    "share",
    "single",
    "source",
    "target",
    "queried",
    "produced",
    "conceptmap",
    "prefix",
    "let",
];

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
    /// Its entry in the analysis.
    binding: Option<usize>,
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
    bindings: Vec<Binding>,
    /// Bindings by the range of their name: a group checked several times declares its
    /// variables once.
    binding_ids: HashMap<TextRange, usize>,
    references: HashMap<TextRange, Reference>,
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
            bindings: Vec::new(),
            binding_ids: HashMap::new(),
            references: HashMap::new(),
        }
    }

    // ---- Analysis ---------------------------------------------------------------------------

    /// Declares a variable named by `token`, visible in `visible`, and returns it for the scope.
    fn bind(
        &mut self,
        token: Option<SyntaxToken>,
        kind: BindingKind,
        visible: TextRange,
        ty: Type,
    ) -> Option<(String, Variable)> {
        let token = token?;
        let name = ast::name_text(&token);
        let range = token.text_range();
        let id = match self.binding_ids.entry(range) {
            Entry::Occupied(entry) => {
                let id = *entry.get();
                let binding = &mut self.bindings[id];
                if matches!(binding.ty, Type::Unknown) {
                    binding.ty = ty.clone();
                }
                id
            }
            Entry::Vacant(entry) => {
                entry.insert(self.bindings.len());
                self.bindings.push(Binding {
                    name: name.clone(),
                    kind,
                    definition: range,
                    visible,
                    ty: ty.clone(),
                });
                self.bindings.len() - 1
            }
        };
        let mode = if kind.is_target() {
            Mode::Target
        } else {
            Mode::Source
        };
        Some((
            name,
            Variable {
                ty,
                mode,
                binding: Some(id),
            },
        ))
    }

    /// Notes what the name at `range` refers to. A group checked again with known argument
    /// types fills in the types it did not know the first time.
    fn refer(&mut self, range: TextRange, referent: Referent, ty: &Type) {
        match self.references.entry(range) {
            Entry::Occupied(mut entry) => {
                if matches!(entry.get().ty, Type::Unknown) && !matches!(ty, Type::Unknown) {
                    entry.insert(Reference {
                        range,
                        referent,
                        ty: ty.clone(),
                    });
                }
            }
            Entry::Vacant(entry) => {
                entry.insert(Reference {
                    range,
                    referent,
                    ty: ty.clone(),
                });
            }
        }
    }

    fn refer_variable(&mut self, range: TextRange, variable: &Variable) {
        if let Some(id) = variable.binding {
            self.refer(range, Referent::Binding(id), &variable.ty);
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

    fn warning(&mut self, node: &SyntaxNode, message: impl Into<String>) {
        self.report(Severity::Warning, trimmed_range(node), message.into());
    }

    fn error_at(&mut self, range: TextRange, message: impl Into<String>) {
        self.report(Severity::Error, range, message.into());
    }

    fn run(&mut self) {
        self.declarations();
        self.identity();
        self.names();
        self.group_signatures();
        self.rule_names();
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

    /// Constant names must be unique in the file, and no name the map defines may be a reserved
    /// keyword (fml.md, "Constants" and "Reserved keywords").
    fn names(&mut self) {
        let mut constants: HashSet<String> = HashSet::new();
        for constant in self.file.constants() {
            let Some(name) = constant.name() else {
                continue;
            };
            if !constants.insert(name.clone()) {
                self.error(
                    constant.syntax(),
                    format!("constant `{name}` is declared twice"),
                );
            }
            if RESERVED.contains(&name.as_str()) {
                self.warning(constant.syntax(), format!("`{name}` is a reserved keyword"));
            }
        }
        for group in self.file.groups() {
            if let Some(name) = group.name()
                && RESERVED.contains(&name.as_str())
            {
                self.warning(group.syntax(), format!("`{name}` is a reserved keyword"));
            }
            for parameter in group.parameters() {
                if let Some(name) = parameter.name()
                    && RESERVED.contains(&name.as_str())
                {
                    self.warning(
                        parameter.syntax(),
                        format!("`{name}` is a reserved keyword"),
                    );
                }
            }
        }
    }

    /// `url` and `name` SHALL be given (fml.md, "Metadata"), in metadata or a `map` line.
    fn identity(&mut self) {
        let declaration = self.file.map_declaration();
        let metadata: Vec<String> = self
            .file
            .metadata()
            .filter_map(|m| m.key().map(|k| k.text()))
            .collect();
        let has = |key: &str| metadata.iter().any(|k| k == key);
        let url = has("url") || declaration.as_ref().is_some_and(|d| d.url().is_some());
        let name = has("name") || declaration.as_ref().is_some_and(|d| d.name().is_some());
        let at = TextRange::empty(0.into());
        if !url {
            self.error_at(at, "the map has no `url`: add `/// url = '…'`");
        }
        if !name {
            self.error_at(at, "the map has no `name`: add `/// name = '…'`");
        }
    }

    /// The parameters a group declares, as (mode, name, type).
    fn signature(group: &ast::Group) -> Vec<(Mode, String, Option<String>)> {
        group
            .parameters()
            .map(|parameter| {
                let mode = match parameter.mode() {
                    Some(ParameterMode::Target) => Mode::Target,
                    _ => Mode::Source,
                };
                let type_name = parameter.type_annotation().and_then(|t| t.type_name());
                (mode, parameter.name().unwrap_or_default(), type_name)
            })
            .collect()
    }

    /// A group that extends another SHALL have the other's parameters, by mode, name and type
    /// (fml.md, "Groups"); a default group SHALL have a typed source and a typed target, in that
    /// order.
    fn group_signatures(&mut self) {
        let groups: Vec<ast::Group> = self.file.groups().collect();
        for group in &groups {
            let own = Self::signature(group);
            if let Some(base) = group
                .extends()
                .and_then(|name| self.groups.get(&name).cloned())
            {
                let base_name = base.name().unwrap_or_default();
                for (mode, name, type_name) in Self::signature(&base) {
                    let matching = own.iter().find(|(_, own_name, _)| *own_name == name);
                    let problem = match matching {
                        None => Some(format!("has no parameter `{name}`")),
                        Some((own_mode, _, _)) if *own_mode != mode => {
                            Some(format!("has `{name}` with another mode"))
                        }
                        Some((_, _, own_type))
                            if type_name.is_some()
                                && own_type.is_some()
                                && *own_type != type_name =>
                        {
                            Some(format!(
                                "has `{name}` as `{}`, not `{}`",
                                own_type.as_deref().unwrap_or_default(),
                                type_name.as_deref().unwrap_or_default()
                            ))
                        }
                        _ => None,
                    };
                    if let Some(problem) = problem {
                        self.error(
                            group.syntax(),
                            format!(
                                "group `{}` extends `{base_name}` but {problem}; it needs the parameters of `{base_name}`",
                                group.name().unwrap_or_default()
                            ),
                        );
                    }
                }
            }
            if group.type_mode().is_some() {
                let valid = matches!(
                    own.as_slice(),
                    [(Mode::Source, _, Some(_)), (Mode::Target, _, Some(_))]
                );
                if !valid {
                    self.warning(
                        group.syntax(),
                        format!(
                            "default group `{}` needs two typed parameters, a source and then a target; it is not used as a default",
                            group.name().unwrap_or_default()
                        ),
                    );
                }
            }
        }
    }

    /// Rule names given in the map must be unique within it (fml.md, "Transform Rules").
    fn rule_names(&mut self) {
        let mut rules: Vec<ast::Rule> = Vec::new();
        let mut pending: Vec<ast::Rule> = self
            .file
            .groups()
            .flat_map(|g| g.rules().collect::<Vec<_>>())
            .collect();
        while let Some(rule) = pending.pop() {
            if let Some(block) = rule.dependent().and_then(|d| d.rules()) {
                pending.extend(block.rules());
            }
            rules.push(rule);
        }
        rules.sort_by_key(|rule| rule.syntax().text_range().start());
        let mut seen: HashSet<String> = HashSet::new();
        for rule in rules {
            let Some(name) = rule.name() else {
                continue;
            };
            if !seen.insert(name.clone()) {
                self.warning(
                    rule.syntax(),
                    format!("rule name `{name}` is used twice in the map"),
                );
            }
        }
    }

    /// The declared type of a group parameter: a `uses` alias, the name of a used structure, or a
    /// core type name.
    fn parameter_type(&mut self, parameter: &ast::Parameter) -> Option<Type> {
        let annotation = parameter.type_annotation()?;
        let name = annotation.type_name()?;
        let ty = self.named_type(&annotation, &name);
        self.refer(
            annotation_range(&annotation),
            Referent::Type(ty.clone()),
            &ty,
        );
        Some(ty)
    }

    fn named_type(&mut self, annotation: &ast::TypeAnnotation, name: &str) -> Type {
        if let Some(url) = self.type_names.get(name).cloned() {
            return self.types().structure(&url);
        }
        let url = type_url(name, "http://hl7.org/fhir/StructureDefinition/");
        if let Some(structure) = self.definitions.structure(&url) {
            return Type::Node {
                structure,
                path: Vec::new(),
            };
        }
        if self.definitions.covers(&url) {
            self.error(annotation.syntax(), format!("unknown type `{name}`"));
        }
        Type::Unknown
    }

    // ---- Groups -----------------------------------------------------------------------------

    /// Checks a group with argument types: declared types, or for untyped parameters the types
    /// inferred at a call site (`None` when not known).
    fn check_group(&mut self, group: &ast::Group, arguments: Vec<Option<Type>>, depth: usize) {
        let name = group.name().unwrap_or_default();
        let parameters: Vec<ast::Parameter> = group.parameters().collect();
        let mut scope = Scope::default();
        let whole = self.file.syntax().text_range();
        for constant in self.file.constants() {
            if let Some((constant_name, variable)) = self.bind(
                constant.name_token(),
                BindingKind::Constant,
                whole,
                Type::Unknown,
            ) {
                scope.define(constant_name, variable);
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
            let kind = match parameter.mode() {
                Some(ParameterMode::Target) => BindingKind::TargetParameter,
                _ => BindingKind::SourceParameter,
            };
            if let Some((parameter_name, variable)) = self.bind(
                parameter.name_token(),
                kind,
                group.syntax().text_range(),
                ty,
            ) {
                scope.define(parameter_name, variable);
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
        let end = rule.syntax().text_range().end();
        for source in rule.sources() {
            self.check_source(&source, outer, end);
        }
        let fields = rule.identity_fields();
        let targets: Vec<ast::Target> = rule.targets().collect();
        if !fields.is_empty() {
            self.check_identity(rule, &fields, outer);
        }
        for target in &targets {
            self.check_target(target, outer, end);
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

    fn check_source(&mut self, source: &ast::Source, scope: &mut Scope, rule_end: TextSize) {
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
        self.refer_variable(segment_range(&path, 0), &variable);
        // A rule reads from source variables; a variable a target made is not one (the engine,
        // like the reference engine, looks the context up among the source variables).
        if variable.mode == Mode::Target {
            self.error_at(
                segment_range(&path, 0),
                format!("`{context}` is a target variable; a source reads from a source variable"),
            );
        }
        let annotation = source.type_annotation();
        let annotated = annotation.as_ref().and_then(|a| a.type_name());
        let ty = self.walk(
            &path,
            variable.ty,
            annotated.as_deref(),
            annotation.as_ref(),
        );
        if let Some(annotation) = &annotation {
            self.refer(
                annotation_range(annotation),
                Referent::Type(ty.clone()),
                &ty,
            );
        }
        if let Some(default) = source.default_value().and_then(|d| d.expression()) {
            self.check_expression(&default, &Type::Unknown, scope);
        }
        let token = source.alias_token();
        let visible = visible_until(token.as_ref(), rule_end);
        if let Some((alias, variable)) = self.bind(token, BindingKind::Source, visible, ty.clone())
        {
            scope.define(alias, variable);
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

    fn check_target(&mut self, target: &ast::Target, scope: &mut Scope, rule_end: TextSize) {
        let mut ty = Type::Unknown;
        if let Some(path) = target.path()
            && let Some(context) = path.segments().first()
        {
            match scope.get(context).cloned() {
                None => {
                    self.error_at(
                        segment_range(&path, 0),
                        format!("unknown variable `{context}`"),
                    );
                }
                Some(variable) => {
                    self.refer_variable(segment_range(&path, 0), &variable);
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
        let token = target.alias_token();
        let visible = visible_until(token.as_ref(), rule_end);
        if let Some((alias, variable)) = self.bind(token, BindingKind::Target, visible, ty) {
            scope.define(alias, variable);
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
                let Some(first) = segments.first() else {
                    return element;
                };
                let variable = scope.get(first).cloned();
                if let Some(variable) = &variable {
                    self.refer_variable(segment_range(name, 0), variable);
                }
                match variable {
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
                            format!("unknown variable `{first}`"),
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
        if let Some(token) = invocation.name_token() {
            self.refer(
                token.text_range(),
                Referent::Transform(name.clone()),
                &Type::Unknown,
            );
        }
        if name == "dateOp" {
            self.warning(
                invocation.syntax(),
                "`dateOp` is not defined: neither the Mapping Language IG nor the StructureMap transform codes say what its parameters are or what it does, so no engine can run it; use toDate, toDateTime, toTime or a FHIRPath expression",
            );
        }
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
                let name = argument.text();
                match scope.get(&name).cloned() {
                    Some(variable) => {
                        self.refer_variable(trimmed_range(argument.syntax()), &variable)
                    }
                    None => self.error(argument.syntax(), format!("unknown variable `{name}`")),
                }
            }
        }
        if name == "create"
            && let Some(type_name) = arguments.first().and_then(string_literal)
        {
            let created = self.created_type(&type_name, &element, &arguments[0]);
            self.refer(
                trimmed_range(arguments[0].syntax()),
                Referent::Type(created.clone()),
                &created,
            );
            return created;
        }
        element
    }

    /// The type `create('Type')` makes, checked against the element it is assigned to.
    fn created_type(&mut self, type_name: &str, element: &Type, at: &Expr) -> Type {
        // In a logical model, a type next to the element's own (`create('CE')` into a `CE`
        // element, or a subtype of it), named as the engine names it.
        if let (None, Type::Node { structure, .. }) = (self.type_names.get(type_name), element)
            && structure.kind == Kind::Logical
            && let Some((prefix, _)) = structure.url.rsplit_once('/')
        {
            for id in [type_name.to_owned(), type_name.replace('_', "-")] {
                if let Some(found) = self.definitions.structure(&format!("{prefix}/{id}"))
                    && found.name == type_name
                {
                    return Type::Node {
                        structure: found,
                        path: Vec::new(),
                    };
                }
            }
        }
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
                let name = argument.text();
                match scope.get(&name).cloned() {
                    Some(variable) => {
                        self.refer_variable(trimmed_range(argument.syntax()), &variable);
                        types.push(Some(variable.ty));
                    }
                    None => {
                        self.error(argument.syntax(), format!("unknown variable `{name}`"));
                        types.push(None);
                    }
                }
            } else {
                types.push(None);
            }
        }
        if let Some(token) = invocation.name_token() {
            self.refer(
                token.text_range(),
                Referent::Group(name.clone()),
                &Type::Unknown,
            );
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
                if let Some(variable) = scope.get(&name).cloned() {
                    self.refer_variable(trimmed_range(node), &variable);
                    // A variable hides an element of the focus with the same name: almost always
                    // a slip (`src.code as code where (code.exists())` tests the variable).
                    // A primitive's own `value` is the primitive: no difference there.
                    if !(focus.is_primitive() && name == "value")
                        && matches!(self.types().lookup(focus, &name), Lookup::Found(_))
                    {
                        let message = format!(
                            "`{name}` here is the variable `{name}`, not the element `{name}` of the focus; rename the variable, or write `$this.{name}` for the element"
                        );
                        self.report(Severity::Warning, trimmed_range(node), message);
                    }
                    return variable.ty;
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
            Lookup::Found(found) => {
                let element = self.types().element_type(&found, None);
                self.refer(trimmed_range(at), Referent::Element(found), &element);
                element
            }
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
                    // A `uses` alias names its structure (a profile, say), as in the engine.
                    let resolved = annotation.map(|name| {
                        self.type_names
                            .get(name)
                            .cloned()
                            .unwrap_or_else(|| name.to_owned())
                    });
                    let chosen = if last { resolved.as_deref() } else { None };
                    if let (Some(chosen), Some(node)) = (chosen, annotation_node)
                        && !found.element.types.is_empty()
                        && !self
                            .types()
                            .is_one_of(chosen, &found.element.types, &found.owner.url)
                    {
                        self.error(
                            node.syntax(),
                            format!(
                                "`{}` is not a type of `{}.{segment}`; it is {}",
                                annotation.unwrap_or(chosen),
                                ty.display(),
                                found.element.types.join(", ")
                            ),
                        );
                    }
                    let element = self.types().element_type(&found, chosen);
                    self.refer(
                        segment_range(path, index),
                        Referent::Element(found),
                        &element,
                    );
                    element
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

/// The type name of a `: Type` annotation, without the colon.
fn annotation_range(annotation: &ast::TypeAnnotation) -> TextRange {
    annotation
        .syntax()
        .children()
        .next()
        .map(|name| trimmed_range(&name))
        .unwrap_or_else(|| trimmed_range(annotation.syntax()))
}

/// Where an alias declared by `token` can be used: from the alias to the end of its rule.
fn visible_until(token: Option<&SyntaxToken>, rule_end: TextSize) -> TextRange {
    match token {
        Some(token) => TextRange::new(
            token.text_range().end(),
            rule_end.max(token.text_range().end()),
        ),
        None => TextRange::empty(rule_end),
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
