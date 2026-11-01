//! FML → StructureMap.
//!
//! Follows the HL7 reference compiler (`StructureMapUtilities.parse` in org.hl7.fhir.core), so the
//! StructureMap is the one the IG Publisher and the validator would build from the same text:
//! auto-generated rule names, the `vvv` variables of the simple copy form, documentation taken
//! from comments, and FHIRPath expressions stored in canonical form.

use fml_syntax::ast::{
    self, AstNode, ConceptMap as ConceptMapNode, Expr, GroupTypeMode, ListMode, ModelMode,
    ParameterMode, TransformValue,
};
use fml_syntax::{
    Diagnostic, Direction, NodeOrToken, SyntaxElement, SyntaxKind, SyntaxNode, TextRange, parse,
    quote_fhirpath_string,
};
use serde_json::{Map, Value, json};

use crate::model::{
    Constant, Dependent, Extension, Group, Input, Parameter, ParameterValue, Rule, Source,
    Structure, StructureMap, Target,
};

/// Extension holding source path segments after the first element (`src.a.b` → `b`).
pub const SOURCE_ELEMENT_EXTENSION: &str =
    "http://hl7.org/fhir/6.0/StructureDefinition/extension-StructureMap.group.rule.source.element";
/// Extension holding target path segments after the first element.
pub const TARGET_ELEMENT_EXTENSION: &str =
    "http://hl7.org/fhir/6.0/StructureDefinition/extension-StructureMap.group.rule.target.element";
/// Extension on a parameter value that refers to a contained resource.
pub const REFERENCES_CONTAINED_EXTENSION: &str =
    "http://hl7.org/fhir/StructureDefinition/referencesContained";
/// The variable the simple copy form `src.a -> tgt.a;` binds on both sides.
pub const AUTO_VARIABLE: &str = "vvv";
/// Name prefix of the rules produced by an unnamed batch identity rule `src -> tgt : a, b;`.
pub const BATCH_UNNAMED_PREFIX: &str = "unnamed";

/// The outcome of compiling FML: the map, as complete as the source allows, and the problems.
#[derive(Debug, Clone)]
pub struct Compiled {
    pub map: StructureMap,
    /// Syntax errors first, then problems found while building the map.
    pub diagnostics: Vec<Diagnostic>,
}

impl Compiled {
    pub fn has_errors(&self) -> bool {
        self.diagnostics
            .iter()
            .any(|d| d.severity == fml_syntax::Severity::Error)
    }
}

/// Compiles FML source text to a StructureMap.
pub fn compile(text: &str) -> Compiled {
    let parse = parse(text);
    let mut compiled = compile_tree(&parse.tree());
    let mut diagnostics = parse.diagnostics().to_vec();
    diagnostics.append(&mut compiled.diagnostics);
    compiled.diagnostics = diagnostics;
    compiled
}

/// Compiles a parsed file. Syntax errors in the tree are not repeated in the result.
pub fn compile_tree(file: &ast::SourceFile) -> Compiled {
    let mut compiler = Compiler {
        diagnostics: Vec::new(),
    };
    let map = compiler.file(file);
    Compiled {
        map,
        diagnostics: compiler.diagnostics,
    }
}

struct Compiler {
    diagnostics: Vec<Diagnostic>,
}

impl Compiler {
    fn error(&mut self, node: &SyntaxNode, message: impl Into<String>) {
        self.diagnostics
            .push(Diagnostic::error(trimmed_range(node), message));
    }

    fn warning(&mut self, node: &SyntaxNode, message: impl Into<String>) {
        self.diagnostics
            .push(Diagnostic::warning(trimmed_range(node), message));
    }

