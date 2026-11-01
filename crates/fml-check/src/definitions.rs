//! The structure definitions a map is checked against.
//!
//! A deliberately small model: what the checker needs to follow element paths and compare types.
//! [`Definitions`] supplies structures by canonical URL; the `fhirschema` feature provides the core
//! FHIR releases, and anything else (logical models, profiles from packages) can be added by
//! implementing the trait.

use std::collections::BTreeMap;
use std::sync::Arc;

/// Supplies structure definitions by canonical URL.
pub trait Definitions {
    /// The structure at `url`, such as `http://hl7.org/fhir/StructureDefinition/Patient` or a
    /// version-specific `http://hl7.org/fhir/4.0/StructureDefinition/Patient`. A `|version`
    /// suffix may be present.
    fn structure(&self, url: &str) -> Option<Arc<Structure>>;

    /// Whether `url` belongs to a set of definitions this provider covers. When it does and
    /// [`structure`](Self::structure) finds nothing, the structure does not exist; otherwise the
    /// checker cannot tell and stays silent.
    fn covers(&self, url: &str) -> bool {
        let _ = url;
        false
    }
}

/// No definitions: only checks that need none (variables, groups, transforms) run.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoDefinitions;

impl Definitions for NoDefinitions {
    fn structure(&self, _url: &str) -> Option<Arc<Structure>> {
        None
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    PrimitiveType,
    ComplexType,
    Resource,
    Logical,
}

/// A resource, data type or logical model.
#[derive(Debug, Clone, PartialEq)]
pub struct Structure {
    pub url: String,
    pub name: String,
    /// The type it defines, such as `Patient`.
    pub type_name: String,
    pub kind: Kind,
    pub is_abstract: bool,
    /// Canonical URL of the base structure.
    pub base: Option<String>,
    /// Top-level elements defined here, not inherited ones.
    pub elements: BTreeMap<String, Element>,
}

/// An element of a structure.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Element {
    /// Type codes or URLs. Several for a choice element (`value[x]`).
    pub types: Vec<String>,
    /// For a choice element, the concrete element names (`valueString`, …).
    pub choices: Vec<String>,
    /// Repeats (`max` > 1).
    pub array: bool,
    /// Inline child elements of a backbone element, shared so an element is cheap to copy.
    pub children: Arc<BTreeMap<String, Element>>,
    /// A content reference to another element: structure URL and element path.
    pub content_reference: Option<(String, Vec<String>)>,
    /// For a concrete choice element (`valueString`), the choice it belongs to (`value`).
    pub choice_of: Option<String>,
    /// Position in the structure's element order, for writing elements in definition order.
    pub index: Option<usize>,
}

impl Element {
    pub fn is_choice(&self) -> bool {
        !self.choices.is_empty() || self.types.len() > 1
    }
}

/// Canonical URL of type `code` as used inside the structure at `context_url`: type codes resolve
/// in the same FHIR release as the structure that uses them, so inside a version-specific core
/// structure (`http://hl7.org/fhir/4.0/StructureDefinition/Patient`) `HumanName` is the 4.0 one.
/// Anywhere else, profiles included, type codes are the core types.
pub fn type_url(code: &str, context_url: &str) -> String {
    if code.contains("://") || code.starts_with("urn:") {
        return code.to_owned();
    }
    match versioned_core(context_url) {
        Some(prefix) => format!("{prefix}/StructureDefinition/{code}"),
        None => format!("http://hl7.org/fhir/StructureDefinition/{code}"),
    }
}

/// `http://hl7.org/fhir/4.0` for a structure URL of a version-specific core release.
pub fn versioned_core(url: &str) -> Option<&str> {
    let url = url.split('|').next().unwrap_or(url);
    let version = url
        .strip_prefix("http://hl7.org/fhir/")?
        .split_once("/StructureDefinition/")?
        .0;
    let is_version = !version.is_empty() && version.chars().all(|c| c.is_ascii_digit() || c == '.');
    is_version.then(|| &url[.."http://hl7.org/fhir/".len() + version.len()])
}

/// FHIR primitive type codes that are also FHIRPath System types under another name.
pub fn same_primitive(a: &str, b: &str) -> bool {
    let system = |code: &str| -> String {
        match code {
            "String" | "string" | "code" | "id" | "markdown" | "uri" | "url" | "canonical"
            | "oid" | "uuid" | "base64Binary" | "xhtml" => "String".into(),
            "Integer" | "integer" | "positiveInt" | "unsignedInt" => "Integer".into(),
            "Integer64" | "integer64" => "Long".into(),
            "Decimal" | "decimal" => "Decimal".into(),
            "Boolean" | "boolean" => "Boolean".into(),
            "Date" | "date" => "Date".into(),
            "DateTime" | "dateTime" | "instant" => "DateTime".into(),
            "Time" | "time" => "Time".into(),
            other => other.into(),
        }
    };
    system(a) == system(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn type_codes_resolve_in_the_release_of_the_structure() {
        let core = "http://hl7.org/fhir/StructureDefinition/HumanName";
        assert_eq!(
            type_url(
                "HumanName",
                "http://hl7.org/fhir/StructureDefinition/Patient"
            ),
            core
        );
        assert_eq!(
            type_url(
                "HumanName",
                "http://hl7.org/fhir/4.0/StructureDefinition/Patient|4.0.1"
            ),
            "http://hl7.org/fhir/4.0/StructureDefinition/HumanName"
        );
        // Profiles in hl7.org/fhir packages use the core types.
        assert_eq!(
            type_url(
                "HumanName",
                "http://hl7.org/fhir/us/core/StructureDefinition/us-core-patient"
            ),
            core
        );
        assert_eq!(
            type_url("HumanName", "http://example.org/StructureDefinition/P"),
            core
        );
        assert_eq!(
            type_url("http://example.org/T", core),
            "http://example.org/T"
        );
    }
}
