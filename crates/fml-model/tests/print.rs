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