    fn file(&mut self, file: &ast::SourceFile) -> StructureMap {
        let mut map = StructureMap::new();
        if let Some(declaration) = file.map_declaration() {
            map.url = declaration.url().and_then(|u| u.value());
            map.name = declaration.name().and_then(|n| n.value());
            map.description = trailing_comments(declaration.syntax());
            map.status = Some("draft".to_owned());
        }
        for metadata in file.metadata() {
            self.metadata(&mut map, &metadata);
        }
        if map.id.is_none()
            && let Some(name) = &map.name
        {
            let id = make_id(name);
            if !id.is_empty() {
                map.id = Some(id);
            }
        }
        if map.status.is_none() {
            map.status = Some("draft".to_owned());
        }
        if map.description.is_none() {
            map.description = map.title.clone();
        }
        for concept_map in file.concept_maps() {
            let contained = self.concept_map(&concept_map, map.status.as_deref());
            map.contained.push(contained);
        }
        for uses in file.uses() {
            let mode = uses.mode().unwrap_or(ModelMode::Source);
            map.structure.push(Structure {
                url: uses.url().and_then(|u| u.value()).unwrap_or_default(),
                mode: mode.as_str().to_owned(),
                alias: uses.alias(),
                documentation: leading_comments(uses.syntax()),
                other: Map::new(),
            });
        }
        for import in file.imports() {
            if let Some(url) = import.url().and_then(|u| u.value()) {
                map.import.push(url);
            }
        }
        for constant in file.constants() {
            map.constants.push(Constant {
                name: constant.name().unwrap_or_default(),
                value: constant
                    .expression()
                    .map(|e| canonical(&e))
                    .unwrap_or_default(),
                other: Map::new(),
            });
        }
        for group in file.groups() {
            let group = self.group(&group);
            map.group.push(group);
        }
        map
    }

    fn metadata(&mut self, map: &mut StructureMap, metadata: &ast::Metadata) {
        let Some(key) = metadata.key().map(|k| k.text()) else {
            return;
        };
        let value = metadata.value().and_then(|v| v.value());
        let field = match key.as_str() {
            "url" => &mut map.url,
            "name" => &mut map.name,
            "title" => &mut map.title,
            "version" => &mut map.version,
            "description" => &mut map.description,
            "status" => &mut map.status,
            "date" => &mut map.date,
            "publisher" => &mut map.publisher,
            "purpose" => &mut map.purpose,
            "copyright" => &mut map.copyright,
            "experimental" => {
                map.experimental = value.map(|v| v == "true");
                return;
            }
            _ => {
                self.warning(
                    metadata.syntax(),
                    format!("metadata `{key}` is not supported and is ignored"),
                );
                return;
            }
        };
        *field = value;
    }

