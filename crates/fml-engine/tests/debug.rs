//! The debug trace in the form of the Mapping Language IG (debug.md), the limits of a run,
//! cancellation, repeatable ids, and `translate` through a terminology service.

use std::sync::atomic::AtomicBool;

use fml_check::{CoreDefinitions, Release};
use fml_engine::{
    DebugReport, Engine, ErrorKind, Limits, NoExpressions, SequentialIds, TraceStatus,
    TranslateFuture, TranslateService, Translation, debug_parameters, no_spans,
};
use fml_syntax::LineIndex;
use serde_json::{Value, json};

const MAP: &str = r#"/// url = 'http://example.org/StructureMap/names'
/// name = 'names'
uses "http://hl7.org/fhir/StructureDefinition/Patient" alias Patient as source
uses "http://hl7.org/fhir/StructureDefinition/Basic" alias Basic as target

group Main(source src : Patient, target tgt : Basic) {
  src.name as n -> tgt.extension as e then Name(n, e) "names";
}

group Name(source n, target e) {
  n.family as f -> e.url = f "family";
}
"#;

fn patient() -> Value {
    json!({ "resourceType": "Patient", "name": [{ "family": "A" }, { "family": "B" }] })
}

/// The parts of a Parameters parameter, by name.
fn part<'a>(parameter: &'a Value, name: &str) -> Option<&'a Value> {
    parameter["part"]
        .as_array()?
        .iter()
        .find(|p| p["name"] == name)
}

#[tokio::test]
async fn trace_events_carry_text_positions_and_failures_their_location() {
    let compiled = fml_model::compile(MAP);
    let index = LineIndex::new(MAP);
    let spans = |source: Option<&str>, position: &str| {
        (source == Some("http://example.org/StructureMap/names"))
            .then(|| compiled.locations.span(MAP, &index, position))
            .flatten()
    };
    let definitions = CoreDefinitions::new(Release::R5);
    let engine = Engine::new(&definitions, NoExpressions).with_trace(true);
    let transformed = engine.transform(&compiled.map, &patient()).await.unwrap();
    let trace = transformed.trace.as_ref().unwrap();
    let parameters = debug_parameters(
        &DebugReport {
            result: Some(&transformed.target),
            trace,
            error: None,
            warnings: &[],
        },
        &spans,
    );
    let events: Vec<&Value> = parameters["parameter"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|p| p["name"] == "trace")
        .collect();
    let family = events
        .iter()
        .find(|e| {
            part(e, "expressionPosition").unwrap()["valueString"]
                == "StructureMap.group[1].rule[0].target[0]"
        })
        .unwrap();
    let at = |name: &str| {
        part(family, name).unwrap()["valueInteger"]
            .as_u64()
            .unwrap()
    };
    assert_eq!(
        (
            at("startRow"),
            at("startColumn"),
            at("endRow"),
            at("endColumn")
        ),
        (10, 19, 10, 28)
    );
    assert_eq!(
        parameters["parameter"][1],
        json!({ "name": "traceStatus", "valueCode": "complete" })
    );

    let broken = MAP.replace("e.url = f", "e.nope = f");
    let compiled = fml_model::compile(&broken);
    let index = LineIndex::new(&broken);
    let spans =
        |_: Option<&str>, position: &str| compiled.locations.span(&broken, &index, position);
    let error = engine
        .transform(&compiled.map, &patient())
        .await
        .unwrap_err();
    assert_eq!(error.kind, ErrorKind::Failed);
    assert_eq!(
        error.position.as_deref(),
        Some("StructureMap.group[1].rule[0].target[0]")
    );
    let parameters = debug_parameters(
        &DebugReport {
            result: None,
            trace: error.trace.as_deref().unwrap(),
            error: Some(&error),
            warnings: &[],
        },
        &spans,
    );
    let outcome = parameters["parameter"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == "outcome")
        .unwrap();
    let issue = &outcome["resource"]["issue"][0];
    assert_eq!(issue["severity"], "error");
    assert_eq!(
        issue["expression"],
        json!(["StructureMap.group[1].rule[0].target[0]"])
    );
    assert_eq!(issue["extension"][1]["valueInteger"], 11);
    assert_eq!(parameters["parameter"][0]["valueCode"], "stopped");
}

