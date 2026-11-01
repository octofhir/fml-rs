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