    /// `conceptmap "id" { … }` → a contained ConceptMap (R5 shape).
    fn concept_map(&mut self, node: &ConceptMapNode, status: Option<&str>) -> Value {
        let id = node.url().and_then(|u| u.value()).unwrap_or_default();
        let prefixes: Vec<(String, String)> = node
            .prefixes()
            .filter_map(|p| Some((p.name()?, p.url()?.value()?)))
            .collect();
        let resolve = |compiler: &mut Compiler, prefix: Option<String>, at: &SyntaxNode| {
            let prefix = prefix.unwrap_or_default();
            match prefixes.iter().find(|(name, _)| *name == prefix) {
                Some((_, url)) => Some(url.clone()),
                None => {
                    compiler.error(at, format!("unknown concept map prefix `{prefix}`"));
                    None
                }
            }
        };
        let mut groups: Vec<ConceptMapGroup> = Vec::new();
        for unmapped in node.unmapped() {
            let Some(source) = resolve(self, unmapped.prefix(), unmapped.syntax()) else {
                continue;
            };
            let index = concept_map_group(&mut groups, &source, None);
            match unmapped.mode().as_deref() {
                Some("provided") => groups[index].unmapped = Some("use-source-code"),
                other => self.error(
                    unmapped.syntax(),
                    format!(
                        "unsupported unmapped mode `{}`; only `provided` is defined",
                        other.unwrap_or_default()
                    ),
                ),
            }
        }
        for entry in node.entries() {
            let (Some(source_code), Some(target_code)) = (entry.source(), entry.target()) else {
                continue;
            };
            let source = resolve(self, source_code.prefix(), source_code.syntax());
            let target = resolve(self, target_code.prefix(), target_code.syntax());
            let (Some(source), Some(target)) = (source, target) else {
                continue;
            };
            let relation = entry.relation().unwrap_or_default();
            let relationship = match relation.as_str() {
                "-" => "related-to",
                "==" => "equivalent",
                "!=" => "not-related-to",
                "<=" => "source-is-narrower-than-target",
                ">=" => "source-is-broader-than-target",
                other => {
                    self.error(
                        entry.syntax(),
                        format!("unknown concept map relationship `{other}`"),
                    );
                    continue;
                }
            };
            let index = concept_map_group(&mut groups, &source, Some(&target));
            groups[index].elements.push(json!({
                "code": source_code.code().unwrap_or_default(),
                "target": [{
                    "code": target_code.code().unwrap_or_default(),
                    "relationship": relationship,
                }],
            }));
        }
        let mut map = Map::new();
        map.insert("resourceType".into(), "ConceptMap".into());
        map.insert("id".into(), id.into());
        if let Some(status) = status {
            map.insert("status".into(), status.into());
        }
        let groups: Vec<Value> = groups
            .into_iter()
            .map(|g| {
                let mut group = Map::new();
                group.insert("source".into(), g.source.into());
                if let Some(target) = g.target {
                    group.insert("target".into(), target.into());
                }
                if !g.elements.is_empty() {
                    group.insert("element".into(), g.elements.into());
                }
                if let Some(mode) = g.unmapped {
                    group.insert("unmapped".into(), json!({ "mode": mode }));
                }
                Value::Object(group)
            })
            .collect();
        if !groups.is_empty() {
            map.insert("group".into(), groups.into());
        }
        Value::Object(map)
    }

    fn group(&mut self, node: &ast::Group) -> Group {
        let mut group = Group {
            name: node.name().unwrap_or_default(),
            extends: node.extends(),
            type_mode: node.type_mode().map(|mode| {
                match mode {
                    GroupTypeMode::Types => "types",
                    GroupTypeMode::TypeAndTypes => "type-and-types",
                }
                .to_owned()
            }),
            documentation: leading_comments(node.syntax()),
            ..Group::default()
        };
        for parameter in node.parameters() {
            group.input.push(Input {
                name: parameter.name().unwrap_or_default(),
                type_name: parameter.type_annotation().and_then(|t| t.type_name()),
                mode: match parameter.mode() {
                    Some(ParameterMode::Target) => "target",
                    _ => "source",
                }
                .to_owned(),
                documentation: None,
                other: Map::new(),
            });
        }
        if let Some(body) = node.body() {
            group.rule = self.rules(&body);
        }
        group
    }

    fn rules(&mut self, block: &ast::RuleBlock) -> Vec<Rule> {
        let mut rules = Vec::new();
        for node in block.rules() {
            self.rule(&node, &mut rules);
        }
        rules
    }

    /// Compiles one rule into `out`; a batch identity rule produces one rule per field.
    fn rule(&mut self, node: &ast::Rule, out: &mut Vec<Rule>) {
        let mut rule = Rule {
            documentation: leading_comments(node.syntax()),
            ..Rule::default()
        };
        rule.source = node.sources().map(|s| self.source(&s)).collect();
        rule.target = node.targets().map(|t| self.target(&t)).collect();

        let fields = node.identity_fields();
        if !fields.is_empty() {
            return self.batch_identity(node, rule, &fields, out);
        }

        if let Some(dependent) = node.dependent() {
            if let Some(block) = dependent.rules() {
                rule.rule = self.rules(&block);
            }
            for invocation in dependent.invocations() {
                rule.dependent.push(Dependent {
                    name: invocation.name().unwrap_or_default(),
                    parameter: invocation
                        .arguments()
                        .map(|argument| self.parameter(&argument, false).into())
                        .collect(),
                    other: Map::new(),
                });
            }
        }
        if is_simple_copy(&rule) {
            rule.source[0].variable = Some(AUTO_VARIABLE.to_owned());
            rule.target[0].variable = Some(AUTO_VARIABLE.to_owned());
            rule.target[0].transform = Some("create".to_owned());
        }
        rule.name = match node.name() {
            Some(name) => Some(name.replace('-', "")),
            None => self.generated_name(node, &rule),
        };
        out.push(rule);
    }

