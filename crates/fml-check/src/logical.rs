//! Logical models read from their snapshots.
//!
//! A logical model may give an element several types without a `[x]` in its name: the CDA model
//! does so for elements whose XML says the type with `xsi:type` (`Observation.value` is a `CD`, a
//! `PQ`, an `IVL_TS`, …). Such an element is not a choice: it keeps its name whatever the type,
//! and each value says which type it has. The snapshot describes it as it is, so logical models
//! with a snapshot are read from it rather than translated to a schema, where every element with
//! several types becomes a `[x]` choice.

use std::collections::BTreeMap;
use std::sync::Arc;

use serde_json::Value;

use crate::definitions::{Element, Kind, Structure, TemplateId};

const TYPE_PROFILE_STYLE: &str = "http://hl7.org/fhir/tools/StructureDefinition/type-profile-style";

/// A logical model, or a profile of one, from the snapshot of its StructureDefinition.
pub(crate) fn from_snapshot(resource: &Value) -> Option<Structure> {
    if resource["kind"] != "logical" {
        return None;
    }
    let snapshot = resource["snapshot"]["element"].as_array()?;
    let url = resource["url"].as_str()?;
    let root = snapshot.first()?["path"].as_str()?;
    let mut elements: BTreeMap<String, Element> = BTreeMap::new();
    // Elements under a slice belong to that slice; the base element describes them all.
    let mut index = 0;
    for definition in &snapshot[1..] {
        let Some(path) = definition["path"].as_str() else {
            continue;
        };
        let id = definition["id"].as_str().unwrap_or(path);
        if id.contains(':') {
            continue;
        }
        let Some(relative) = path.strip_prefix(root).and_then(|p| p.strip_prefix('.')) else {
            continue;
        };
        let segments: Vec<&str> = relative.split('.').collect();
        let (last, parents) = segments.split_last()?;
        index += 1;
        let element = element_of(definition, url, index);
        let name = last.trim_end_matches("[x]").to_owned();
        let choice = last.ends_with("[x]");
        insert(&mut elements, parents, &name, element, choice);
    }
    Some(Structure {
        url: url.to_owned(),
        name: resource["name"].as_str().unwrap_or(root).to_owned(),
        type_name: resource["type"].as_str().unwrap_or(url).to_owned(),
        kind: Kind::Logical,
        is_abstract: resource["abstract"].as_bool().unwrap_or(false),
        base: resource["baseDefinition"].as_str().map(str::to_owned),
        elements,
    })
}

/// The templates through which a profile applies, when its model profiles by template id
/// (`type-profile-style` = `cda`): the profile's `urn:hl7ii:root:extension` (or `urn:oid:root`)
/// identifiers.
pub(crate) fn templates(resource: &Value) -> Vec<TemplateId> {
    let by_template = resource["extension"].as_array().is_some_and(|exts| {
        exts.iter()
            .any(|e| e["url"] == TYPE_PROFILE_STYLE && e["valueCode"] == "cda")
    });
    if !by_template || resource["derivation"] != "constraint" {
        return Vec::new();
    }
    resource["identifier"]
        .as_array()
        .map(|identifiers| {
            identifiers
                .iter()
                .filter_map(|i| i["value"].as_str())
                .filter_map(TemplateId::parse)
                .collect()
        })
        .unwrap_or_default()
}

