//! Definitions from FHIR packages: profiles, extensions and logical models over the core
//! releases.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use octofhir_fhirschema::{StructureDefinition, translate};
use serde_json::Value;

use crate::core::convert;
use crate::definitions::{Definitions, Structure};

/// StructureDefinitions loaded from FHIR packages (or any folder of JSON resources), in front of
/// another set of definitions, usually [`CoreDefinitions`](crate::CoreDefinitions). A profile
/// inherits what it does not constrain through its base.
pub struct PackageDefinitions<D> {
    by_url: HashMap<String, Arc<Structure>>,
    base: D,
}

impl<D: Definitions> PackageDefinitions<D> {
    pub fn new(base: D) -> Self {
        Self {
            by_url: HashMap::new(),
            base,
        }
    }

    /// Adds a StructureDefinition (FHIR JSON). Returns whether it was one and converted.
    pub fn add(&mut self, resource: &Value) -> Result<bool, String> {
        if resource["resourceType"] != "StructureDefinition" {
            return Ok(false);
        }
        let url = resource["url"].as_str().unwrap_or_default().to_owned();
        let definition: StructureDefinition = serde_json::from_value(resource.clone())
            .map_err(|e| format!("StructureDefinition `{url}`: {e}"))?;
        let schema =
            translate(definition, None).map_err(|e| format!("StructureDefinition `{url}`: {e}"))?;
        let structure = Arc::new(convert(&schema, &url));
        if let Some(version) = resource["version"].as_str() {
            self.by_url
                .insert(format!("{url}|{version}"), structure.clone());
        }
        self.by_url.insert(url, structure);
        Ok(true)
    }

    /// Adds a core StructureDefinition of another FHIR release under that release's
    /// version-specific URL, as the cross-version maps name them: `release` `3.0` turns
    /// `http://hl7.org/fhir/StructureDefinition/Patient` into
    /// `http://hl7.org/fhir/3.0/StructureDefinition/Patient`, and the types it uses resolve in the
    /// same release. STU3 definitions are read too: their element types are brought to the R4
    /// shape first.
    pub fn add_release(&mut self, resource: &Value, release: &str) -> Result<bool, String> {
        if resource["resourceType"] != "StructureDefinition" {
            return Ok(false);
        }
        let Some(name) = resource["url"]
            .as_str()
            .and_then(|url| url.strip_prefix("http://hl7.org/fhir/StructureDefinition/"))
        else {
            return Ok(false);
        };
        let url = format!("http://hl7.org/fhir/{release}/StructureDefinition/{name}");
        let mut resource = resource.clone();
        resource["url"] = Value::String(url.clone());
        if release.starts_with('3') || release.starts_with('1') {
            stu3_types_to_r4(&mut resource);
        }
        let definition: StructureDefinition = serde_json::from_value(resource)
            .map_err(|e| format!("StructureDefinition `{url}`: {e}"))?;
        let schema =
            translate(definition, None).map_err(|e| format!("StructureDefinition `{url}`: {e}"))?;
        self.by_url
            .insert(url.clone(), Arc::new(convert(&schema, &url)));
        Ok(true)
    }

