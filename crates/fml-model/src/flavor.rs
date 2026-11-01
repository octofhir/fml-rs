//! StructureMap as FHIR R4 (and R4B) shape it, and back.
//!
//! The model follows R5 and the Mapping Language IG. R4 differs in a few elements, converted here
//! the way the HL7 version converter (`StructureMap40_50`, `ConceptMap40_50`) does:
//!
//! | R5 / IG | R4 |
//! |---|---|
//! | `dependent.parameter` (`valueId`, …) | `dependent.variable` (strings) |
//! | `source.defaultValue` (a FHIRPath literal) | `source.defaultValue[x]` (`'a'` → `defaultValueString`, `true` → `defaultValueBoolean`, numbers) |
//! | target with a context | `contextType: variable` |
//! | `group.typeMode` absent | `typeMode: none` (required in R4) |
//! | `const` | not in R4: the R4 output has none, [`to_r4`] reports it |
//! | parameter `valueDate`/`valueTime`/`valueDateTime` | `valueString` |
//! | ConceptMap `relationship` | `equivalence` |
//! | ConceptMap `unmapped.mode: use-source-code` | `provided` |

use serde_json::{Map, Value, json};

use crate::model::StructureMap;

/// A StructureMap in R4 JSON, and the R5 content R4 cannot hold.
#[derive(Debug, Clone, PartialEq)]
pub struct R4Map {
    pub json: Value,
    /// What R4 cannot express, for the caller to report: `let` constants (left out) and default
    /// values that are expressions rather than literals (kept as text in `defaultValueString`).
    pub dropped: Vec<String>,
}

/// Converts a StructureMap to its R4 JSON form.
pub fn to_r4(map: &StructureMap) -> R4Map {
    let mut json = serde_json::to_value(map).expect("a StructureMap serializes");
    let mut dropped = Vec::new();
    let object = json.as_object_mut().expect("a StructureMap is an object");
    if let Some(Value::Array(constants)) = object.remove("const") {
        for constant in constants {
            dropped.push(format!(
                "const `{}`",
                constant["name"].as_str().unwrap_or("?")
            ));
        }
    }
    for contained in array_mut(object, "contained") {
        if contained["resourceType"] == "ConceptMap" {
            concept_map_to_r4(contained);
        }
    }
    for group in array_mut(object, "group") {
        let group = group.as_object_mut().expect("a group is an object");
        group
            .entry("typeMode")
            .or_insert_with(|| Value::String("none".into()));
        for rule in array_mut(group, "rule") {
            rule_to_r4(rule, &mut dropped);
        }
    }
    R4Map { json, dropped }
}

/// Reads a StructureMap in R4 or R5 JSON. R4 elements are converted to the model; R5 content
/// passes through.
pub fn from_json(mut json: Value) -> Result<StructureMap, serde_json::Error> {
    if let Some(object) = json.as_object_mut() {
        for contained in array_mut(object, "contained") {
            if contained["resourceType"] == "ConceptMap" {
                concept_map_from_r4(contained);
            }
        }
        for group in array_mut(object, "group") {
            let group = group.as_object_mut().expect("a group is an object");
            if group.get("typeMode").and_then(Value::as_str) == Some("none") {
                group.remove("typeMode");
            }
            for rule in array_mut(group, "rule") {
                rule_from_r4(rule);
            }
        }
    }
    serde_json::from_value(json)
}

fn array_mut<'a>(
    object: &'a mut Map<String, Value>,
    key: &str,
) -> impl Iterator<Item = &'a mut Value> {
    object
        .get_mut(key)
        .and_then(Value::as_array_mut)
        .into_iter()
        .flatten()
}

fn rule_to_r4(rule: &mut Value, dropped: &mut Vec<String>) {
    let rule = rule.as_object_mut().expect("a rule is an object");
    let name = rule
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("?")
        .to_owned();
    for source in array_mut(rule, "source") {
        let source = source.as_object_mut().expect("a source is an object");
        if let Some(Value::String(default)) = source.remove("defaultValue") {
            let (key, value) = literal_default(&default).unwrap_or_else(|| {
                dropped.push(format!(
                    "default `{default}` of rule `{name}` is not a literal"
                ));
                ("defaultValueString", Value::String(default.clone()))
            });
            source.insert(key.into(), value);
        }
    }
    for target in array_mut(rule, "target") {
        let target = target.as_object_mut().expect("a target is an object");
        if target.contains_key("context") {
            target.insert("contextType".into(), "variable".into());
        }
        for parameter in array_mut(target, "parameter") {
            parameter_to_r4(parameter);
        }
    }
    for dependent in array_mut(rule, "dependent") {
        let dependent = dependent.as_object_mut().expect("a dependent is an object");
        if let Some(Value::Array(parameters)) = dependent.remove("parameter") {
            let variables: Vec<Value> = parameters
                .iter()
                .filter_map(|p| {
                    p.as_object()?
                        .iter()
                        .find(|(k, _)| k.starts_with("value"))
                        .map(|(_, v)| match v {
                            Value::String(s) => Value::String(s.clone()),
                            other => Value::String(other.to_string()),
                        })
                })
                .collect();
            dependent.insert("variable".into(), Value::Array(variables));
        }
    }
    for nested in array_mut(rule, "rule") {
        rule_to_r4(nested, dropped);
    }
}

