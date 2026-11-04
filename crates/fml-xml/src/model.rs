//! The XML shape of a logical model, read from its StructureDefinitions.
//!
//! FHIR describes how a logical model looks in XML with element and structure extensions from the
//! FHIR tools IG and the `ElementDefinition.representation` codes:
//!
//! - `xml-namespace` and `xml-name` give the namespace and the name of an element, or of the root
//!   element when a structure is a document root;
//! - `representation` says an element is an attribute (`xmlAttr`), the text content (`xmlText`),
//!   an element whose type an `xsi:type` attribute states (`typeAttr`), or narrative kept as
//!   markup (`cdaText`, `xhtml`);
//! - `xml-choice-group` marks an element that is not in the XML: its children appear directly in
//!   the parent, in any order (the parts of an address);
//! - `elementdefinition-defaulttype` is the type of a `typeAttr` element without `xsi:type`;
//! - `elementdefinition-date-format` gives the lexical form of a date and time in the XML.

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::Value;

const XML_NAME: &str = "http://hl7.org/fhir/tools/StructureDefinition/xml-name";
const XML_NAMESPACE: &[&str] = &[
    "http://hl7.org/fhir/tools/StructureDefinition/xml-namespace",
    "http://hl7.org/fhir/StructureDefinition/elementdefinition-namespace",
];
const CHOICE_GROUP: &str = "http://hl7.org/fhir/tools/StructureDefinition/xml-choice-group";
const DEFAULT_TYPE: &str = "http://hl7.org/fhir/StructureDefinition/elementdefinition-defaulttype";
const DATE_FORMAT: &str =
    "http://hl7.org/fhir/tools/StructureDefinition/elementdefinition-date-format";

/// How an element appears in XML.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Representation {
    /// A child element.
    Element,
    /// An attribute.
    Attribute,
    /// The text content of the parent element.
    Text,
    /// Markup kept as it is (CDA narrative, XHTML): the element and its content as a string.
    Markup,
}

/// An element of a type, in definition order.
#[derive(Debug, Clone)]
pub struct Element {
    /// The name in the model, and the property name in JSON.
    pub name: String,
    /// The name in XML.
    pub xml_name: String,
    /// The namespace of the XML element: stated on the element, or the namespace of the structure
    /// that defines it.
    pub namespace: Option<String>,
    /// Whether the namespace is stated on the element itself (an attribute is in no namespace
    /// unless it is).
    pub explicit_namespace: bool,
    pub representation: Representation,
    /// `xsi:type` states the type of the value (`typeAttr`).
    pub type_attribute: bool,
    /// Type codes or URLs.
    pub types: Vec<String>,
    /// The type of a `typeAttr` element without `xsi:type`.
    pub default_type: Option<String>,
    pub many: bool,
    /// Not in the XML: the children appear directly in the parent, in any order.
    pub choice_group: bool,
    /// A date and time in the model's own lexical form (`YYYYMMDDHHMMSS.UUUU[+|-ZZzz]`).
    pub date_format: bool,
    /// Elements declared inline, which take the place of the type's own.
    pub children: Vec<Element>,
}

impl Element {
    /// The single type of the element, if it has one.
    pub fn single_type(&self) -> Option<&str> {
        match self.types.as_slice() {
            [single] => Some(single),
            _ => None,
        }
    }

    pub fn child(&self, name: &str) -> Option<&Element> {
        self.children.iter().find(|c| c.name == name)
    }
}

/// A type of the model: a data type or a class.
#[derive(Debug, Clone)]
pub struct TypeDef {
    pub url: String,
    /// The structure's name: what `xsi:type` names it (`IVL_TS`).
    pub name: String,
    pub namespace: Option<String>,
    /// The element name when the type is a document root (`ClinicalDocument`).
    pub xml_name: Option<String>,
    pub base: Option<String>,
    pub is_abstract: bool,
    /// All elements, inherited ones included, in definition order.
    pub elements: Vec<Element>,
}

impl TypeDef {
    pub fn element(&self, name: &str) -> Option<&Element> {
        self.elements.iter().find(|e| e.name == name)
    }
}

/// What a document's root element is: a type, or an element of a type (a fragment such as the
/// `component` of a structured body, whose content is declared inline).
#[derive(Debug, Clone, Copy)]
pub enum Root<'m> {
    Type(&'m TypeDef),
    Element(&'m TypeDef, &'m Element),
}

impl<'m> Root<'m> {
    /// How the root is named: the type's URL, or the URL and the element (`….StructuredBody.component`).
    pub fn spec(&self) -> String {
        match self {
            Root::Type(ty) => ty.url.clone(),
            Root::Element(ty, element) => format!("{}.{}", ty.url, element.name),
        }
    }

