//! Running maps without FHIRPath: element paths, targets, transforms and groups.

use fml_check::{CoreDefinitions, Release};
use fml_engine::{Engine, MapSet, NoExpressions};
use serde_json::{Value, json};

async fn run(map: &str, source: Value) -> Result<Value, String> {
    let compiled = fml_model::compile(map);
    assert!(!compiled.has_errors(), "{:?}", compiled.diagnostics);
    let definitions = CoreDefinitions::new(Release::R5);
    Engine::new(&definitions, NoExpressions)
        .transform(&compiled.map, &source)
        .await
        .map(|t| t.target)
        .map_err(|e| e.to_string())
}

const HEADER: &str = r#"uses "http://hl7.org/fhir/StructureDefinition/Patient" alias Patient as source
uses "http://hl7.org/fhir/StructureDefinition/Basic" alias Basic as target
"#;

#[tokio::test]
async fn copies_and_creates() {
    let map = format!(
        r#"{HEADER}
group Main(source src : Patient, target tgt : Basic) {{
  src.id as v -> tgt.id = v;
  src.birthDate as d -> tgt.created = d;
  src.name as n -> tgt.extension as ext, ext.url = 'http://example.org/name', ext.value = create('HumanName') as hn then {{
    n.family as f -> hn.family = f;
    n.given as g -> hn.given = g;
  }} "names";
  src -> tgt.code = cc('http://example.org/cs', 'patient', 'Patient') "code";
  src -> tgt.identifier = id('http://example.org/ids', 'b-1') "identifier";
  src -> tgt.extension as ext, ext.url = 'http://example.org/ref', ext.value = create('Reference') as r, r.reference = reference(src) "ref";
}}
"#
    );
    let source = json!({
        "resourceType": "Patient",
        "id": "p1",
        "birthDate": "1970-01-01",
        "name": [
            { "family": "Doe", "given": ["Jane", "Q"] },
            { "family": "Roe" }
        ]
    });
    assert_eq!(
        run(&map, source).await.unwrap(),
        json!({
            "resourceType": "Basic",
            "id": "p1",
            "extension": [
                { "url": "http://example.org/name", "valueHumanName": { "family": "Doe", "given": ["Jane", "Q"] } },
                { "url": "http://example.org/name", "valueHumanName": { "family": "Roe" } },
                { "url": "http://example.org/ref", "valueReference": { "reference": "Patient/p1" } }
            ],
            "identifier": [{ "system": "http://example.org/ids", "value": "b-1" }],
            "code": { "coding": [{ "system": "http://example.org/cs", "code": "patient", "display": "Patient" }] },
            "created": "1970-01-01"
        })
    );
}

#[tokio::test]
async fn choice_sources_filter_by_type_and_list_modes_pick_items() {
    let map = r#"uses "http://hl7.org/fhir/StructureDefinition/Patient" alias Patient as source
uses "http://hl7.org/fhir/StructureDefinition/Patient" alias Patient as target

group Main(source src : Patient, target tgt : Patient) {
  src.deceased : boolean as d -> tgt.deceased = d "deceasedBoolean";
  src.deceased : dateTime as d -> tgt.active = true "deceasedDateTime";
  src.name first as n -> tgt.name = n "firstName";
  src.telecom last as t -> tgt.telecom = t "lastTelecom";
}
"#;
    let source = json!({
        "resourceType": "Patient",
        "deceasedBoolean": true,
        "name": [{ "family": "A" }, { "family": "B" }],
        "telecom": [{ "value": "1" }, { "value": "2" }]
    });
    assert_eq!(
        run(map, source).await.unwrap(),
        json!({
            "resourceType": "Patient",
            "deceasedBoolean": true,
            "name": [{ "family": "A" }],
            "telecom": [{ "value": "2" }]
        })
    );
}

#[tokio::test]
async fn shared_targets_and_dependent_groups() {
    let map = r#"uses "http://hl7.org/fhir/StructureDefinition/Patient" alias Patient as source
uses "http://hl7.org/fhir/StructureDefinition/Patient" alias Patient as target

group Main(source src : Patient, target tgt : Patient) {
  src.name as n -> tgt.contact as c share contact then Contact(n, c) "first";
  src.telecom as t -> tgt.contact as c share contact then Telecom(t, c) "second";
  src -> tgt : gender, birthDate;
}

group Contact(source src, target tgt) {
  src -> tgt.name = src "name";
}

group Telecom(source src, target tgt) {
  src -> tgt.telecom = src "telecom";
}
"#;
    let source = json!({
        "resourceType": "Patient",
        "gender": "female",
        "birthDate": "1980-02-03",
        "name": [{ "family": "Doe" }],
        "telecom": [{ "system": "phone", "value": "555" }]
    });
    assert_eq!(
        run(map, source).await.unwrap(),
        json!({
            "resourceType": "Patient",
            "gender": "female",
            "birthDate": "1980-02-03",
            "contact": [{
                "name": { "family": "Doe" },
                "telecom": [{ "system": "phone", "value": "555" }]
            }]
        })
    );
}

