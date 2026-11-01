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

/// A transform or dependent-group parameter: `value[x]`.
#[derive(Debug, Clone, PartialEq)]
pub enum Parameter {
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

impl Parameter {
    fn key(&self) -> &'static str {
        match self {
            Parameter::Id(_) => "valueId",
            Parameter::String(_) => "valueString",
            Parameter::Boolean(_) => "valueBoolean",
            Parameter::Integer(_) => "valueInteger",
            Parameter::Decimal(_) => "valueDecimal",
            Parameter::Date(_) => "valueDate",
            Parameter::Time(_) => "valueTime",
            Parameter::DateTime(_) => "valueDateTime",
        }
    }

    fn value(&self) -> Value {
        match self {
            Parameter::Id(v)
            | Parameter::String(v)
            | Parameter::Date(v)
            | Parameter::Time(v)
            | Parameter::DateTime(v) => Value::String(v.clone()),
            Parameter::Boolean(v) => Value::Bool(*v),
            Parameter::Integer(v) => Value::from(*v),
            Parameter::Decimal(v) => Value::Number(v.clone()),
        }
    }
}

impl Serialize for Parameter {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(1))?;
        map.serialize_entry(self.key(), &self.value())?;
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
                let mut found = None;
                while let Some(key) = map.next_key::<String>()? {
                    let value: Value = map.next_value()?;
                    if !key.starts_with("value") {
                        continue;
                    }
                    let text = || {
                        value
                            .as_str()
                            .map(str::to_owned)
                            .ok_or_else(|| de::Error::custom(format!("{key} must be a string")))
                    };
                    found = Some(match key.as_str() {
                        "valueId" => Parameter::Id(text()?),
                        "valueString" => Parameter::String(text()?),
                        "valueDate" => Parameter::Date(text()?),
                        "valueTime" => Parameter::Time(text()?),
                        "valueDateTime" => Parameter::DateTime(text()?),
                        "valueBoolean" => {
                            Parameter::Boolean(value.as_bool().ok_or_else(|| {
                                de::Error::custom("valueBoolean must be a boolean")
                            })?)
                        }
                        "valueInteger" => {
                            Parameter::Integer(value.as_i64().ok_or_else(|| {
                                de::Error::custom("valueInteger must be an integer")
                            })?)
                        }
                        "valueDecimal" => match value {
                            Value::Number(n) => Parameter::Decimal(n),
                            _ => return Err(de::Error::custom("valueDecimal must be a number")),
                        },
                        other => {
                            return Err(de::Error::custom(format!(
                                "unsupported parameter type {other}"
                            )));
                        }
                    });
                }
                found.ok_or_else(|| de::Error::custom("parameter has no value[x]"))
            }
        }

        deserializer.deserialize_map(ParameterVisitor)
    }
}