    /// The name the reference compiler gives an unnamed rule: the source path, plus the
    /// capitalised type when the source is typed.
    fn generated_name(&mut self, node: &ast::Rule, rule: &Rule) -> Option<String> {
        if rule.source.len() != 1 {
            self.error(node.syntax(), "a rule with several sources needs a name");
            return None;
        }
        let source = &rule.source[0];
        let path = source_path(source);
        if path.is_empty() {
            self.warning(
                node.syntax(),
                "a rule whose source has no element needs a name",
            );
            return None;
        }
        Some(match &source.type_name {
            Some(type_name) => path + &capitalize(type_name),
            None => path,
        })
    }

    /// `src -> tgt : a, b "name";` → one copy rule per field, named `name` + field.
    fn batch_identity(
        &mut self,
        node: &ast::Rule,
        mut first: Rule,
        fields: &[String],
        out: &mut Vec<Rule>,
    ) {
        let prefix = node
            .name()
            .map(|n| n.replace('-', ""))
            .unwrap_or_else(|| BATCH_UNNAMED_PREFIX.to_owned());
        let source_context = first
            .source
            .first()
            .map(|s| s.context.clone())
            .unwrap_or_default();
        let target_context = first.target.first().and_then(|t| t.context.clone());
        if first.target.is_empty() {
            first.target.push(Target::default());
        }
        let copy = |rule: &mut Rule, field: &str| {
            let source = &mut rule.source[0];
            source.element = Some(field.to_owned());
            source.variable = Some(AUTO_VARIABLE.to_owned());
            let target = &mut rule.target[0];
            target.element = Some(field.to_owned());
            target.variable = Some(AUTO_VARIABLE.to_owned());
            target.transform = Some("create".to_owned());
            rule.name = Some(make_id(&format!("{prefix}{field}")));
        };
        copy(&mut first, &fields[0]);
        out.push(first);
        for field in &fields[1..] {
            let mut rule = Rule {
                source: vec![Source {
                    context: source_context.clone(),
                    ..Source::default()
                }],
                target: vec![Target {
                    context: target_context.clone(),
                    ..Target::default()
                }],
                ..Rule::default()
            };
            copy(&mut rule, field);
            out.push(rule);
        }
    }

    fn source(&mut self, node: &ast::Source) -> Source {
        let mut segments = node.path().map(|p| p.segments()).unwrap_or_default();
        let mut source = Source::default();
        if !segments.is_empty() {
            source.context = segments.remove(0);
        }
        let mut segments = segments.into_iter();
        source.element = segments.next();
        source.extension = segments
            .map(|s| Extension::string(SOURCE_ELEMENT_EXTENSION, s))
            .collect();
        source.type_name = node.type_annotation().and_then(|t| t.type_name());
        if let Some((min, max)) = node.cardinality() {
            source.min = min.parse().ok();
            source.max = Some(max);
        }
        if let Some(default) = node.default_value()
            && let Some(expression) = default.expression()
        {
            // The legacy `default "text"` form holds a string, stored as a FHIRPath literal.
            source.default_value = Some(match expression.syntax().kind() {
                SyntaxKind::Literal
                    if expression
                        .syntax()
                        .first_token()
                        .is_some_and(|t| t.kind() == SyntaxKind::DoubleQuotedString) =>
                {
                    quote_fhirpath_string(&ast::unescape(
                        expression.syntax().first_token().unwrap().text(),
                    ))
                }
                _ => canonical(&expression),
            });
        }
        source.list_mode = node
            .list_mode()
            .map(|mode| list_mode_code(&mode).to_owned());
        source.variable = node.alias();
        source.condition = node.where_clause().map(|e| canonical(&e));
        source.check = node.check_clause().map(|e| canonical(&e));
        source.log_message = node.log_clause().map(|e| canonical(&e));
        source
    }

