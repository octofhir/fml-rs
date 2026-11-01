//! Core FHIR definitions for R4, R4B, R5 and R6, from the schemas embedded in octofhir-fhirschema.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};

use octofhir_fhirschema::embedded::{FhirVersion, get_schema};
use octofhir_fhirschema::types::{FhirSchema, FhirSchemaElement};

use crate::definitions::{Definitions, Element, Kind, Structure};

/// A FHIR release with embedded definitions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Release {
    R4,
    R4B,
    R5,
    R6,
}

impl Release {
    fn version(self) -> FhirVersion {
        match self {
            Release::R4 => FhirVersion::R4,
            Release::R4B => FhirVersion::R4B,
            Release::R5 => FhirVersion::R5,
            Release::R6 => FhirVersion::R6,
        }
    }

    /// The release a `http://hl7.org/fhir/{major.minor}/` URL prefix names, as the cross-version
    /// maps use them. `None` for releases without embedded definitions (DSTU2, STU3).
    pub fn from_url_version(version: &str) -> Option<Release> {
        match version {
            "4.0" => Some(Release::R4),
            "4.3" => Some(Release::R4B),
            "5.0" => Some(Release::R5),
            "6.0" => Some(Release::R6),
            _ => None,
        }
    }

    pub fn parse(name: &str) -> Option<Release> {
        match name.to_ascii_lowercase().as_str() {
            "r4" | "4.0" | "4.0.1" => Some(Release::R4),
            "r4b" | "4.3" | "4.3.0" => Some(Release::R4B),
            "r5" | "5.0" | "5.0.0" => Some(Release::R5),
            "r6" | "6.0" => Some(Release::R6),
            _ => None,
        }
    }
}

/// The core definitions of every embedded release. Unversioned core URLs
/// (`http://hl7.org/fhir/StructureDefinition/Patient`) resolve in the default release;
/// version-specific ones (`http://hl7.org/fhir/4.0/StructureDefinition/Patient`) in theirs.
/// Converted structures by release and type name; `None` remembers a missing one.
type Cache = HashMap<(Release, String), Option<Arc<Structure>>>;

#[derive(Debug)]
pub struct CoreDefinitions {
    default: Release,
    cache: Mutex<Cache>,
}

impl CoreDefinitions {
    pub fn new(default: Release) -> Self {
        Self {
            default,
            cache: Mutex::new(HashMap::new()),
        }
    }

    /// The release and type name a core URL refers to. `Err(())` for a core URL of a release
    /// without definitions; `Ok(None)` for URLs outside the core namespace.
    fn locate(&self, url: &str) -> Result<Option<(Release, String)>, ()> {
        let url = url.split('|').next().unwrap_or(url);
        let Some(rest) = url.strip_prefix("http://hl7.org/fhir/") else {
            return Ok(None);
        };
        if let Some(name) = rest.strip_prefix("StructureDefinition/") {
            return Ok(Some((self.default, name.to_owned())));
        }
        let Some((version, name)) = rest.split_once("/StructureDefinition/") else {
            return Ok(None);
        };
        match Release::from_url_version(version) {
            Some(release) => Ok(Some((release, name.to_owned()))),
            None => Err(()),
        }
    }
}

impl Definitions for CoreDefinitions {
    fn structure(&self, url: &str) -> Option<Arc<Structure>> {
        let (release, name) = self.locate(url).ok()??;
        let mut cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        cache
            .entry((release, name.clone()))
            .or_insert_with(|| {
                get_schema(release.version(), &name).map(|schema| Arc::new(convert(schema, url)))
            })
            .clone()
    }

    fn covers(&self, url: &str) -> bool {
        matches!(self.locate(url), Ok(Some(_)))
    }
}

fn convert(schema: &FhirSchema, url: &str) -> Structure {
    let kind = match schema.kind.as_str() {
        "primitive-type" => Kind::PrimitiveType,
        "resource" => Kind::Resource,
        "logical" => Kind::Logical,
        _ => Kind::ComplexType,
    };
    let base = schema
        .base
        .as_deref()
        .map(|base| crate::definitions::type_url(base.rsplit('/').next().unwrap_or(base), url));
    Structure {
        url: url.split('|').next().unwrap_or(url).to_owned(),
        name: schema.name.clone(),
        type_name: schema.type_name.clone(),
        kind,
        is_abstract: schema.abstract_type.unwrap_or(false),
        base,
        elements: schema
            .elements
            .as_ref()
            .map(|elements| convert_elements(elements, url))
            .unwrap_or_default(),
    }
}

fn convert_elements(
    elements: &HashMap<String, FhirSchemaElement>,
    url: &str,
) -> BTreeMap<String, Element> {
    elements
        .iter()
        .map(|(name, element)| (name.clone(), convert_element(element, elements, url)))
        .collect()
}

fn convert_element(
    element: &FhirSchemaElement,
    siblings: &HashMap<String, FhirSchemaElement>,
    url: &str,
) -> Element {
    let choices = element.choices.clone().unwrap_or_default();
    let types = if choices.is_empty() {
        element.type_name.iter().cloned().collect()
    } else {
        choices
            .iter()
            .filter_map(|choice| siblings.get(choice)?.type_name.clone())
            .collect()
    };
    let content_reference = element.element_reference.as_ref().and_then(|reference| {
        let (structure, path) = reference.split_first()?;
        let path: Vec<String> = path
            .iter()
            .filter(|segment| *segment != "elements")
            .cloned()
            .collect();
        let name = structure.rsplit('/').next().unwrap_or(structure);
        Some((crate::definitions::type_url(name, url), path))
    });
    Element {
        types,
        choices,
        array: element.array.unwrap_or(false),
        children: element
            .elements
            .as_ref()
            .map(|children| convert_elements(children, url))
            .unwrap_or_default(),
        content_reference,
        choice_of: element.choice_of.clone(),
        index: element.index,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_versioned_and_default_urls() {
        let core = CoreDefinitions::new(Release::R5);
        let r4 = core
            .structure("http://hl7.org/fhir/4.0/StructureDefinition/Patient")
            .unwrap();
        assert_eq!(r4.type_name, "Patient");
        assert_eq!(
            r4.base.as_deref(),
            Some("http://hl7.org/fhir/4.0/StructureDefinition/DomainResource")
        );
        let deceased = &r4.elements["deceased"];
        assert_eq!(deceased.types, vec!["boolean", "dateTime"]);
        assert!(r4.elements["contact"].children.contains_key("relationship"));

        let r5 = core
            .structure("http://hl7.org/fhir/StructureDefinition/Patient|5.0.0")
            .unwrap();
        assert_eq!(r5.url, "http://hl7.org/fhir/StructureDefinition/Patient");

        assert!(
            core.structure("http://hl7.org/fhir/3.0/StructureDefinition/Patient")
                .is_none()
        );
        assert!(!core.covers("http://hl7.org/fhir/3.0/StructureDefinition/Patient"));
        assert!(core.covers("http://hl7.org/fhir/StructureDefinition/Nonexistent"));
        assert!(!core.covers("http://example.org/StructureDefinition/Custom"));
    }

    #[test]
    fn content_references() {
        let core = CoreDefinitions::new(Release::R4);
        let questionnaire = core
            .structure("http://hl7.org/fhir/StructureDefinition/Questionnaire")
            .unwrap();
        let nested = &questionnaire.elements["item"].children["item"];
        assert_eq!(
            nested.content_reference,
            Some((
                "http://hl7.org/fhir/StructureDefinition/Questionnaire".to_owned(),
                vec!["item".to_owned()]
            ))
        );
    }
}
