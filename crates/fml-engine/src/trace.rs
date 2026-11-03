//! Debug trace of a transform, in the form the Mapping Language IG proposes for `$transform`
//! with `debug=true` (debug.md): `enter` / `step` / `exit` events with their depth, the position
//! in the StructureMap, and a snapshot of the variables in scope.

use fml_model::Span;
use serde_json::{Map, Value, json};

use crate::engine::{Error, ErrorKind};

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
    /// Where the value is in the input, for values that come from it: a FHIRPath of names and
    /// indexes (`Patient.name[0].given[1]`).
    pub path: Option<String>,
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

impl TraceEvent {
    /// Roughly the size of the event as JSON, in bytes: enough to keep a trace within a budget.
    pub fn approximate_size(&self) -> usize {
        let variables: usize = self
            .variables
            .iter()
            .map(|v| v.name.len() + v.context_type.len() + json_size(&v.value) + 96)
            .sum();
        let messages: usize = self.messages.iter().map(|m| m.len() + 48).sum();
        self.position.len()
            + self.source.as_ref().map_or(0, String::len)
            + 128
            + variables
            + messages
    }
}

fn json_size(value: &Value) -> usize {
    match value {
        Value::Null | Value::Bool(_) => 5,
        Value::Number(_) => 12,
        Value::String(text) => text.len() + 2,
        Value::Array(items) => 2 + items.iter().map(|v| json_size(v) + 1).sum::<usize>(),
        Value::Object(map) => {
            2 + map
                .iter()
                .map(|(k, v)| k.len() + 4 + json_size(v))
                .sum::<usize>()
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TraceStatus {
    /// Evaluation ran to the end and every event was kept.
    Complete,
    /// Evaluation went on past the kept events: the trace reached its limits.
    Truncated,
    /// Evaluation stopped on an error; open scopes have no `exit`.
    Stopped,
}

impl TraceStatus {
    pub fn code(self) -> &'static str {
        match self {
            TraceStatus::Complete => "complete",
            TraceStatus::Truncated => "truncated",
            TraceStatus::Stopped => "stopped",
        }
    }
}

/// The recorded events of a transform.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Trace {
    pub status: TraceStatus,
    /// Why events were left out, for a `truncated` trace.
    pub note: Option<String>,
    pub events: Vec<TraceEvent>,
}

/// What a `$transform` with `debug=true` reports.
#[derive(Debug, Clone, Copy)]
pub struct DebugReport<'a> {
    /// The result, when the transform produced one.
    pub result: Option<&'a Value>,
    pub trace: &'a Trace,
    /// The failure, when it failed.
    pub error: Option<&'a Error>,
    /// Values left out of the result ([`Transformed::warnings`](crate::Transformed::warnings)).
    pub warnings: &'a [String],
}

/// Where a part of a map is in its FML text: the map's canonical URL (the trace's `source`) and
/// the part's location (`StructureMap.group[0].rule[1]`). For maps compiled from FML, this comes
/// from [`fml_model::Locations`].
pub type SpanLookup<'a> = &'a dyn Fn(Option<&str>, &str) -> Option<Span>;

/// No text positions: maps that were not compiled from FML here.
pub fn no_spans(_source: Option<&str>, _position: &str) -> Option<Span> {
    None
}

/// The `Parameters` response of `$transform` with `debug=true` (debug.md): `return` when there
/// is a result, `traceStatus`, the `trace` events with their text positions where `spans` knows
/// them, and an `outcome` with the failure, the reason events were left out, and values left out
/// of the result.
pub fn debug_parameters(report: &DebugReport<'_>, spans: SpanLookup<'_>) -> Value {
    let trace = report.trace;
    let mut parameters = Vec::new();
    if let Some(result) = report.result {
        parameters.push(json!({ "name": "return", "resource": result }));
    }
    parameters.push(json!({ "name": "traceStatus", "valueCode": trace.status.code() }));
    for event in &trace.events {
        let mut parts = Vec::new();
        if let Some(source) = &event.source {
            parts.push(json!({ "name": "source", "valueString": source }));
        }
        if let Some(span) = spans(event.source.as_deref(), &event.position) {
            parts.push(json!({ "name": "startRow", "valueInteger": span.start_row }));
            parts.push(json!({ "name": "startColumn", "valueInteger": span.start_column }));
            parts.push(json!({ "name": "endRow", "valueInteger": span.end_row }));
            parts.push(json!({ "name": "endColumn", "valueInteger": span.end_column }));
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
    let mut issues = Vec::new();
    if let Some(error) = report.error {
        let code = match error.kind {
            ErrorKind::Failed => "processing",
            ErrorKind::Limit => "too-costly",
            ErrorKind::Cancelled => "incomplete",
        };
        let mut issue = json!({ "severity": "error", "code": code, "diagnostics": error.message });
        if let Some(position) = &error.position {
            issue["expression"] = json!([position]);
            let mut extensions = Vec::new();
            if let Some(map) = &error.map {
                extensions.push(json!({
                    "url": "http://hl7.org/fhir/StructureDefinition/operationoutcome-file",
                    "valueString": map
                }));
            }
            if let Some(span) = spans(error.map.as_deref(), position) {
                // The location extensions count from 1.
                extensions.push(json!({
                    "url": "http://hl7.org/fhir/StructureDefinition/operationoutcome-issue-line",
                    "valueInteger": span.start_row + 1
                }));
                extensions.push(json!({
                    "url": "http://hl7.org/fhir/StructureDefinition/operationoutcome-issue-col",
                    "valueInteger": span.start_column + 1
                }));
            }
            if !extensions.is_empty() {
                issue["extension"] = Value::Array(extensions);
            }
        }
        issues.push(issue);
    }
    if let Some(note) = &trace.note {
        issues.push(json!({ "severity": "warning", "code": "too-costly", "diagnostics": note }));
    }
    for warning in report.warnings {
        issues.push(
            json!({ "severity": "warning", "code": "informational", "diagnostics": warning }),
        );
    }
    if !issues.is_empty() {
        parameters.push(json!({
            "name": "outcome",
            "resource": { "resourceType": "OperationOutcome", "issue": issues }
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
    if let Some(path) = &variable.path {
        parts.push(json!({ "name": "path", "valueString": path }));
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
