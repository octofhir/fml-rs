//! The StructureMap resource, as FHIR R5 and the Mapping Language IG define it.
//!
//! Fields follow the FHIR element order, so serialized maps read like the ones the HL7 tools
//! produce. Elements this model does not name are kept in `other` maps, so reading and writing a
//! StructureMap does not lose them.

use serde::de::{self, Deserializer, MapAccess, Visitor};
use serde::ser::{SerializeMap, Serializer};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// A StructureMap resource.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StructureMap {
    #[serde(default = "resource_type")]
    pub resource_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// Embedded resources: the concept maps declared with `conceptmap`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub contained: Vec<Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extension: Vec<Extension>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub experimental: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub publisher: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub purpose: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub copyright: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub structure: Vec<Structure>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub import: Vec<String>,
    #[serde(rename = "const", default, skip_serializing_if = "Vec::is_empty")]
    pub constants: Vec<Constant>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub group: Vec<Group>,
    /// Elements not modelled above, such as `identifier`, `contact` or `jurisdiction`.
    #[serde(flatten)]
    pub other: Map<String, Value>,
}

fn resource_type() -> String {
    "StructureMap".to_owned()
}

impl StructureMap {
    pub fn new() -> Self {
        Self {
            resource_type: resource_type(),
            ..Self::default()
        }
    }
}

/// An extension with a string value, the only kind the mapping language produces.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Extension {
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_string: Option<String>,
    #[serde(flatten)]
    pub other: Map<String, Value>,
}

impl Extension {
    pub fn string(url: &str, value: impl Into<String>) -> Self {
        Self {
            url: url.to_owned(),
            value_string: Some(value.into()),
            other: Map::new(),
        }
    }
}

/// `uses "url" alias Name as mode`
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Structure {
    pub url: String,
    pub mode: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub documentation: Option<String>,
    #[serde(flatten)]
    pub other: Map<String, Value>,
}

/// `let name = expression;`
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Constant {
    pub name: String,
    pub value: String,
    #[serde(flatten)]
    pub other: Map<String, Value>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Group {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extends: Option<String>,
    /// `types` or `type-and-types`; R4 maps also use `none`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub type_mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub documentation: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub input: Vec<Input>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rule: Vec<Rule>,
    #[serde(flatten)]
    pub other: Map<String, Value>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Input {
    pub name: String,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub type_name: Option<String>,
    /// `source` or `target`.
    pub mode: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub documentation: Option<String>,
    #[serde(flatten)]
    pub other: Map<String, Value>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Rule {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub source: Vec<Source>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub target: Vec<Target>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rule: Vec<Rule>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dependent: Vec<Dependent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub documentation: Option<String>,
    #[serde(flatten)]
    pub other: Map<String, Value>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Source {
    /// Path segments after the first element, in the IG's element extension.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extension: Vec<Extension>,
    pub context: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max: Option<String>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub type_name: Option<String>,
    /// A FHIRPath expression.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub element: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub list_mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub variable: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub condition: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub check: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub log_message: Option<String>,
    #[serde(flatten)]
    pub other: Map<String, Value>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Target {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extension: Vec<Extension>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
    /// R4 only: `variable` (or `type`, which R5 dropped).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub element: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub variable: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub list_mode: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub list_rule_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transform: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parameter: Vec<Parameter>,
    #[serde(flatten)]
    pub other: Map<String, Value>,
}

/// `then Group(a, b)`
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Dependent {
    pub name: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parameter: Vec<Parameter>,
    #[serde(flatten)]
    pub other: Map<String, Value>,
}

/// A transform or dependent-group parameter: `value[x]`, and the `_value[x]` element that carries
/// extensions on the value (such as the reference to a contained ConceptMap).
#[derive(Debug, Clone, PartialEq)]
pub struct Parameter {
    pub value: ParameterValue,
    pub value_element: Option<Value>,
}

impl From<ParameterValue> for Parameter {
    fn from(value: ParameterValue) -> Self {
        Self {
            value,
            value_element: None,
        }
    }
}

/// The value of a parameter.
#[derive(Debug, Clone, PartialEq)]
pub enum ParameterValue {
    /// A variable name.
    Id(String),
    String(String),
    Boolean(bool),
    Integer(i64),
    Decimal(serde_json::Number),
    Date(String),
    Time(String),
    DateTime(String),
}

impl ParameterValue {
    fn key(&self) -> &'static str {
        match self {
            ParameterValue::Id(_) => "valueId",
            ParameterValue::String(_) => "valueString",
            ParameterValue::Boolean(_) => "valueBoolean",
            ParameterValue::Integer(_) => "valueInteger",
            ParameterValue::Decimal(_) => "valueDecimal",
            ParameterValue::Date(_) => "valueDate",
            ParameterValue::Time(_) => "valueTime",
            ParameterValue::DateTime(_) => "valueDateTime",
        }
    }

    fn json(&self) -> Value {
        match self {
            ParameterValue::Id(v)
            | ParameterValue::String(v)
            | ParameterValue::Date(v)
            | ParameterValue::Time(v)
            | ParameterValue::DateTime(v) => Value::String(v.clone()),
            ParameterValue::Boolean(v) => Value::Bool(*v),
            ParameterValue::Integer(v) => Value::from(*v),
            ParameterValue::Decimal(v) => Value::Number(v.clone()),
        }
    }

    fn from_json(key: &str, value: Value) -> Result<Option<Self>, String> {
        let text = || {
            value
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| format!("{key} must be a string"))
        };
        Ok(Some(match key {
            "valueId" => ParameterValue::Id(text()?),
            "valueString" => ParameterValue::String(text()?),
            "valueDate" => ParameterValue::Date(text()?),
            "valueTime" => ParameterValue::Time(text()?),
            "valueDateTime" => ParameterValue::DateTime(text()?),
            "valueBoolean" => {
                ParameterValue::Boolean(value.as_bool().ok_or("valueBoolean must be a boolean")?)
            }
            "valueInteger" => {
                ParameterValue::Integer(value.as_i64().ok_or("valueInteger must be an integer")?)
            }
            "valueDecimal" => match value {
                Value::Number(n) => ParameterValue::Decimal(n),
                _ => return Err("valueDecimal must be a number".to_owned()),
            },
            _ => return Ok(None),
        }))
    }
}

impl Serialize for Parameter {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let key = self.value.key();
        let mut map = serializer.serialize_map(None)?;
        map.serialize_entry(key, &self.value.json())?;
        if let Some(element) = &self.value_element {
            map.serialize_entry(&format!("_{key}"), element)?;
        }
        map.end()
    }
}

impl<'de> Deserialize<'de> for Parameter {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct ParameterVisitor;

        impl<'de> Visitor<'de> for ParameterVisitor {
            type Value = Parameter;

            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("a StructureMap parameter with one value[x] element")
            }

            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Parameter, A::Error> {
                let mut value = None;
                let mut value_element = None;
                while let Some(key) = map.next_key::<String>()? {
                    let json: Value = map.next_value()?;
                    if key.starts_with("_value") {
                        value_element = Some(json);
                    } else if key.starts_with("value") {
                        value = Some(
                            ParameterValue::from_json(&key, json)
                                .map_err(de::Error::custom)?
                                .ok_or_else(|| {
                                    de::Error::custom(format!("unsupported parameter type {key}"))
                                })?,
                        );
                    }
                }
                let value = value.ok_or_else(|| de::Error::custom("parameter has no value[x]"))?;
                Ok(Parameter {
                    value,
                    value_element,
                })
            }
        }

        deserializer.deserialize_map(ParameterVisitor)
    }
}