    fn target(&mut self, node: &ast::Target) -> Target {
        let mut target = Target::default();
        if let Some(path) = node.path() {
            let mut segments = path.segments().into_iter();
            target.context = segments.next();
            target.element = segments.next();
            target.extension = segments
                .map(|s| Extension::string(TARGET_ELEMENT_EXTENSION, s))
                .collect();
        }
        match node.transform() {
            Some(TransformValue::Literal(literal)) => {
                target.transform = Some("copy".to_owned());
                target.parameter.push(literal_parameter(&literal).into());
            }
            Some(TransformValue::Name(name)) => {
                target.transform = Some("copy".to_owned());
                target
                    .parameter
                    .push(ParameterValue::Id(name.text()).into());
            }
            Some(TransformValue::Expression(expression)) => {
                target.transform = Some("evaluate".to_owned());
                target
                    .parameter
                    .push(ParameterValue::String(canonical(&expression)).into());
            }
            Some(TransformValue::Invocation(invocation)) => {
                let name = invocation.name().unwrap_or_default();
                if !TRANSFORMS.contains(&name.as_str()) {
                    self.error(invocation.syntax(), format!("unknown transform `{name}`"));
                }
                let evaluate = name == "evaluate";
                target.parameter = invocation
                    .arguments()
                    .enumerate()
                    .map(|(index, argument)| {
                        Parameter::from(self.parameter(&argument, evaluate && index == 1))
                    })
                    .collect();
                // `translate(v, '#id', …)` names a contained ConceptMap; the reference compiler
                // records that with an extension on the value.
                if name == "translate"
                    && let Some(parameter) = target.parameter.get_mut(1)
                    && let ParameterValue::String(map) = &parameter.value
                    && map.starts_with('#')
                {
                    parameter.value_element = Some(json!({
                        "extension": [{
                            "url": REFERENCES_CONTAINED_EXTENSION,
                            "valueReference": { "reference": map },
                        }],
                    }));
                }
                target.transform = Some(name);
            }
            None => {}
        }
        target.variable = node.alias();
        if let Some(mode) = node.list_mode() {
            if let ListMode::Share(id) = &mode {
                target.list_rule_id = Some(id.clone());
            }
            target.list_mode.push(list_mode_code(&mode).to_owned());
        }
        target
    }

    /// A transform or dependent-group argument: a variable, a constant, or (for the expression
    /// argument of `evaluate`) FHIRPath text.
    fn parameter(&mut self, argument: &Expr, expression: bool) -> ParameterValue {
        if expression {
            return ParameterValue::String(canonical(argument));
        }
        let node = argument.syntax();
        match node.kind() {
            SyntaxKind::NameRef => {
                let token = node.first_token().expect("a name has a token");
                ParameterValue::Id(ast::name_text(&token))
            }
            SyntaxKind::Literal => match ast::Literal::cast(node.clone()) {
                Some(literal) => literal_parameter(&literal),
                None => ParameterValue::String(canonical(argument)),
            },
            SyntaxKind::PrefixExpr
                if node
                    .first_token()
                    .is_some_and(|t| t.kind() == SyntaxKind::Minus) =>
            {
                let text: String = canonical(argument).split_whitespace().collect();
                if let Ok(value) = text.parse::<i64>() {
                    ParameterValue::Integer(value)
                } else if let Ok(value) = text.parse::<serde_json::Number>() {
                    ParameterValue::Decimal(value)
                } else {
                    self.complex_parameter(argument)
                }
            }
            _ => self.complex_parameter(argument),
        }
    }

