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

/// The FHIR type a `cast` names. The default namespace is FHIR (transforms.md, "cast"); a
/// FHIRPath system type becomes the FHIR primitive that holds it.
pub fn cast_type(name: &str) -> String {
    if let Some(name) = name.strip_prefix("FHIR.") {
        return name.to_owned();
    }
    match name {
        "System.Boolean" => "boolean",
        "System.String" => "string",
        "System.Integer" => "integer",
        "System.Long" => "integer64",
        "System.Decimal" => "decimal",
        "System.Date" => "date",
        "System.DateTime" => "dateTime",
        "System.Time" => "time",
        other => other,
    }
    .to_owned()
}

/// `cast(value, type)`: the value as a primitive of `type`, with the JSON representation FHIR uses
/// for it. The value must be valid for the type.
pub fn cast(value: &str, type_name: &str) -> Result<Value, String> {
    let invalid = || format!("cannot cast `{value}` to {type_name}");
    let valid = |ok: bool| {
        if ok {
            Ok(Value::String(value.to_owned()))
        } else {
            Err(invalid())
        }
    };
    match type_name {
        "boolean" => match value {
            "true" => Ok(Value::Bool(true)),
            "false" => Ok(Value::Bool(false)),
            _ => Err(invalid()),
        },
        "integer" | "positiveInt" | "unsignedInt" => {
            let number: i32 = value.trim().parse().map_err(|_| invalid())?;
            let minimum = match type_name {
                "positiveInt" => 1,
                "unsignedInt" => 0,
                _ => i32::MIN,
            };
            if number < minimum {
                return Err(invalid());
            }
            Ok(Value::from(number))
        }
        "integer64" => valid(value.trim().parse::<i64>().is_ok()),
        "decimal" => number(value.trim()).map_err(|_| invalid()),
        "date" => valid(is_date(value)),
        "dateTime" => valid(is_date_time(value)),
        "instant" => valid(is_instant(value)),
        "time" => valid(is_time(value)),
        "id" => valid(
            (1..=64).contains(&value.len())
                && value
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.'),
        ),
        "code" => valid(!value.is_empty() && value.trim() == value && !value.contains("  ")),
        "string" | "markdown" | "uri" | "url" | "canonical" | "oid" | "uuid" | "base64Binary" => {
            valid(!value.is_empty())
        }
        other => Err(format!("cast to {other} is not supported")),
    }
}

/// `YYYY`, `YYYY-MM` or `YYYY-MM-DD`.
fn is_date(text: &str) -> bool {
    let parts: Vec<&str> = text.split('-').collect();
    let digits =
        |part: &str, len: usize| part.len() == len && part.bytes().all(|b| b.is_ascii_digit());
    let in_range = |part: &str, low: u32, high: u32| {
        part.parse::<u32>().is_ok_and(|n| (low..=high).contains(&n))
    };
    match parts.as_slice() {
        [year] => digits(year, 4),
        [year, month] => digits(year, 4) && digits(month, 2) && in_range(month, 1, 12),
        [year, month, day] => {
            digits(year, 4)
                && digits(month, 2)
                && in_range(month, 1, 12)
                && digits(day, 2)
                && in_range(day, 1, 31)
        }
        _ => false,
    }
}

/// `hh:mm:ss` with optional fractional seconds.
fn is_time(text: &str) -> bool {
    let (clock, fraction) = match text.split_once('.') {
        Some((clock, fraction)) => (clock, Some(fraction)),
        None => (text, None),
    };
    let parts: Vec<&str> = clock.split(':').collect();
    let field =
        |part: &str, high: u32| part.len() == 2 && part.parse::<u32>().is_ok_and(|n| n <= high);
    matches!(parts.as_slice(), [h, m, s] if field(h, 23) && field(m, 59) && field(s, 60))
        && fraction.is_none_or(|f| !f.is_empty() && f.bytes().all(|b| b.is_ascii_digit()))
}

/// `Z` or `±hh:mm`.
fn is_zone(text: &str) -> bool {
    text == "Z"
        || (text.len() == 6
            && matches!(&text[..1], "+" | "-")
            && &text[3..4] == ":"
            && text[1..3].parse::<u32>().is_ok_and(|h| h <= 14)
            && text[4..].parse::<u32>().is_ok_and(|m| m <= 59))
}

/// A full date and time with a zone.
fn is_instant(text: &str) -> bool {
    let Some((date, rest)) = text.split_once('T') else {
        return false;
    };
    let split = rest.find(['Z', '+', '-']).unwrap_or(rest.len());
    date.len() == 10 && is_date(date) && is_time(&rest[..split]) && is_zone(&rest[split..])
}

/// A date, or a full date with a time and a zone.
fn is_date_time(text: &str) -> bool {
    if text.contains('T') {
        is_instant(text)
    } else {
        is_date(text)
    }
}