#[tokio::test]
async fn a_trace_past_its_limits_keeps_a_prefix_and_says_why() {
    let compiled = fml_model::compile(MAP);
    let definitions = CoreDefinitions::new(Release::R5);
    let engine = Engine::new(&definitions, NoExpressions)
        .with_trace(true)
        .with_limits(Limits {
            max_trace_events: 4,
            ..Limits::default()
        });
    let transformed = engine.transform(&compiled.map, &patient()).await.unwrap();
    assert_eq!(
        transformed.target["extension"],
        json!([{ "url": "A" }, { "url": "B" }])
    );
    let trace = transformed.trace.unwrap();
    assert_eq!(trace.status, TraceStatus::Truncated);
    assert_eq!(trace.events.len(), 4);
    let parameters = debug_parameters(
        &DebugReport {
            result: Some(&transformed.target),
            trace: &trace,
            error: None,
            warnings: &["left out".to_owned()],
        },
        &no_spans,
    );
    let outcome = parameters["parameter"].as_array().unwrap().last().unwrap();
    assert_eq!(outcome["resource"]["issue"][0]["code"], "too-costly");
    assert_eq!(outcome["resource"]["issue"][1]["diagnostics"], "left out");
}

#[tokio::test]
async fn runs_stop_at_their_limits_and_when_cancelled() {
    let compiled = fml_model::compile(MAP);
    let definitions = CoreDefinitions::new(Release::R5);
    let error = Engine::new(&definitions, NoExpressions)
        .with_limits(Limits {
            max_rules: 2,
            ..Limits::default()
        })
        .transform(&compiled.map, &patient())
        .await
        .unwrap_err();
    assert_eq!(error.kind, ErrorKind::Limit);
    assert_eq!(error.message, "the transform ran more than 2 rules");

    let cancel = AtomicBool::new(true);
    let error = Engine::new(&definitions, NoExpressions)
        .with_cancellation(&cancel)
        .transform(&compiled.map, &patient())
        .await
        .unwrap_err();
    assert_eq!(error.kind, ErrorKind::Cancelled);
}

#[tokio::test]
async fn ids_come_from_the_id_generator() {
    let compiled = fml_model::compile(
        r#"uses "http://hl7.org/fhir/StructureDefinition/Patient" alias Patient as source
uses "http://hl7.org/fhir/StructureDefinition/Basic" alias Basic as target

group Main(source src : Patient, target tgt : Basic) {
  src -> tgt.id = uuid() "id";
  src -> tgt.subject = create('Reference') as r, r.reference = reference(src) "subject";
}
"#,
    );
    let definitions = CoreDefinitions::new(Release::R5);
    let ids = SequentialIds::new();
    let transformed = Engine::new(&definitions, NoExpressions)
        .with_ids(&ids)
        .transform(&compiled.map, &json!({ "resourceType": "Patient" }))
        .await
        .unwrap();
    assert_eq!(
        transformed.target,
        json!({
            "resourceType": "Basic",
            "id": "00000000-0000-0000-0000-000000000001",
            "subject": { "reference": "Patient/00000000-0000-0000-0000-000000000002" }
        })
    );
}

/// A terminology service that knows one ConceptMap.
struct Terminology;

impl TranslateService for Terminology {
    fn translate<'a>(
        &'a self,
        url: &'a str,
        _system: Option<&'a str>,
        code: &'a str,
    ) -> TranslateFuture<'a> {
        Box::pin(async move {
            if url != "http://example.org/ConceptMap/gender" {
                return Err(format!("unknown ConceptMap {url}"));
            }
            Ok((code == "female").then(|| Translation {
                system: Some("http://example.org/sex".to_owned()),
                code: Some("F".to_owned()),
                display: None,
            }))
        })
    }
}

#[tokio::test]
async fn translate_asks_the_terminology_service_for_maps_it_does_not_hold() {
    let compiled = fml_model::compile(
        r#"uses "http://hl7.org/fhir/StructureDefinition/Patient" alias Patient as source
uses "http://hl7.org/fhir/StructureDefinition/Basic" alias Basic as target

group Main(source src : Patient, target tgt : Basic) {
  src.gender as g -> tgt.code = translate(g, 'http://example.org/ConceptMap/gender', 'CodeableConcept') "code";
}
"#,
    );
    let definitions = CoreDefinitions::new(Release::R5);
    let terminology = Terminology;
    let engine = Engine::new(&definitions, NoExpressions).with_translator(&terminology);
    let transformed = engine
        .transform(
            &compiled.map,
            &json!({ "resourceType": "Patient", "gender": "female" }),
        )
        .await
        .unwrap();
    assert_eq!(
        transformed.target["code"],
        json!({ "coding": [{ "system": "http://example.org/sex", "code": "F" }] })
    );
    let left_out = engine
        .transform(
            &compiled.map,
            &json!({ "resourceType": "Patient", "gender": "male" }),
        )
        .await
        .unwrap();
    assert_eq!(left_out.warnings.len(), 1);
}