    fn complex_parameter(&mut self, argument: &Expr) -> ParameterValue {
        self.error(
            argument.syntax(),
            "a transform argument must be a variable or a constant; use `evaluate` for expressions",
        );
        ParameterValue::String(canonical(argument))
    }
}

/// The transforms of the Mapping Language IG, R5 and earlier versions.
const TRANSFORMS: &[&str] = &[
    "create",
    "copy",
    "truncate",
    "escape",
    "cast",
    "append",
    "translate",
    "reference",
    "dateOp",
    "uuid",
    "pointer",
    "evaluate",
    "cc",
    "c",
    "qty",
    "id",
    "cp",
    "toDateTime",
    "unixToDateTime",
    "toDate",
    "unixToDate",
    "toTime",
    "unixToTime",
];

struct ConceptMapGroup {
    source: String,
    target: Option<String>,
    elements: Vec<Value>,
    unmapped: Option<&'static str>,
}

/// The group for a source and target system, created when missing; same rules as the reference
/// compiler, where a group without a target yet adopts the first target used with it.
fn concept_map_group(
    groups: &mut Vec<ConceptMapGroup>,
    source: &str,
    target: Option<&str>,
) -> usize {
    for (index, group) in groups.iter_mut().enumerate() {
        if group.source == source
            && (group.target.is_none() || target.is_none() || group.target.as_deref() == target)
        {
            if group.target.is_none()
                && let Some(target) = target
            {
                group.target = Some(target.to_owned());
            }
            return index;
        }
    }
    groups.push(ConceptMapGroup {
        source: source.to_owned(),
        target: target.map(str::to_owned),
        elements: Vec::new(),
        unmapped: None,
    });
    groups.len() - 1
}

/// The simple copy form `src.a -> tgt.a;`: one source and one target, both with an element and no
/// variable, no parameters, no dependent rules.
fn is_simple_copy(rule: &Rule) -> bool {
    let [source] = rule.source.as_slice() else {
        return false;
    };
    let [target] = rule.target.as_slice() else {
        return false;
    };
    !source.context.is_empty()
        && source.element.is_some()
        && source.variable.is_none()
        && target.context.is_some()
        && target.element.is_some()
        && target.variable.is_none()
        && target.parameter.is_empty()
        && rule.dependent.is_empty()
        && rule.rule.is_empty()
}

/// The source element path: the element and the path segments in the element extension.
pub fn source_path(source: &Source) -> String {
    source
        .element
        .iter()
        .cloned()
        .chain(
            source
                .extension
                .iter()
                .filter(|e| e.url == SOURCE_ELEMENT_EXTENSION)
                .filter_map(|e| e.value_string.clone()),
        )
        .collect::<Vec<_>>()
        .join(".")
}

/// The FHIRPath text StructureMap stores; see [`crate::canonical`].
fn canonical(expression: &Expr) -> String {
    crate::canonical::canonical(expression)
}

fn literal_parameter(literal: &ast::Literal) -> ParameterValue {
    let value = literal.value().unwrap_or_default();
    match literal.kind() {
        Some(SyntaxKind::Integer | SyntaxKind::LongNumber) => value
            .trim_end_matches('L')
            .parse()
            .map(ParameterValue::Integer)
            .unwrap_or(ParameterValue::String(value)),
        Some(SyntaxKind::Decimal) => value
            .parse()
            .map(ParameterValue::Decimal)
            .unwrap_or(ParameterValue::String(value)),
        Some(SyntaxKind::Date) => ParameterValue::Date(value.trim_start_matches('@').to_owned()),
        Some(SyntaxKind::DateTime) => {
            ParameterValue::DateTime(value.trim_start_matches('@').to_owned())
        }
        Some(SyntaxKind::Time) => ParameterValue::Time(value.trim_start_matches("@T").to_owned()),
        Some(SyntaxKind::Ident) if value == "true" || value == "false" => {
            ParameterValue::Boolean(value == "true")
        }
        _ => ParameterValue::String(value),
    }
}