    /// The first segment of JSON paths: the type name, or type and element (`StructuredBody.component`).
    pub fn path(&self) -> String {
        match self {
            Root::Type(ty) => ty.name.clone(),
            Root::Element(ty, element) => format!("{}.{}", ty.name, element.name),
        }
    }
}

/// The types of one or more logical models, by canonical URL.
#[derive(Debug, Default, Clone)]
pub struct Model {
    types: HashMap<String, Arc<TypeDef>>,
    /// `xsi:type` names: (namespace, name) and name alone.
    by_name: HashMap<(Option<String>, String), String>,
    by_bare_name: HashMap<String, Vec<String>>,
    /// Root element names: (namespace, name).
    roots: HashMap<(Option<String>, String), Vec<String>>,
}

impl Model {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a StructureDefinition. Profiles (constraints on another type) are not types of their
    /// own and are skipped, as are definitions without a snapshot. Returns whether it was added.
    pub fn add(&mut self, resource: &Value) -> bool {
        if resource["resourceType"] != "StructureDefinition"
            || resource["derivation"] == "constraint"
        {
            return false;
        }
        let Some(url) = resource["url"].as_str() else {
            return false;
        };
        let Some(snapshot) = resource["snapshot"]["element"].as_array() else {
            return false;
        };
        let Some(root) = snapshot.first() else {
            return false;
        };
        let root_path = root["path"].as_str().unwrap_or_default();
        let namespace = namespace_of(resource);
        let xml_name = extension_str(resource, XML_NAME).map(str::to_owned);
        let mut elements: Vec<Element> = Vec::new();
        for definition in &snapshot[1..] {
            let Some(path) = definition["path"].as_str() else {
                continue;
            };
            // Slices belong to profiles; a snapshot of a base type has none.
            if definition["sliceName"].is_string() {
                continue;
            }
            let Some(relative) = path
                .strip_prefix(root_path)
                .and_then(|p| p.strip_prefix('.'))
            else {
                continue;
            };
            let segments: Vec<&str> = relative.split('.').collect();
            let element = element_of(definition, segments[segments.len() - 1], &namespace);
            insert(&mut elements, &segments[..segments.len() - 1], element);
        }
        let name = resource["name"].as_str().unwrap_or(root_path).to_owned();
        let def = TypeDef {
            url: url.to_owned(),
            name: name.clone(),
            namespace: namespace.clone(),
            xml_name: xml_name.clone(),
            base: resource["baseDefinition"].as_str().map(str::to_owned),
            is_abstract: resource["abstract"].as_bool().unwrap_or(false),
            elements,
        };
        self.by_name
            .insert((namespace.clone(), name.clone()), url.to_owned());
        self.by_bare_name
            .entry(name)
            .or_default()
            .push(url.to_owned());
        if let Some(xml_name) = xml_name
            && !def.is_abstract
        {
            self.roots
                .entry((namespace, xml_name))
                .or_default()
                .push(url.to_owned());
        }
        self.types.insert(url.to_owned(), Arc::new(def));
        true
    }

    /// Adds every StructureDefinition in a folder of JSON files (a package's `package` folder).
    pub fn add_folder(&mut self, folder: &std::path::Path) -> std::io::Result<usize> {
        let mut added = 0;
        let mut paths: Vec<_> = std::fs::read_dir(folder)?
            .filter_map(|entry| entry.ok().map(|e| e.path()))
            .filter(|path| path.extension().is_some_and(|e| e == "json"))
            .collect();
        paths.sort();
        for path in paths {
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            let Ok(resource) = serde_json::from_str::<Value>(&text) else {
                continue;
            };
            if self.add(&resource) {
                added += 1;
            }
        }
        Ok(added)
    }

    pub fn len(&self) -> usize {
        self.types.len()
    }

    /// Every type, in no particular order.
    pub fn types(&self) -> impl Iterator<Item = &Arc<TypeDef>> {
        self.types.values()
    }

    pub fn is_empty(&self) -> bool {
        self.types.is_empty()
    }

    /// The type at a canonical URL (a `|version` suffix is ignored).
    pub fn get(&self, url: &str) -> Option<&Arc<TypeDef>> {
        self.types.get(url.split('|').next().unwrap_or(url))
    }

    /// Whether the model has a type at `url`.
    pub fn contains(&self, url: &str) -> bool {
        self.get(url).is_some()
    }

    /// The type an `xsi:type` names: by namespace and name, or by name when only one type has it.
    pub fn by_type_name(&self, namespace: Option<&str>, name: &str) -> Option<&Arc<TypeDef>> {
        if let Some(url) = self
            .by_name
            .get(&(namespace.map(str::to_owned), name.to_owned()))
        {
            return self.get(url);
        }
        match self.by_bare_name.get(name).map(Vec::as_slice) {
            Some([single]) => self.get(single),
            _ => None,
        }
    }

