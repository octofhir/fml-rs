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
    /// Inline child elements of a backbone element.
    pub children: BTreeMap<String, Element>,
    /// A content reference to another element: structure URL and element path.
    pub content_reference: Option<(String, Vec<String>)>,
}

impl Element {
    pub fn is_choice(&self) -> bool {
        !self.choices.is_empty() || self.types.len() > 1
    }
}

/// Canonical URL of type `code` as used inside the structure at `context_url`: type codes resolve
/// in the same FHIR release as the structure that uses them.
pub fn type_url(code: &str, context_url: &str) -> String {
    if code.contains("://") || code.starts_with("urn:") {
        return code.to_owned();
    }
    let context_url = context_url.split('|').next().unwrap_or(context_url);
    match context_url.rfind("/StructureDefinition/") {
        Some(index) if context_url.starts_with("http://hl7.org/fhir/") => {
            format!("{}/StructureDefinition/{code}", &context_url[..index])
        }
        _ => format!("http://hl7.org/fhir/StructureDefinition/{code}"),
    }
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
