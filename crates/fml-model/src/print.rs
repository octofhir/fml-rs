//! StructureMap → FML.
//!
//! The output compiles back to the same StructureMap: simple copies and batch identity rules are
//! written in their short forms, generated rule names are left out, documentation becomes
//! comments. The text is then laid out by the formatter.

use std::fmt::Write;

use fml_syntax::{FormatOptions, format, quote_fhirpath_string};
use serde_json::Value;

use crate::compile::{
    AUTO_VARIABLE, BATCH_UNNAMED_PREFIX, SOURCE_ELEMENT_EXTENSION, TARGET_ELEMENT_EXTENSION,
    make_id, source_path,
};
use crate::model::{Group, Parameter, ParameterValue, Rule, Source, StructureMap, Target};

/// Writes a StructureMap as FML.
pub fn print(map: &StructureMap) -> String {
    let mut printer = Printer { out: String::new() };
    printer.map(map);
    let text = printer.out;
    format(&text, &FormatOptions::default()).unwrap_or(text)
}

struct Printer {
    out: String,
}

impl Printer {
    fn line(&mut self, depth: usize, text: &str) {
        for _ in 0..depth {
            self.out.push_str("  ");
        }
        self.out.push_str(text);
        self.out.push('\n');
    }

    /// Documentation as comments, written so the compiler reads back the same text. The compiler
    /// trims each comment and joins the comments before a statement with `\r\n`, so text made of
    /// trimmed single lines becomes `//` lines and anything else one `/* */` block.
    fn documentation(&mut self, depth: usize, documentation: Option<&str>) {
        let Some(documentation) = documentation.filter(|d| !d.is_empty()) else {
            return;
        };
        let as_lines = documentation
            .split("\r\n")
            .all(|part| !part.contains(['\n', '\r']) && part == part.trim());
        if as_lines {
            for part in documentation.split("\r\n") {
                self.line(depth, format!("// {part}").trim_end());
            }
        } else if !documentation.contains("*/") && documentation == documentation.trim() {
            self.line(depth, &format!("/* {documentation} */"));
        } else {
            for line in documentation.lines() {
                self.line(depth, format!("// {}", line.trim()).trim_end());
            }
        }
    }

    fn map(&mut self, map: &StructureMap) {
        let metadata = |key: &str, value: &Option<String>| {
            value
                .as_ref()
                .map(|v| format!("/// {key} = {}", quote_fhirpath_string(v)))
        };
        let mut header: Vec<String> = Vec::new();
        header.extend(metadata("url", &map.url));
        header.extend(metadata("version", &map.version));
        header.extend(metadata("name", &map.name));
        header.extend(metadata("title", &map.title));
        header.extend(metadata("status", &map.status));
        if let Some(description) = &map.description
            && map.title.as_ref() != Some(description)
        {
            header.push(
                if description.contains(['\n', '\r'])
                    && !description.contains("\"\"\"")
                    && !description.ends_with('"')
                {
                    format!("/// description = \"\"\"{description}\"\"\"")
                } else {
                    format!("/// description = {}", quote_fhirpath_string(description))
                },
            );
        }
        if let Some(experimental) = map.experimental {
            header.push(format!("/// experimental = {experimental}"));
        }
        header.extend(metadata("date", &map.date));
        header.extend(metadata("publisher", &map.publisher));
        header.extend(metadata("purpose", &map.purpose));
        header.extend(metadata("copyright", &map.copyright));
        for line in header {
            self.line(0, &line);
        }
        self.out.push('\n');

        for contained in &map.contained {
            if contained.get("resourceType").and_then(Value::as_str) == Some("ConceptMap") {
                self.concept_map(contained);
            }
        }
        for structure in &map.structure {
            self.documentation(0, structure.documentation.as_deref());
            let alias = structure
                .alias
                .as_ref()
                .map(|a| format!(" alias {}", name(a)))
                .unwrap_or_default();
            self.line(
                0,
                &format!(
                    "uses {}{alias} as {}",
                    double_quoted(&structure.url),
                    structure.mode
                ),
            );
        }
        self.out.push('\n');
        for import in &map.import {
            self.line(0, &format!("imports {}", double_quoted(import)));
        }
        self.out.push('\n');
        for constant in &map.constants {
            self.line(
                0,
                &format!("let {} = {};", name(&constant.name), constant.value),
            );
        }
        self.out.push('\n');
        for group in &map.group {
            self.group(group);
            self.out.push('\n');
        }
    }