    /// The root a name gives: a canonical URL or type name, or either followed by `.element`.
    pub fn root(&self, spec: &str) -> Option<Root<'_>> {
        if let Some(ty) = self.get(spec).or_else(|| self.by_type_name(None, spec)) {
            return Some(Root::Type(ty));
        }
        let (ty, element) = spec.rsplit_once('.')?;
        let ty = self.get(ty).or_else(|| self.by_type_name(None, ty))?;
        let element = ty.element(element)?;
        Some(Root::Element(ty, element))
    }

    /// The types whose documents have this root element.
    pub fn roots(&self, namespace: Option<&str>, name: &str) -> Vec<&Arc<TypeDef>> {
        self.roots
            .get(&(namespace.map(str::to_owned), name.to_owned()))
            .map(|urls| urls.iter().filter_map(|url| self.get(url)).collect())
            .unwrap_or_default()
    }

    /// The type a type code of an element names, resolved like the element's own URL: a URL as
    /// it is, anything else in the namespace of `context` (`http://…/StructureDefinition/`).
    pub fn type_of(&self, code: &str, context: &str) -> Option<&Arc<TypeDef>> {
        if let Some(found) = self.get(code) {
            return Some(found);
        }
        let prefix = context.rsplit_once('/').map(|(p, _)| p)?;
        self.get(&format!("{prefix}/{code}"))
    }

    /// Whether `ty` is `ancestor` or derives from it.
    pub fn derives_from(&self, ty: &TypeDef, ancestor: &str) -> bool {
        let mut current = Some(ty.url.clone());
        for _ in 0..32 {
            let Some(url) = current else { return false };
            if url == ancestor {
                return true;
            }
            current = self.get(&url).and_then(|t| t.base.clone());
        }
        false
    }
}

fn insert(elements: &mut Vec<Element>, parents: &[&str], element: Element) {
    match parents.split_first() {
        None => elements.push(element),
        Some((first, rest)) => {
            if let Some(parent) = elements.iter_mut().rev().find(|e| e.name == *first) {
                insert(&mut parent.children, rest, element);
            }
        }
    }
}

fn element_of(definition: &Value, segment: &str, type_namespace: &Option<String>) -> Element {
    let name = segment.trim_end_matches("[x]").to_owned();
    let explicit = XML_NAMESPACE
        .iter()
        .find_map(|url| extension_str(definition, url))
        .map(str::to_owned);
    let representation = definition["representation"]
        .as_array()
        .map(|codes| codes.iter().filter_map(Value::as_str).collect::<Vec<_>>())
        .unwrap_or_default();
    let has = |code: &str| representation.contains(&code);
    let kind = if has("xmlAttr") {
        Representation::Attribute
    } else if has("xmlText") {
        Representation::Text
    } else if has("cdaText") || has("xhtml") {
        Representation::Markup
    } else {
        Representation::Element
    };
    let types = definition["type"]
        .as_array()
        .map(|types| {
            types
                .iter()
                .filter_map(|t| t["code"].as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    let many = match definition["max"].as_str() {
        Some("*") => true,
        Some(max) => max.parse::<u32>().is_ok_and(|m| m > 1),
        None => false,
    };
    Element {
        xml_name: extension_str(definition, XML_NAME)
            .map(str::to_owned)
            .unwrap_or_else(|| name.clone()),
        name,
        namespace: explicit.clone().or_else(|| type_namespace.clone()),
        explicit_namespace: explicit.is_some(),
        representation: kind,
        type_attribute: has("typeAttr"),
        types,
        default_type: extension_str(definition, DEFAULT_TYPE).map(str::to_owned),
        many,
        choice_group: definition["extension"].as_array().is_some_and(|exts| {
            exts.iter()
                .any(|e| e["url"] == CHOICE_GROUP && e["valueBoolean"] == true)
        }),
        date_format: extension_str(definition, DATE_FORMAT).is_some(),
        children: Vec::new(),
    }
}

fn namespace_of(resource: &Value) -> Option<String> {
    XML_NAMESPACE
        .iter()
        .find_map(|url| extension_str(resource, url))
        .map(str::to_owned)
}

/// The string value (`valueUri`, `valueString`, `valueCode`, `valueCanonical`) of an extension.
fn extension_str<'a>(holder: &'a Value, url: &str) -> Option<&'a str> {
    holder["extension"].as_array()?.iter().find_map(|ext| {
        if ext["url"] != url {
            return None;
        }
        [
            "valueUri",
            "valueString",
            "valueCode",
            "valueCanonical",
            "valueUrl",
        ]
        .iter()
        .find_map(|key| ext[*key].as_str())
    })
}