fn element_of(definition: &Value, context: &str, index: usize) -> Element {
    let types: Vec<String> = definition["type"]
        .as_array()
        .map(|types| {
            types
                .iter()
                .filter_map(|t| t["code"].as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    let array = match definition["max"].as_str() {
        Some("*") => true,
        Some(max) => max.parse::<u32>().is_ok_and(|m| m > 1),
        None => false,
    };
    let content_reference = definition["contentReference"]
        .as_str()
        .and_then(|reference| {
            let (structure, path) = reference.split_once('#')?;
            let structure = if structure.is_empty() {
                context
            } else {
                structure
            };
            let segments: Vec<String> = path.split('.').skip(1).map(str::to_owned).collect();
            Some((structure.to_owned(), segments))
        });
    Element {
        types,
        choices: Vec::new(),
        array,
        children: Arc::new(BTreeMap::new()),
        content_reference,
        choice_of: None,
        index: Some(index),
    }
}

fn insert(
    elements: &mut BTreeMap<String, Element>,
    parents: &[&str],
    name: &str,
    element: Element,
    choice: bool,
) {
    if let Some((first, rest)) = parents.split_first() {
        if let Some(parent) = elements.get_mut(first.trim_end_matches("[x]")) {
            insert(
                Arc::make_mut(&mut parent.children),
                rest,
                name,
                element,
                choice,
            );
        }
        return;
    }
    if choice && element.types.len() > 1 {
        // A `[x]` choice: the element under its base name, and one element per type.
        let mut choices = Vec::new();
        for code in &element.types {
            let suffix = capitalize(code.rsplit('/').next().unwrap_or(code));
            let concrete = format!("{name}{suffix}");
            choices.push(concrete.clone());
            elements.insert(
                concrete,
                Element {
                    types: vec![code.clone()],
                    choice_of: Some(name.to_owned()),
                    ..element.clone()
                },
            );
        }
        elements.insert(name.to_owned(), Element { choices, ..element });
        return;
    }
    elements.insert(name.to_owned(), element);
}

fn capitalize(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn several_types_without_x_stay_one_element() {
        let sd = json!({
            "resourceType": "StructureDefinition",
            "url": "http://example.org/StructureDefinition/Obs",
            "name": "Obs", "kind": "logical", "abstract": false,
            "type": "http://example.org/StructureDefinition/Obs",
            "snapshot": { "element": [
                { "id": "Obs", "path": "Obs" },
                { "id": "Obs.value", "path": "Obs.value", "max": "*",
                  "type": [{ "code": "http://example.org/StructureDefinition/CD" },
                           { "code": "http://example.org/StructureDefinition/PQ" }] },
                { "id": "Obs.effective[x]", "path": "Obs.effective[x]", "max": "1",
                  "type": [{ "code": "dateTime" }, { "code": "Period" }] },
                { "id": "Obs.range", "path": "Obs.range", "max": "1",
                  "type": [{ "code": "Base" }] },
                { "id": "Obs.range.low", "path": "Obs.range.low", "max": "1",
                  "type": [{ "code": "string" }] },
                { "id": "Obs.value:cd", "path": "Obs.value", "sliceName": "cd", "max": "1" }
            ]}
        });
        let structure = from_snapshot(&sd).unwrap();
        let value = &structure.elements["value"];
        assert_eq!(value.types.len(), 2);
        assert!(value.choices.is_empty());
        assert!(value.array);
        assert!(value.is_choice());
        assert_eq!(
            structure.elements["effective"].choices,
            vec!["effectiveDateTime", "effectivePeriod"]
        );
        assert_eq!(
            structure.elements["effectivePeriod"].choice_of.as_deref(),
            Some("effective")
        );
        assert!(structure.elements["range"].children.contains_key("low"));
    }

    #[test]
    fn templates_of_a_profile() {
        let sd = json!({
            "resourceType": "StructureDefinition",
            "url": "http://example.org/StructureDefinition/AllergyObs",
            "kind": "logical", "derivation": "constraint",
            "extension": [{ "url": TYPE_PROFILE_STYLE, "valueCode": "cda" }],
            "identifier": [
                { "system": "urn:ietf:rfc:3986", "value": "urn:hl7ii:2.16.840.1.113883.10.20.22.4.7:2014-06-09" },
                { "system": "urn:ietf:rfc:3986", "value": "urn:oid:2.16.840.1.113883.10.20.22.4.7" }
            ]
        });
        assert_eq!(
            templates(&sd),
            vec![
                TemplateId {
                    root: "2.16.840.1.113883.10.20.22.4.7".into(),
                    extension: Some("2014-06-09".into())
                },
                TemplateId {
                    root: "2.16.840.1.113883.10.20.22.4.7".into(),
                    extension: None
                },
            ]
        );
    }
}