    /// Adds every core StructureDefinition in a folder (a core package's `package` folder) under
    /// the release's version-specific URLs; see [`add_release`](Self::add_release).
    pub fn add_release_folder(
        &mut self,
        folder: &Path,
        release: &str,
    ) -> std::io::Result<(usize, Vec<String>)> {
        let mut added = 0;
        let mut problems = Vec::new();
        for entry in std::fs::read_dir(folder)? {
            let path = entry?.path();
            if path.extension().is_none_or(|e| e != "json") {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            let Ok(resource) = serde_json::from_str::<Value>(&text) else {
                continue;
            };
            match self.add_release(&resource, release) {
                Ok(true) => added += 1,
                Ok(false) => {}
                Err(problem) => problems.push(format!("{}: {problem}", path.display())),
            }
        }
        Ok((added, problems))
    }

    /// Adds every StructureDefinition in a folder (a package's `package` folder, say); returns
    /// how many, and the files that could not be read as one.
    pub fn add_folder(&mut self, folder: &Path) -> std::io::Result<(usize, Vec<String>)> {
        let mut added = 0;
        let mut problems = Vec::new();
        for entry in std::fs::read_dir(folder)? {
            let path = entry?.path();
            if path.extension().is_none_or(|e| e != "json") {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            let Ok(resource) = serde_json::from_str::<Value>(&text) else {
                continue;
            };
            match self.add(&resource) {
                Ok(true) => added += 1,
                Ok(false) => {}
                Err(problem) => problems.push(format!("{}: {problem}", path.display())),
            }
        }
        Ok((added, problems))
    }

    pub fn len(&self) -> usize {
        self.by_url.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_url.is_empty()
    }
}

/// STU3 element types in the R4 shape: one entry per type code, with `profile` and
/// `targetProfile` as lists (STU3 repeats the code once per profile, each a single URL).
fn stu3_types_to_r4(resource: &mut Value) {
    // A primitive's `value` has no type code in STU3, only extensions; R4 names the FHIRPath type.
    let system = match resource["type"].as_str().unwrap_or_default() {
        "boolean" => "Boolean",
        "integer" | "positiveInt" | "unsignedInt" => "Integer",
        "decimal" => "Decimal",
        "date" => "Date",
        "dateTime" | "instant" => "DateTime",
        "time" => "Time",
        _ => "String",
    };
    let system = format!("http://hl7.org/fhirpath/System.{system}");
    for view in ["snapshot", "differential"] {
        let Some(elements) = resource[view]["element"].as_array_mut() else {
            continue;
        };
        for element in elements {
            let Some(types) = element["type"].as_array() else {
                continue;
            };
            let mut merged: Vec<Value> = Vec::new();
            for entry in types {
                let mut entry = entry.clone();
                if entry.get("code").is_none() {
                    entry["code"] = Value::String(system.clone());
                }
                for key in ["profile", "targetProfile"] {
                    if let Some(single) = entry[key].as_str() {
                        entry[key] = Value::Array(vec![Value::String(single.to_owned())]);
                    }
                }
                match merged.iter_mut().find(|m| m["code"] == entry["code"]) {
                    Some(existing) => {
                        for key in ["profile", "targetProfile"] {
                            if let Some(more) = entry[key].as_array() {
                                let list = existing[key].as_array_mut();
                                match list {
                                    Some(list) => list.extend(more.iter().cloned()),
                                    None => existing[key] = Value::Array(more.clone()),
                                }
                            }
                        }
                    }
                    None => merged.push(entry),
                }
            }
            element["type"] = Value::Array(merged);
        }
    }
}

impl<D: Definitions> Definitions for PackageDefinitions<D> {
    fn structure(&self, url: &str) -> Option<Arc<Structure>> {
        self.by_url
            .get(url)
            .or_else(|| self.by_url.get(url.split('|').next().unwrap_or(url)))
            .cloned()
            .or_else(|| self.base.structure(url))
    }

    fn covers(&self, url: &str) -> bool {
        self.by_url
            .contains_key(url.split('|').next().unwrap_or(url))
            || self.base.covers(url)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::{CoreDefinitions, Lookup, Release, Types};

    #[test]
    fn profiles_and_logical_models_resolve_over_the_core() {
        let mut definitions = PackageDefinitions::new(CoreDefinitions::new(Release::R4));
        let logical = json!({
            "resourceType": "StructureDefinition",
            "url": "http://example.org/StructureDefinition/Lab",
            "name": "Lab",
            "status": "draft",
            "kind": "logical",
            "abstract": false,
            "type": "http://example.org/StructureDefinition/Lab",
            "baseDefinition": "http://hl7.org/fhir/StructureDefinition/Base",
            "derivation": "specialization",
            "differential": { "element": [
                { "id": "Lab", "path": "Lab" },
                { "id": "Lab.code", "path": "Lab.code", "min": 1, "max": "1", "type": [{ "code": "string" }] },
                { "id": "Lab.value", "path": "Lab.value", "min": 0, "max": "1", "type": [{ "code": "decimal" }] }
            ]}
        });
        assert_eq!(definitions.add(&logical), Ok(true));
        assert_eq!(
            definitions.add(&json!({ "resourceType": "ValueSet" })),
            Ok(false)
        );
        let types = Types::new(&definitions);
        let lab = types.structure("http://example.org/StructureDefinition/Lab");
        assert!(matches!(types.lookup(&lab, "code"), Lookup::Found(_)));
        assert!(matches!(
            types.lookup(&lab, "nope"),
            Lookup::Missing | Lookup::Unknown
        ));
        // A profile reads what it does not constrain from its base.
        let profile = json!({
            "resourceType": "StructureDefinition",
            "url": "http://example.org/StructureDefinition/my-patient",
            "name": "MyPatient",
            "status": "draft",
            "kind": "resource",
            "abstract": false,
            "type": "Patient",
            "baseDefinition": "http://hl7.org/fhir/StructureDefinition/Patient",
            "derivation": "constraint",
            "differential": { "element": [
                { "id": "Patient", "path": "Patient" },
                { "id": "Patient.identifier", "path": "Patient.identifier", "min": 1 }
            ]}
        });
        assert_eq!(definitions.add(&profile), Ok(true));
        let types = Types::new(&definitions);
        let profile = types.structure("http://example.org/StructureDefinition/my-patient");
        assert!(matches!(
            types.lookup(&profile, "identifier"),
            Lookup::Found(_)
        ));
        assert!(matches!(
            types.lookup(&profile, "birthDate"),
            Lookup::Found(_)
        ));
        assert!(matches!(
            types.lookup(&profile, "identifer"),
            Lookup::Missing
        ));
        // The core is still there.
        let patient = types.structure("http://hl7.org/fhir/StructureDefinition/Patient");
        assert!(matches!(
            types.lookup(&patient, "birthDate"),
            Lookup::Found(_)
        ));
    }
}