#[tokio::test]
async fn simple_copies_use_default_groups_by_type() {
    let map = r#"uses "http://hl7.org/fhir/StructureDefinition/Patient" alias Patient as source
uses "http://hl7.org/fhir/StructureDefinition/Patient" alias Patient as target

group Main(source src : Patient, target tgt : Patient) {
  src.name -> tgt.name;
  src.active -> tgt.active;
}

group HumanName(source src : HumanName, target tgt : HumanName) <<type+>> {
  src.family as f -> tgt.family = f "family";
  src.family as f -> tgt.text = f "text";
}
"#;
    let source = json!({ "resourceType": "Patient", "active": true, "name": [{ "family": "Doe", "given": ["J"] }] });
    assert_eq!(
        run(map, source).await.unwrap(),
        json!({ "resourceType": "Patient", "active": true, "name": [{ "family": "Doe", "text": "Doe" }] })
    );
}

#[tokio::test]
async fn translate_with_a_contained_concept_map() {
    let map = r#"conceptmap "gender" {
  prefix s = "http://hl7.org/fhir/administrative-gender"
  prefix t = "http://terminology.hl7.org/CodeSystem/v2-0001"
  s:male == t:M
  s:female == t:F
}

uses "http://hl7.org/fhir/StructureDefinition/Patient" alias Patient as source
uses "http://hl7.org/fhir/StructureDefinition/Basic" alias Basic as target

group Main(source src : Patient, target tgt : Basic) {
  src.gender as g -> tgt.code = translate(g, '#gender', 'CodeableConcept') "code";
}
"#;
    assert_eq!(
        run(
            map,
            json!({ "resourceType": "Patient", "gender": "female" })
        )
        .await
        .unwrap(),
        json!({
            "resourceType": "Basic",
            "code": { "coding": [{ "system": "http://terminology.hl7.org/CodeSystem/v2-0001", "code": "F" }] }
        })
    );
}

#[tokio::test]
async fn imported_groups_resolve_through_the_map_set() {
    let library = fml_model::compile(
        r#"/// url = 'http://example.org/StructureMap/lib'
/// name = 'lib'
group CopyName(source src, target tgt) {
  src.family as f -> tgt.family = f "family";
}
"#,
    )
    .map;
    let mut maps = MapSet::new();
    maps.add(library);
    let main = fml_model::compile(
        r#"/// url = 'http://example.org/StructureMap/main'
/// name = 'main'
uses "http://hl7.org/fhir/StructureDefinition/Patient" alias Patient as source
uses "http://hl7.org/fhir/StructureDefinition/Patient" alias Patient as target
imports "http://example.org/StructureMap/*"

group Main(source src : Patient, target tgt : Patient) {
  src.name as n -> tgt.name as t then CopyName(n, t) "name";
}
"#,
    )
    .map;
    let definitions = CoreDefinitions::new(Release::R5);
    let result = Engine::new(&definitions, NoExpressions)
        .with_maps(&maps)
        .transform(
            &main,
            &json!({ "resourceType": "Patient", "name": [{ "family": "X", "given": ["Y"] }] }),
        )
        .await
        .unwrap();
    assert_eq!(
        result.target,
        json!({ "resourceType": "Patient", "name": [{ "family": "X" }] })
    );
}

#[tokio::test]
async fn reports_errors_with_the_rule() {
    let map = format!(
        "{HEADER}\ngroup Main(source src : Patient, target tgt : Basic) {{\n  src.id as v -> tgt.nope = v \"bad\";\n}}\n"
    );
    assert_eq!(
        run(&map, json!({ "resourceType": "Patient", "id": "1" })).await,
        Err("rule `bad`: `Basic` has no element `nope`".to_owned())
    );
    let map = format!(
        "{HEADER}\ngroup Main(source src : Patient, target tgt : Basic) {{\n  src.id as v where (v.length() > 1) -> tgt.id = v \"needsFhirPath\";\n}}\n"
    );
    let error = run(&map, json!({ "resourceType": "Patient", "id": "1" }))
        .await
        .unwrap_err();
    assert!(error.contains("needs FHIRPath"), "{error}");
}

