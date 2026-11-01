//! Debug trace of a transform, in the form the Mapping Language IG proposes for `$transform`
//! with `debug=true` (debug.md): `enter` / `step` / `exit` events with their depth, the position
//! in the StructureMap, and a snapshot of the variables in scope.

use serde_json::{Map, Value, json};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Enter,
    Step,
    Exit,
}

impl Phase {
    pub fn code(self) -> &'static str {
        match self {
            Phase::Enter => "enter",
            Phase::Step => "step",
            Phase::Exit => "exit",
        }
    }
}

/// One variable binding at an event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TraceVariable {
    pub name: String,
    /// `input` or `output`.
    pub context_type: String,
    /// The value as FHIR JSON.
    pub value: Value,
    /// The FHIR type, where known: `Patient`, `string`, `QuestionnaireResponse.item`.
    pub data_type: Option<String>,
}

/// One evaluation unit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TraceEvent {
    pub phase: Phase,
    pub depth: usize,
    /// The map the event is in: its canonical URL.
    pub source: Option<String>,
    /// Where in the map, as a FHIRPath location: `StructureMap.group[0].rule[2].target[1]`.
    pub position: String,
    pub variables: Vec<TraceVariable>,
    /// `log (…)` messages produced by this event.
    pub messages: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TraceStatus {
    /// Evaluation ran to the end.
    Complete,
    /// Evaluation stopped on an error; open scopes have no `exit`.
    Stopped,
}

/// The recorded events of a transform.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Trace {
    pub status: TraceStatus,
    pub events: Vec<TraceEvent>,
}

/// The `Parameters` response of `$transform` with `debug=true`: `return` (when there is a
/// result), `traceStatus`, the `trace` events and, on failure, an `outcome`.
pub fn debug_parameters(result: Option<&Value>, trace: &Trace, error: Option<&str>) -> Value {
    let mut parameters = Vec::new();
    if let Some(result) = result {
        parameters.push(json!({ "name": "return", "resource": result }));
    }
    parameters.push(json!({
        "name": "traceStatus",
        "valueCode": match trace.status {
            TraceStatus::Complete => "complete",
            TraceStatus::Stopped => "stopped",
        }
    }));
    for event in &trace.events {
        let mut parts = Vec::new();
        if let Some(source) = &event.source {
            parts.push(json!({ "name": "source", "valueString": source }));
        }
        parts.push(json!({ "name": "expressionPosition", "valueString": event.position }));
        parts.push(json!({ "name": "phase", "valueCode": event.phase.code() }));
        parts.push(json!({ "name": "depth", "valueInteger": event.depth }));
        for variable in &event.variables {
            parts.push(variable_parameter(variable));
        }
        for message in &event.messages {
            parts.push(json!({
                "name": "message",
                "part": [{ "name": "text", "valueString": message }]
            }));
        }
        parameters.push(json!({ "name": "trace", "part": parts }));
    }
    if let Some(error) = error {
        parameters.push(json!({
            "name": "outcome",
            "resource": {
                "resourceType": "OperationOutcome",
                "issue": [{ "severity": "error", "code": "processing", "diagnostics": error }]
            }
        }));
    }
    json!({ "resourceType": "Parameters", "parameter": parameters })
}

/// A variable as a `trace.variable` part: a resource in `resource`, a primitive or data type in
/// `value[x]`, anything else (a backbone element) as `jsonValue`.
fn variable_parameter(variable: &TraceVariable) -> Value {
    let mut parts = vec![
        json!({ "name": "name", "valueString": variable.name }),
        json!({ "name": "contextType", "valueString": variable.context_type }),
    ];
    let data_type = variable.data_type.as_deref();
    let value = &variable.value;
    let is_resource = value.get("resourceType").is_some();
    let is_data_type = data_type.is_some_and(|t| !t.contains('.') && t != "BackboneElement");
    if is_resource {
        parts.push(json!({ "name": "resource", "resource": value }));
    } else if value.is_null() {
        // An unset value: an empty collection.
    } else if let (Some(type_name), true) = (data_type, is_data_type) {
        let mut part = Map::new();
        part.insert("name".into(), "value".into());
        part.insert(format!("value{}", capitalize(type_name)), value.clone());
        parts.push(Value::Object(part));
    } else if !value.is_object() && !value.is_array() {
        parts.push(json!({ "name": "value", "valueString": scalar_text(value) }));
    } else {
        parts.push(json!({ "name": "jsonValue", "valueString": value.to_string() }));
    }
    if let Some(data_type) = data_type {
        parts.push(json!({ "name": "dataType", "valueString": data_type }));
    }
    json!({ "name": "variable", "part": parts })
}

fn scalar_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

fn capitalize(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}