    fn concept_map(&mut self, concept_map: &Value) {
        let id = concept_map.get("id").and_then(Value::as_str).unwrap_or("");
        self.line(0, &format!("conceptmap {} {{", double_quoted(id)));
        let groups: Vec<&Value> = concept_map
            .get("group")
            .and_then(Value::as_array)
            .map(|g| g.iter().collect())
            .unwrap_or_default();
        let text = |value: &Value, key: &str| -> String {
            value
                .get(key)
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned()
        };
        // One prefix per system, in order of appearance: s, t, u, …
        let mut prefixes: Vec<(String, char)> = Vec::new();
        let prefix_for = |system: String, prefixes: &mut Vec<(String, char)>| -> char {
            if let Some((_, p)) = prefixes.iter().find(|(s, _)| *s == system) {
                return *p;
            }
            let next = (b's' + prefixes.len() as u8) as char;
            prefixes.push((system, next));
            next
        };
        let mut lines = Vec::new();
        for group in &groups {
            let source = prefix_for(text(group, "source"), &mut prefixes);
            if let Some(mode) = group
                .get("unmapped")
                .and_then(|u| u.get("mode"))
                .and_then(Value::as_str)
            {
                let mode = if mode == "use-source-code" {
                    "provided"
                } else {
                    mode
                };
                lines.push(format!("unmapped for {source} = {mode}"));
            }
            let Some(target_system) = group.get("target").and_then(Value::as_str) else {
                continue;
            };
            let target = prefix_for(target_system.to_owned(), &mut prefixes);
            for element in group
                .get("element")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                let code = text(element, "code");
                for mapping in element
                    .get("target")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                {
                    let relation = match mapping
                        .get("relationship")
                        .or_else(|| mapping.get("equivalence"))
                        .and_then(Value::as_str)
                        .unwrap_or("related-to")
                    {
                        "equivalent" | "equal" => "==",
                        "not-related-to" | "disjoint" | "unmatched" => "!=",
                        "source-is-narrower-than-target" | "narrower" | "specializes" => "<=",
                        "source-is-broader-than-target" | "wider" | "subsumes" => ">=",
                        _ => "-",
                    };
                    lines.push(format!(
                        "{source}:{} {relation} {target}:{}",
                        code_text(&code),
                        code_text(&text(mapping, "code"))
                    ));
                }
            }
        }
        for (system, prefix) in &prefixes {
            self.line(1, &format!("prefix {prefix} = {}", double_quoted(system)));
        }
        self.out.push('\n');
        for line in lines {
            self.line(1, &line);
        }
        self.line(0, "}");
        self.out.push('\n');
    }

    fn group(&mut self, group: &Group) {
        self.documentation(0, group.documentation.as_deref());
        let mut header = format!("group {}(", name(&group.name));
        for (index, input) in group.input.iter().enumerate() {
            if index > 0 {
                header.push_str(", ");
            }
            let _ = write!(header, "{} {}", input.mode, name(&input.name));
            if let Some(type_name) = &input.type_name {
                let _ = write!(header, " : {type_name}");
            }
        }
        header.push(')');
        if let Some(extends) = &group.extends {
            let _ = write!(header, " extends {}", name(extends));
        }
        match group.type_mode.as_deref() {
            Some("types") => header.push_str(" <<types>>"),
            Some("type-and-types") => header.push_str(" <<type+>>"),
            _ => {}
        }
        header.push_str(" {");
        self.line(0, &header);
        self.rules(&group.rule, 1);
        self.line(0, "}");
    }

    fn rules(&mut self, rules: &[Rule], depth: usize) {
        let mut index = 0;
        while index < rules.len() {
            let end = batch_end(rules, index);
            if end > index {
                self.batch(&rules[index..=end], depth);
                index = end + 1;
            } else {
                self.rule(&rules[index], depth);
                index += 1;
            }
        }
    }

    /// `src -> tgt : a, b, c "prefix";`
    fn batch(&mut self, rules: &[Rule], depth: usize) {
        let first = &rules[0];
        self.documentation(depth, first.documentation.as_deref());
        let prefix = batch_prefix(first).unwrap_or_default();
        let fields: Vec<String> = rules
            .iter()
            .map(|r| name(r.source[0].element.as_deref().unwrap_or("")))
            .collect();
        let rule_name = if prefix == BATCH_UNNAMED_PREFIX {
            String::new()
        } else {
            format!(" {}", double_quoted(&prefix))
        };
        self.line(
            depth,
            &format!(
                "{} -> {} : {}{rule_name};",
                first.source[0].context,
                first.target[0].context.as_deref().unwrap_or(""),
                fields.join(", ")
            ),
        );
    }

    fn rule(&mut self, rule: &Rule, depth: usize) {
        self.documentation(depth, rule.documentation.as_deref());
        let abbreviate = is_abbreviated_copy(rule);
        let sources: Vec<String> = rule
            .source
            .iter()
            .map(|s| source_text(s, abbreviate))
            .collect();
        let mut text = sources.join(", ");
        if !rule.target.is_empty() {
            let targets: Vec<String> = rule
                .target
                .iter()
                .map(|t| target_text(t, abbreviate))
                .collect();
            let _ = write!(text, " -> {}", targets.join(", "));
        }
        if !rule.dependent.is_empty() && !abbreviate {
            let calls: Vec<String> = rule
                .dependent
                .iter()
                .map(|d| {
                    let arguments: Vec<String> = d.parameter.iter().map(parameter_text).collect();
                    format!("{}({})", name(&d.name), arguments.join(", "))
                })
                .collect();
            let _ = write!(text, " then {}", calls.join(", "));
            if !rule.rule.is_empty() {
                text.push_str(" {");
            }
        } else if !rule.rule.is_empty() {
            text.push_str(" then {");
        }
        let name_suffix = match &rule.name {
            Some(rule_name) if Some(rule_name) != generated_name(rule).as_ref() => {
                format!(" {}", double_quoted(rule_name))
            }
            _ => String::new(),
        };
        if rule.rule.is_empty() {
            self.line(depth, &format!("{text}{name_suffix};"));
        } else {
            self.line(depth, &text);
            self.rules(&rule.rule, depth + 1);
            self.line(depth, &format!("}}{name_suffix};"));
        }
    }
}