/// The R4 `defaultValue[x]` for a FHIRPath literal default.
fn literal_default(expression: &str) -> Option<(&'static str, Value)> {
    let text = expression.trim();
    if text.len() >= 2 && text.starts_with('\'') && text.ends_with('\'') {
        let inner = &text[1..text.len() - 1];
        // A single string literal, not `'a' + 'b'`.
        if !inner.replace("\\\\", "").replace("\\'", "").contains('\'') {
            return Some(("defaultValueString", fml_syntax::ast::unescape(text).into()));
        }
        return None;
    }
    match text {
        "true" => return Some(("defaultValueBoolean", true.into())),
        "false" => return Some(("defaultValueBoolean", false.into())),
        _ => {}
    }
    if let Ok(integer) = text.parse::<i64>() {
        return Some(("defaultValueInteger", integer.into()));
    }
    if text.contains('.')
        && let Ok(Value::Number(number)) = serde_json::from_str::<Value>(text)
    {
        return Some(("defaultValueDecimal", Value::Number(number)));
    }
    None
}

fn rule_from_r4(rule: &mut Value) {
    let rule = rule.as_object_mut().expect("a rule is an object");
    for source in array_mut(rule, "source") {
        let source = source.as_object_mut().expect("a source is an object");
        let key = source
            .keys()
            .find(|k| k.starts_with("defaultValue") && k.as_str() != "defaultValue")
            .cloned();
        if let Some(key) = key
            && let Some(value) = source.remove(&key)
        {
            // R4 holds a value; R5 a FHIRPath expression: write the value as a literal.
            let expression = match value {
                Value::String(text) if key == "defaultValueBoolean" => text,
                Value::String(text) => fml_syntax::quote_fhirpath_string(&text),
                Value::Bool(flag) => flag.to_string(),
                Value::Number(number) => number.to_string(),
                other => other.to_string(),
            };
            source.insert("defaultValue".into(), Value::String(expression));
        }
    }
    for target in array_mut(rule, "target") {
        let target = target.as_object_mut().expect("a target is an object");
        target.remove("contextType");
    }
    for dependent in array_mut(rule, "dependent") {
        let dependent = dependent.as_object_mut().expect("a dependent is an object");
        if let Some(Value::Array(variables)) = dependent.remove("variable") {
            let parameters: Vec<Value> = variables
                .into_iter()
                .map(|v| json!({ "valueId": v }))
                .collect();
            dependent.insert("parameter".into(), Value::Array(parameters));
        }
    }
    for nested in array_mut(rule, "rule") {
        rule_from_r4(nested);
    }
}

fn parameter_to_r4(parameter: &mut Value) {
    let Some(object) = parameter.as_object_mut() else {
        return;
    };
    for key in ["valueDate", "valueTime", "valueDateTime"] {
        if let Some(value) = object.remove(key) {
            object.insert("valueString".into(), value);
        }
    }
}

const RELATIONSHIPS: &[(&str, &str)] = &[
    ("related-to", "relatedto"),
    ("equivalent", "equivalent"),
    ("source-is-narrower-than-target", "wider"),
    ("source-is-broader-than-target", "narrower"),
    ("not-related-to", "disjoint"),
];

fn concept_map_to_r4(concept_map: &mut Value) {
    for group in concept_map["group"].as_array_mut().into_iter().flatten() {
        if group["unmapped"]["mode"] == "use-source-code" {
            group["unmapped"]["mode"] = "provided".into();
        }
        for element in group["element"].as_array_mut().into_iter().flatten() {
            for target in element["target"].as_array_mut().into_iter().flatten() {
                let Some(object) = target.as_object_mut() else {
                    continue;
                };
                if let Some(Value::String(relationship)) = object.remove("relationship") {
                    let equivalence = RELATIONSHIPS
                        .iter()
                        .find(|(r5, _)| *r5 == relationship)
                        .map_or("relatedto", |(_, r4)| r4);
                    object.insert("equivalence".into(), equivalence.into());
                }
            }
        }
    }
}

fn concept_map_from_r4(concept_map: &mut Value) {
    for group in concept_map["group"].as_array_mut().into_iter().flatten() {
        if group["unmapped"]["mode"] == "provided" {
            group["unmapped"]["mode"] = "use-source-code".into();
        }
        for element in group["element"].as_array_mut().into_iter().flatten() {
            for target in element["target"].as_array_mut().into_iter().flatten() {
                let Some(object) = target.as_object_mut() else {
                    continue;
                };
                if let Some(Value::String(equivalence)) = object.remove("equivalence") {
                    let relationship = match equivalence.as_str() {
                        "equal" => "equivalent",
                        "subsumes" => "source-is-broader-than-target",
                        "specializes" => "source-is-narrower-than-target",
                        "unmatched" => "not-related-to",
                        "inexact" => "related-to",
                        other => RELATIONSHIPS
                            .iter()
                            .find(|(_, r4)| *r4 == other)
                            .map_or("related-to", |(r5, _)| r5),
                    };
                    object.insert("relationship".into(), relationship.into());
                }
            }
        }
    }
}
