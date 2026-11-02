//! Metadata lines for StructureMap elements the model does not name: `/// jurisdiction.coding.code
//! = 'AQ'`.
//!
//! The IG's metadata rules ("Metadata" in fml.md): the name is a StructureMap element, properties
//! after it reach sub-elements, a key written again starts a new repeat, an empty value starts a
//! complex element, and values are FHIRPath literals. Which elements repeat and which are complex
//! comes from the small part of the StructureMap definition below; the elements the model names
//! (`url`, `name`, `status`, …) are handled by the compiler.

use fml_syntax::SyntaxKind;
use fml_syntax::quote_fhirpath_string;
use serde_json::{Map, Value};

/// A metadata value as written: a FHIRPath literal.
pub(crate) struct Literal {
    pub kind: SyntaxKind,
    /// The text, unquoted for strings.
    pub text: String,
}

/// An element of a metadata type: its name, whether it repeats, and its type (`None` for a
/// primitive).
type Element = (&'static str, bool, Option<&'static str>);

/// The StructureMap elements a metadata line may set beyond the ones the model names (R5
/// `StructureMap`, metadata resource pattern).
const STRUCTURE_MAP: &[Element] = &[
    ("identifier", true, Some("Identifier")),
    ("versionAlgorithmString", false, None),
    ("versionAlgorithmCoding", false, Some("Coding")),
    ("contact", true, Some("ContactDetail")),
    ("useContext", true, Some("UsageContext")),
    ("jurisdiction", true, Some("CodeableConcept")),
    ("copyrightLabel", false, None),
];

fn elements(type_name: &str) -> &'static [Element] {
    match type_name {
        "Identifier" => &[
            ("use", false, None),
            ("type", false, Some("CodeableConcept")),
            ("system", false, None),
            ("value", false, None),
            ("period", false, Some("Period")),
            ("assigner", false, Some("Reference")),
        ],
        "Coding" => &[
            ("system", false, None),
            ("version", false, None),
            ("code", false, None),
            ("display", false, None),
            ("userSelected", false, None),
        ],
        "CodeableConcept" => &[("coding", true, Some("Coding")), ("text", false, None)],
        "ContactDetail" => &[
            ("name", false, None),
            ("telecom", true, Some("ContactPoint")),
        ],
        "ContactPoint" => &[
            ("system", false, None),
            ("value", false, None),
            ("use", false, None),
            ("rank", false, None),
            ("period", false, Some("Period")),
        ],
        "UsageContext" => &[
            ("code", false, Some("Coding")),
            ("valueCodeableConcept", false, Some("CodeableConcept")),
            ("valueQuantity", false, Some("Quantity")),
            ("valueRange", false, Some("Range")),
            ("valueReference", false, Some("Reference")),
        ],
        "Period" => &[("start", false, None), ("end", false, None)],
        "Reference" => &[
            ("reference", false, None),
            ("type", false, None),
            ("identifier", false, Some("Identifier")),
            ("display", false, None),
        ],
        "Quantity" => &[
            ("value", false, None),
            ("comparator", false, None),
            ("unit", false, None),
            ("system", false, None),
            ("code", false, None),
        ],
        "Range" => &[
            ("low", false, Some("Quantity")),
            ("high", false, Some("Quantity")),
        ],
        _ => &[],
    }
}

fn find(elements: &[Element], name: &str) -> Option<Element> {
    elements.iter().find(|(n, _, _)| *n == name).copied()
}

/// Whether `name` is a metadata element this module handles.
pub(crate) fn is_generic(name: &str) -> bool {
    find(STRUCTURE_MAP, name).is_some()
}

