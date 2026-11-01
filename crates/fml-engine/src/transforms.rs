//! The data-type building transforms: `cc`, `c`, `id`, `cp`, `qty`, and `cast` conversions.

use serde_json::{Map, Value, json};

/// `cc(text)` or `cc(system, code [, display])`: a CodeableConcept.
pub fn codeable_concept(arguments: &[String]) -> Value {
    match arguments {
        [text] => json!({ "text": text }),
        [system, code, rest @ ..] => json!({ "coding": [coding(system, code, rest.first())] }),
        [] => json!({}),
    }
}

/// `c(system, code [, display])`: a Coding.
pub fn coding(system: &str, code: &str, display: Option<&String>) -> Value {
    let mut coding = Map::new();
    coding.insert("system".into(), system.into());
    coding.insert("code".into(), code.into());
    if let Some(display) = display {
        coding.insert("display".into(), display.as_str().into());
    }
    Value::Object(coding)
}

/// `id(system, value [, type])`: an Identifier; the type is a v2-0203 code.
pub fn identifier(arguments: &[String]) -> Value {
    let mut identifier = Map::new();
    if let Some(system) = arguments.first() {
        identifier.insert("system".into(), system.as_str().into());
    }
    if let Some(value) = arguments.get(1) {
        identifier.insert("value".into(), value.as_str().into());
    }
    if let Some(code) = arguments.get(2) {
        identifier.insert(
            "type".into(),
            json!({ "coding": [{
                "system": "http://terminology.hl7.org/CodeSystem/v2-0203",
                "code": code,
            }]}),
        );
    }
    Value::Object(identifier)
}

/// `cp(system, value)` or `cp(value)`, where the system is guessed from the value.
pub fn contact_point(arguments: &[String]) -> Value {
    match arguments {
        [system, value, ..] => json!({ "system": system, "value": value }),
        [value] => {
            let system = if value.contains('@') {
                "email"
            } else if value.starts_with("http://") || value.starts_with("https://") {
                "url"
            } else {
                "phone"
            };
            let value = value.strip_prefix("mailto:").unwrap_or(value);
            let value = value.strip_prefix("tel:").unwrap_or(value);
            json!({ "system": system, "value": value })
        }
        [] => json!({}),
    }
}

/// `qty(text)`, `qty(value, unit)` or `qty(value, unit, system, code)`: a Quantity.
pub fn quantity(arguments: &[String]) -> Result<Value, String> {
    let mut quantity = Map::new();
    match arguments {
        [text] => {
            let mut rest = text.trim();
            for comparator in ["<=", ">=", "<", ">", "ad"] {
                if let Some(after) = rest.strip_prefix(comparator) {
                    quantity.insert("comparator".into(), comparator.into());
                    rest = after.trim_start();
                    break;
                }
            }
            let (value, unit) = match rest.split_once(char::is_whitespace) {
                Some((value, unit)) => (value, Some(unit.trim().trim_matches('\''))),
                None => (rest, None),
            };
            quantity.insert("value".into(), number(value)?);
            if let Some(unit) = unit {
                quantity.insert("unit".into(), unit.into());
            }
        }
        [value, unit, rest @ ..] => {
            quantity.insert("value".into(), number(value)?);
            quantity.insert("unit".into(), unit.as_str().into());
            if let [system, code] = rest {
                quantity.insert("system".into(), system.as_str().into());
                quantity.insert("code".into(), code.as_str().into());
            }
        }
        [] => return Err("qty needs a value".into()),
    }
    Ok(Value::Object(quantity))
}

fn number(text: &str) -> Result<Value, String> {
    text.parse::<serde_json::Number>()
        .map(Value::Number)
        .map_err(|_| format!("`{text}` is not a number"))
}

/// `cast(value, type)`: the value as a primitive of `type`, with the JSON representation FHIR uses
/// for it.
pub fn cast(value: &str, type_name: &str) -> Result<Value, String> {
    Ok(match type_name {
        "boolean" => match value {
            "true" => Value::Bool(true),
            "false" => Value::Bool(false),
            _ => return Err(format!("cannot cast `{value}` to boolean")),
        },
        "integer" | "positiveInt" | "unsignedInt" => value
            .parse::<i64>()
            .map(Value::from)
            .map_err(|_| format!("cannot cast `{value}` to {type_name}"))?,
        "decimal" => number(value)?,
        "string" | "integer64" | "uri" | "base64Binary" | "instant" | "date" | "dateTime"
        | "time" | "code" | "oid" | "id" | "markdown" | "uuid" | "url" | "canonical" => {
            Value::String(value.to_owned())
        }
        other => return Err(format!("cast to {other} is not supported")),
    })
}
