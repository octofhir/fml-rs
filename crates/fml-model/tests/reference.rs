//! Compares our StructureMaps with the HL7 reference compiler's, map by map.
//!
//! `scripts/reference.sh` builds the reference output into `reference/maps`; `FML_REFERENCE`
//! points elsewhere. Without it the test does nothing.

use std::path::{Path, PathBuf};

use fml_model::compile;
use serde_json::Value;

fn reference_dir() -> PathBuf {
    match std::env::var_os("FML_REFERENCE") {
        Some(dir) => PathBuf::from(dir),
        None => Path::new(env!("CARGO_MANIFEST_DIR")).join("../../reference/maps"),
    }
}

/// The known, deliberate difference, applied to our output before comparing: the reference
/// compiler writes the calendar duration `5 days` as `5 'days'`, a quantity with the (non-UCUM)
/// unit `days`, which FHIRPath does not treat as the same thing. We keep the calendar duration.
/// (Whitespace-only strings such as the separator in `append(a, ' ', b)`, which older reference
/// JSON writers dropped, are kept by both now.)
fn align_with_reference(value: &mut Value) {
    match value {
        Value::String(text) => *text = quote_calendar_units(text),
        Value::Object(map) => {
            map.values_mut().for_each(align_with_reference);
        }
        Value::Array(items) => items.iter_mut().for_each(align_with_reference),
        _ => {}
    }
}

const CALENDAR_UNITS: &[&str] = &[
    "years",
    "months",
    "weeks",
    "days",
    "hours",
    "minutes",
    "seconds",
    "milliseconds",
    "year",
    "month",
    "week",
    "day",
    "hour",
    "minute",
    "second",
    "millisecond",
];

/// `5 days` → `5 'days'`, the reference compiler's rendering of a calendar duration.
fn quote_calendar_units(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    'scan: while !rest.is_empty() {
        if out.ends_with(|c: char| c.is_ascii_digit()) && rest.starts_with(' ') {
            for unit in CALENDAR_UNITS {
                if let Some(after) = rest[1..].strip_prefix(unit)
                    && !after.starts_with(|c: char| c.is_alphanumeric() || c == '_')
                {
                    out.push_str(&format!(" '{unit}'"));
                    rest = after;
                    continue 'scan;
                }
            }
        }
        let c = rest.chars().next().unwrap();
        out.push(c);
        rest = &rest[c.len_utf8()..];
    }
    out
}

#[test]
fn matches_the_reference_compiler() {
    let dir = reference_dir();
    let Ok(index) = std::fs::read_to_string(dir.join("index.tsv")) else {
        eprintln!("no reference output; run scripts/reference.sh");
        return;
    };
    let mut compared = 0;
    let mut different = Vec::new();
    for line in index.lines() {
        let Some((number, file)) = line.split_once('\t') else {
            continue;
        };
        let Ok(expected) = std::fs::read_to_string(dir.join(format!("{number}.json"))) else {
            continue;
        };
        let Ok(text) = std::fs::read_to_string(file) else {
            continue;
        };
        let expected: Value = serde_json::from_str(&expected).unwrap();
        let mut actual = serde_json::to_value(compile(&text).map).unwrap();
        align_with_reference(&mut actual);
        compared += 1;
        if actual != expected {
            different.push(file.to_owned());
        }
    }
    eprintln!(
        "reference: {compared} maps compared, {} different",
        different.len()
    );
    assert!(different.is_empty(), "{}", different.join("\n"));
}