/// Applies one metadata line to the elements of the map the model does not name.
pub(crate) fn apply(
    root: &mut Map<String, Value>,
    path: &[String],
    value: Option<Literal>,
) -> Result<(), String> {
    let (last, parents) = path.split_last().ok_or("metadata needs a name")?;
    let mut object = root;
    let mut available = STRUCTURE_MAP;
    let mut walked = String::new();
    for segment in parents {
        let (name, repeats, type_name) =
            find(available, segment).ok_or_else(|| unknown(&walked, segment))?;
        let Some(type_name) = type_name else {
            return Err(format!(
                "`{}` is a primitive and has no properties",
                join(&walked, name)
            ));
        };
        let slot = object.entry(name).or_insert_with(|| empty(repeats));
        let item = if repeats {
            let items = slot
                .as_array_mut()
                .ok_or("metadata conflicts with a value")?;
            if items.is_empty() {
                items.push(Value::Object(Map::new()));
            }
            items.last_mut().expect("not empty")
        } else {
            slot
        };
        object = item
            .as_object_mut()
            .ok_or("metadata conflicts with a value")?;
        available = elements(type_name);
        walked = join(&walked, name);
    }
    let (name, repeats, type_name) = find(available, last).ok_or_else(|| unknown(&walked, last))?;
    let path = join(&walked, name);
    let item = match (value, type_name) {
        (None, Some(_)) => Value::Object(Map::new()),
        (None, None) => return Err(format!("`{path}` is a primitive and needs a value")),
        (Some(_), Some(type_name)) => {
            return Err(format!(
                "`{path}` is a {type_name}: give it an empty value and set its properties"
            ));
        }
        (Some(literal), None) => literal_value(literal)?,
    };
    if repeats {
        let slot = object.entry(name).or_insert_with(|| empty(true));
        slot.as_array_mut()
            .ok_or("metadata conflicts with a value")?
            .push(item);
    } else if object.insert(name.to_owned(), item).is_some() {
        return Err(format!("`{path}` does not repeat; it is already set"));
    }
    Ok(())
}

fn empty(repeats: bool) -> Value {
    if repeats {
        Value::Array(Vec::new())
    } else {
        Value::Object(Map::new())
    }
}

fn join(walked: &str, name: &str) -> String {
    if walked.is_empty() {
        name.to_owned()
    } else {
        format!("{walked}.{name}")
    }
}

fn unknown(walked: &str, name: &str) -> String {
    if walked.is_empty() {
        format!("metadata `{name}` is not a StructureMap element")
    } else {
        format!("`{walked}` has no property `{name}`")
    }
}

/// A FHIRPath literal as a JSON value.
fn literal_value(literal: Literal) -> Result<Value, String> {
    Ok(match literal.kind {
        SyntaxKind::Integer | SyntaxKind::Decimal => Value::Number(
            literal
                .text
                .parse()
                .map_err(|_| format!("`{}` is not a number", literal.text))?,
        ),
        SyntaxKind::Ident => Value::Bool(literal.text == "true"),
        SyntaxKind::Date | SyntaxKind::DateTime => {
            Value::String(literal.text.trim_start_matches('@').to_owned())
        }
        SyntaxKind::Time => Value::String(literal.text.trim_start_matches("@T").to_owned()),
        SyntaxKind::LongNumber => {
            return Err(format!(
                "`{}`: long numbers are not FHIR values",
                literal.text
            ));
        }
        _ => Value::String(literal.text),
    })
}

/// The metadata lines for the elements of `other` this module handles, in the order a compiler
/// reads them back into the same elements.
pub(crate) fn print(other: &Map<String, Value>) -> Vec<String> {
    let mut lines = Vec::new();
    for (name, repeats, type_name) in STRUCTURE_MAP {
        if let Some(value) = other.get(*name) {
            print_element(&mut lines, name, *repeats, *type_name, value);
        }
    }
    lines
}

fn print_element(
    lines: &mut Vec<String>,
    path: &str,
    repeats: bool,
    type_name: Option<&str>,
    value: &Value,
) {
    let items: Vec<&Value> = match (repeats, value) {
        (true, Value::Array(items)) => items.iter().collect(),
        _ => vec![value],
    };
    for item in items {
        match (type_name, item) {
            (Some(type_name), Value::Object(object)) => {
                lines.push(format!("/// {path} ="));
                for (name, repeats, child_type) in elements(type_name) {
                    if let Some(child) = object.get(*name) {
                        print_element(
                            lines,
                            &format!("{path}.{name}"),
                            *repeats,
                            *child_type,
                            child,
                        );
                    }
                }
            }
            (None, Value::String(text)) => {
                lines.push(format!("/// {path} = {}", quote_fhirpath_string(text)));
            }
            (None, Value::Bool(_) | Value::Number(_)) => {
                lines.push(format!("/// {path} = {item}"));
            }
            _ => {}
        }
    }
}
