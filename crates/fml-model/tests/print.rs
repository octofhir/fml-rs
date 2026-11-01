//! StructureMap JSON → FML.

use fml_model::{StructureMap, compile, print};

#[test]
fn prints_legacy_default_group_copies_in_short_form() {
    let map: StructureMap = serde_json::from_value(serde_json::json!({
        "resourceType": "StructureMap",
        "url": "http://example.org/StructureMap/Legacy",
        "name": "Legacy",
        "status": "active",
        "group": [{
            "name": "Patient",
            "input": [
                { "name": "src", "mode": "source" },
                { "name": "tgt", "mode": "target" }
            ],
            "rule": [{
                "name": "identifier",
                "source": [{ "context": "src", "element": "identifier", "variable": "vvv" }],
                "target": [{
                    "context": "tgt", "element": "identifier", "variable": "vvv",
                    "transform": "create"
                }],
                "dependent": [{
                    "name": "DefaultMappingGroupAnonymousAlias",
                    "parameter": [{ "valueId": "vvv" }, { "valueId": "vvv" }]
                }]
            }, {
                "name": "status",
                "source": [{ "context": "src", "element": "status", "variable": "v" }],
                "target": [{
                    "context": "tgt", "element": "status", "transform": "translate",
                    "parameter": [
                        { "valueId": "v" },
                        { "valueString": "http://example.org/ConceptMap/status" },
                        { "valueString": "code" }
                    ]
                }]
            }]
        }]
    }))
    .unwrap();
    insta::assert_snapshot!(print(&map), @r"
    /// url = 'http://example.org/StructureMap/Legacy'
    /// name = 'Legacy'
    /// status = 'active'

    group Patient(source src, target tgt) {
      src.identifier -> tgt.identifier;
      src.status as v -> tgt.status = translate(v, 'http://example.org/ConceptMap/status', 'code');
    }
    ");
}

#[test]
fn keeps_unknown_elements_when_reading_json() {
    let json = serde_json::json!({
        "resourceType": "StructureMap",
        "url": "http://example.org/StructureMap/X",
        "name": "X",
        "status": "draft",
        "jurisdiction": [{ "coding": [{ "system": "urn:iso:std:iso:3166", "code": "US" }] }],
        "group": [{ "name": "G", "input": [{ "name": "s", "mode": "source" }] }]
    });
    let map: StructureMap = serde_json::from_value(json.clone()).unwrap();
    assert_eq!(serde_json::to_value(&map).unwrap(), json);
}

#[test]
fn printed_map_compiles_to_the_same_structure_map() {
    let source = r#"/// url = 'http://example.org/StructureMap/RoundTrip'
/// name = 'RoundTrip'
/// status = 'draft'

uses "http://hl7.org/fhir/StructureDefinition/Patient" as source
uses "http://hl7.org/fhir/StructureDefinition/Patient" as target

group RoundTrip(source src : Patient, target tgt : Patient) {
  src -> tgt: id, active "Simple";
  src.name as n where (n.use = 'official') -> tgt.name as t then {
    n.given as g -> t.given = g;
  } "officialName";
}
"#;
    let first = compile(source);
    assert!(!first.has_errors(), "{:?}", first.diagnostics);
    let printed = print(&first.map);
    assert_eq!(printed, source);
}

#[test]
fn reads_and_writes_the_r4_form() {
    let compiled = compile(
        r#"conceptmap "g" {
  prefix s = "http://hl7.org/fhir/administrative-gender"
  prefix t = "http://terminology.hl7.org/CodeSystem/v2-0001"
  s:male <= t:M
}

group G(source src, target tgt) {
  src.a default ('x') as a -> tgt.b = a "copy";
  src.c as c -> tgt.d as d then Inner(c, d) "call";
}

group Inner(source s, target t) {
  s.e -> t.e;
}
"#,
    );
    assert!(!compiled.has_errors(), "{:?}", compiled.diagnostics);
    let r4 = fml_model::to_r4(&compiled.map);
    assert!(r4.dropped.is_empty());
    let json = &r4.json;
    let rules = &json["group"][0]["rule"];
    assert_eq!(json["group"][0]["typeMode"], "none");
    assert_eq!(rules[0]["source"][0]["defaultValueString"], "x");
    assert_eq!(rules[0]["target"][0]["contextType"], "variable");
    assert_eq!(
        rules[1]["dependent"][0]["variable"],
        serde_json::json!(["c", "d"])
    );
    assert_eq!(
        json["contained"][0]["group"][0]["element"][0]["target"][0]["equivalence"],
        "wider"
    );
    assert_eq!(fml_model::from_json(r4.json.clone()).unwrap(), compiled.map);
}