/// The name the compiler generates for an unnamed rule, which the printer can leave out.
fn generated_name(rule: &Rule) -> Option<String> {
    let [source] = rule.source.as_slice() else {
        return None;
    };
    let path = source_path(source);
    if path.is_empty() {
        return None;
    }
    Some(match &source.type_name {
        Some(type_name) => {
            let mut chars = type_name.chars();
            let capitalized: String = chars
                .next()
                .map(|c| c.to_uppercase().chain(chars).collect())
                .unwrap_or_default();
            path + &capitalized
        }
        None => path,
    })
}

/// The dependent group older reference compilers (before 2025) added to every simple copy.
const LEGACY_DEFAULT_GROUP: &str = "DefaultMappingGroupAnonymousAlias";

/// Whether the rule's dependents are only the legacy implicit default-group call.
fn has_only_legacy_default_dependent(rule: &Rule) -> bool {
    match rule.dependent.as_slice() {
        [] => true,
        [dependent] => {
            dependent.name == LEGACY_DEFAULT_GROUP
                && dependent
                    .parameter
                    .iter()
                    .all(|p| matches!(&p.value, ParameterValue::Id(id) if id == AUTO_VARIABLE))
        }
        _ => false,
    }
}

/// The simple copy form, written `src.a -> tgt.a`: both sides bound to the automatic variable and
/// an argument-less `create`.
fn is_abbreviated_copy(rule: &Rule) -> bool {
    let ([source], [target]) = (rule.source.as_slice(), rule.target.as_slice()) else {
        return false;
    };
    source.element.is_some()
        && source.variable.as_deref() == Some(AUTO_VARIABLE)
        && target.context.is_some()
        && target.element.is_some()
        && target.variable.as_deref() == Some(AUTO_VARIABLE)
        && matches!(target.transform.as_deref(), None | Some("create"))
        && target.parameter.is_empty()
        && has_only_legacy_default_dependent(rule)
        && rule.rule.is_empty()
}

/// A rule that a batch identity rule produces: `src.a -> tgt.a` with the automatic variable.
fn is_identity(rule: &Rule) -> bool {
    let ([source], [target]) = (rule.source.as_slice(), rule.target.as_slice()) else {
        return false;
    };
    is_abbreviated_copy(rule)
        && source.extension.is_empty()
        && target.extension.is_empty()
        && source.type_name.is_none()
        && source.min.is_none()
        && source.list_mode.is_none()
        && source.default_value.is_none()
        && source.condition.is_none()
        && source.check.is_none()
        && source.log_message.is_none()
        && target.list_mode.is_empty()
        && source.element == target.element
}

/// The batch name prefix of an identity rule named `prefix` + field.
fn batch_prefix(rule: &Rule) -> Option<String> {
    if !is_identity(rule) {
        return None;
    }
    let suffix = make_id(rule.source[0].element.as_deref()?);
    let name = rule.name.as_deref()?;
    let prefix = name.strip_suffix(suffix.as_str())?;
    (!suffix.is_empty() && !prefix.is_empty()).then(|| prefix.to_owned())
}