fn list_mode_code(mode: &ListMode) -> &'static str {
    match mode {
        ListMode::First => "first",
        ListMode::NotFirst => "not_first",
        ListMode::Last => "last",
        ListMode::NotLast => "not_last",
        ListMode::OnlyOne => "only_one",
        ListMode::Single => "single",
        ListMode::Collate => "collate",
        ListMode::Share(_) => "share",
    }
}

/// `Utilities.makeId` of the reference implementation: keeps ASCII letters, digits, `-` and `.`.
pub fn make_id(name: &str) -> String {
    name.chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '.')
        .collect()
}

fn capitalize(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// The documentation the reference compiler takes from comments: the comments between the
/// previous token and this statement, one per line, without their `//` or `/* */`. A comment on
/// the same line as the end of the previous rule or `uses` belongs to that statement instead.
fn leading_comments(node: &SyntaxNode) -> Option<String> {
    let mut comments = Vec::new();
    let mut previous: Option<SyntaxElement> = None;
    for element in node.siblings_with_tokens(Direction::Prev).skip(1) {
        match element {
            NodeOrToken::Token(token) if token.kind() == SyntaxKind::Whitespace => {}
            NodeOrToken::Token(token) if token.kind().is_comment() => comments.push(token),
            element => {
                previous = Some(element);
                break;
            }
        }
    }
    comments.reverse();
    // Comments right after `map "url" = "name"` are the map's description.
    if previous
        .as_ref()
        .is_some_and(|p| p.kind() == SyntaxKind::MapDeclaration)
    {
        return None;
    }
    let owns_trailing = previous
        .as_ref()
        .is_some_and(|p| matches!(p.kind(), SyntaxKind::Rule | SyntaxKind::UsesDeclaration));
    if owns_trailing
        && comments
            .first()
            .is_some_and(|first| !after_line_break(first))
    {
        comments.remove(0);
    }
    if comments.is_empty() {
        return None;
    }
    Some(
        comments
            .iter()
            .map(|token| comment_text(token.text()))
            .collect::<Vec<_>>()
            .join("\r\n"),
    )
}

/// Whether a line break comes right before `token`.
fn after_line_break(token: &fml_syntax::SyntaxToken) -> bool {
    token.prev_sibling_or_token().is_some_and(|e| match e {
        NodeOrToken::Token(t) => {
            t.kind() == SyntaxKind::Whitespace && t.text().contains(['\n', '\r'])
        }
        NodeOrToken::Node(_) => false,
    })
}

/// Comments right after `node` on the following lines, up to the next token.
fn trailing_comments(node: &SyntaxNode) -> Option<String> {
    let comments: Vec<String> = node
        .siblings_with_tokens(Direction::Next)
        .skip(1)
        .take_while(|e| e.kind().is_trivia())
        .filter_map(|e| e.into_token())
        .filter(|t| t.kind().is_comment())
        .map(|t| comment_text(t.text()))
        .collect();
    (!comments.is_empty()).then(|| comments.join("\r\n"))
}

fn comment_text(text: &str) -> String {
    let inner = if let Some(line) = text.strip_prefix("//") {
        line
    } else {
        text.strip_prefix("/*")
            .map(|t| t.strip_suffix("*/").unwrap_or(t))
            .unwrap_or(text)
    };
    inner.trim().to_owned()
}

/// The node's range without leading and trailing trivia, for diagnostics.
fn trimmed_range(node: &SyntaxNode) -> TextRange {
    let mut tokens = node
        .descendants_with_tokens()
        .filter_map(NodeOrToken::into_token)
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