#[tokio::test]
async fn date_time_and_escape_transforms() {
    let map = format!(
        r#"{HEADER}
group Main(source src : Patient, target tgt : Basic) {{
  src.id as v -> tgt.created = toDate(v, 'dd/MM/yyyy') "date";
  src.id as v -> tgt.extension as e, e.url = 'http://example.org/when', e.value = unixToDateTime(1714570200, '+10:00') "unix";
  src.id as v -> tgt.extension as e, e.url = 'http://example.org/text', e.value = escape('a < b', 'none', 'html') "escaped";
}}
"#
    );
    assert_eq!(
        run(
            &map,
            json!({ "resourceType": "Patient", "id": "02/03/1980" })
        )
        .await
        .unwrap(),
        json!({
            "resourceType": "Basic",
            "extension": [
                { "url": "http://example.org/when", "valueDateTime": "2024-05-01T23:30:00+10:00" },
                { "url": "http://example.org/text", "valueString": "a &lt; b" }
            ],
            "created": "1980-03-02"
        })
    );
}

#[tokio::test]
async fn resources_created_outside_the_target_are_produced() {
    let compiled = fml_model::compile(&format!(
        r#"{HEADER}
group Main(source src : Patient, target tgt : Basic) {{
  src.id as v -> create('Observation') as obs, obs.status = 'final', obs.id = v, tgt.id = v "split";
  src.id as v -> create('Condition') as kept, tgt.extension as e, e.url = 'http://example.org/c' "notProduced";
}}
"#
    ));
    assert!(!compiled.has_errors(), "{:?}", compiled.diagnostics);
    let definitions = CoreDefinitions::new(Release::R5);
    let result = Engine::new(&definitions, NoExpressions)
        .transform(
            &compiled.map,
            &json!({ "resourceType": "Patient", "id": "p1" }),
        )
        .await
        .unwrap();
    assert_eq!(
        result.target,
        json!({
            "resourceType": "Basic",
            "id": "p1",
            "extension": [{ "url": "http://example.org/c" }]
        })
    );
    // Both are produced: neither was put into the target (the Condition stays empty).
    assert_eq!(
        result.produced,
        vec![
            json!({ "resourceType": "Observation", "id": "p1", "status": "final" }),
            json!({ "resourceType": "Condition" })
        ]
    );
}

#[tokio::test]
async fn traces_follow_the_stack_rules() {
    use fml_engine::{Phase, TraceStatus};
    let compiled = fml_model::compile(&format!(
        "{HEADER}\ngroup Main(source src : Patient, target tgt : Basic) {{\n  src.name as n -> tgt.extension as e then Name(n, e) \"names\";\n}}\ngroup Name(source n, target e) {{\n  n.family as f -> e.url = f \"family\";\n}}\n"
    ));
    let definitions = CoreDefinitions::new(Release::R5);
    let engine = Engine::new(&definitions, NoExpressions).with_trace(true);
    let source =
        json!({ "resourceType": "Patient", "name": [{ "family": "A" }, { "family": "B" }] });
    let trace = engine
        .transform(&compiled.map, &source)
        .await
        .unwrap()
        .trace
        .unwrap();
    assert_eq!(trace.status, TraceStatus::Complete);
    // Every enter has a matching exit at the same depth; steps are one deeper than their scope.
    let mut stack: Vec<(usize, String)> = Vec::new();
    for event in &trace.events {
        match event.phase {
            Phase::Enter => {
                assert_eq!(event.depth, stack.len());
                stack.push((event.depth, event.position.clone()));
            }
            Phase::Step => assert_eq!(event.depth, stack.len()),
            Phase::Exit => {
                let (depth, position) = stack.pop().expect("an exit without an enter");
                assert_eq!((event.depth, &event.position), (depth, &position));
            }
        }
    }
    assert!(stack.is_empty());
    assert_eq!(
        trace.events.first().unwrap().position,
        "StructureMap.group[0]"
    );
    assert!(
        trace
            .events
            .iter()
            .any(|e| e.position == "StructureMap.group[1].rule[0].target[0]")
    );

    // On an error the trace stops where evaluation stopped, without closing open scopes.
    let broken = fml_model::compile(&format!(
        "{HEADER}\ngroup Main(source src : Patient, target tgt : Basic) {{\n  src.name as n -> tgt.nope = n \"bad\";\n}}\n"
    ));
    let error = engine.transform(&broken.map, &source).await.unwrap_err();
    let trace = error.trace.unwrap();
    assert_eq!(trace.status, TraceStatus::Stopped);
    assert_eq!(trace.events.last().unwrap().phase, Phase::Step);
}