/// The last index of a batch identity run starting at `start`, or `start` when there is none.
fn batch_end(rules: &[Rule], start: usize) -> usize {
    let Some(prefix) = batch_prefix(&rules[start]) else {
        return start;
    };
    let contexts = |rule: &Rule| {
        (
            rule.source[0].context.clone(),
            rule.target[0].context.clone(),
        )
    };
    let first = contexts(&rules[start]);
    let mut end = start;
    for (index, rule) in rules.iter().enumerate().skip(start + 1) {
        if rule.documentation.is_some()
            || batch_prefix(rule).as_deref() != Some(prefix.as_str())
            || contexts(rule) != first
        {
            break;
        }
        end = index;
    }
    // A single rule is a batch only when its name could not come from the plain form.
    if end == start && prefix != BATCH_UNNAMED_PREFIX {
        return start;
    }
    end
}

fn source_text(source: &Source, abbreviate: bool) -> String {
    let mut text = source.context.clone();
    for segment in source.element.iter().chain(
        source
            .extension
            .iter()
            .filter(|e| e.url == SOURCE_ELEMENT_EXTENSION)
            .filter_map(|e| e.value_string.as_ref()),
    ) {
        text.push('.');
        text.push_str(&name(segment));
    }
    if let Some(type_name) = &source.type_name {
        let _ = write!(text, " : {type_name}");
    }
    if let Some(min) = source.min {
        let _ = write!(text, " {min}..{}", source.max.as_deref().unwrap_or("*"));
    }
    if let Some(mode) = &source.list_mode {
        let _ = write!(text, " {mode}");
    }
    if let Some(default) = &source.default_value {
        let _ = write!(text, " default ({default})");
    }
    if !abbreviate && let Some(variable) = &source.variable {
        let _ = write!(text, " as {}", name(variable));
    }
    if let Some(condition) = &source.condition {
        let _ = write!(text, " where ({condition})");
    }
    if let Some(check) = &source.check {
        let _ = write!(text, " check ({check})");
    }
    if let Some(log) = &source.log_message {
        let _ = write!(text, " log ({log})");
    }
    text
}

fn target_text(target: &Target, abbreviate: bool) -> String {
    let mut text = String::new();
    if let Some(context) = &target.context {
        text.push_str(context);
        for segment in target.element.iter().chain(
            target
                .extension
                .iter()
                .filter(|e| e.url == TARGET_ELEMENT_EXTENSION)
                .filter_map(|e| e.value_string.as_ref()),
        ) {
            text.push('.');
            text.push_str(&name(segment));
        }
    }
    if !abbreviate && let Some(transform) = &target.transform {
        if target.context.is_some() {
            text.push_str(" = ");
        }
        let values: Vec<&ParameterValue> = target.parameter.iter().map(|p| &p.value).collect();
        match (transform.as_str(), values.as_slice()) {
            ("copy", [_]) => text.push_str(&parameter_text(&target.parameter[0])),
            ("evaluate", [ParameterValue::String(expression)]) => {
                let _ = write!(text, "({expression})");
            }
            ("evaluate", [_, ParameterValue::String(expression)]) => {
                let _ = write!(
                    text,
                    "evaluate({}, {expression})",
                    parameter_text(&target.parameter[0])
                );
            }
            (transform, _) => {
                let arguments: Vec<String> = target.parameter.iter().map(parameter_text).collect();
                let _ = write!(text, "{transform}({})", arguments.join(", "));
            }
        }
    }
    if !abbreviate && let Some(variable) = &target.variable {
        let _ = write!(text, " as {}", name(variable));
    }
    for mode in &target.list_mode {
        let _ = write!(text, " {mode}");
        if mode == "share"
            && let Some(id) = &target.list_rule_id
        {
            let _ = write!(text, " {}", name(id));
        }
    }
    text
}

fn parameter_text(parameter: &Parameter) -> String {
    match &parameter.value {
        ParameterValue::Id(id) => id.split('.').map(name).collect::<Vec<_>>().join("."),
        ParameterValue::String(value) => quote_fhirpath_string(value),
        ParameterValue::Boolean(value) => value.to_string(),
        ParameterValue::Integer(value) => value.to_string(),
        ParameterValue::Decimal(value) => value.to_string(),
        ParameterValue::Date(value) | ParameterValue::DateTime(value) => format!("@{value}"),
        ParameterValue::Time(value) => format!("@T{value}"),
    }
}

/// An identifier, in backticks when it is not a plain name.
fn name(text: &str) -> String {
    let plain = text
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && text.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    if plain {
        text.to_owned()
    } else {
        format!("`{}`", text.replace('\\', "\\\\").replace('`', "\\`"))
    }
}

fn code_text(code: &str) -> String {
    let plain = !code.is_empty() && code.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    let starts_like_name = code
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_');
    if plain && (starts_like_name || code.chars().all(|c| c.is_ascii_digit())) {
        code.to_owned()
    } else {
        double_quoted(code)
    }
}

fn double_quoted(text: &str) -> String {
    format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""))
}
