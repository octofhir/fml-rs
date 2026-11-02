//! StructureMap produced for each FML construct, following the HL7 reference compiler.

use fml_model::{compile, print};

fn map_json(source: &str) -> serde_json::Value {
    let compiled = compile(source);
    assert!(!compiled.has_errors(), "{:?}", compiled.diagnostics);
    let printed = print(&compiled.map);
    let again = compile(&printed);
    assert_eq!(again.map, compiled.map, "round trip through:\n{printed}");
    serde_json::to_value(&compiled.map).unwrap()
}

#[test]
fn header() {
    insta::assert_json_snapshot!(map_json(
        r#"/// url = 'http://example.org/StructureMap/Demo'
/// name = 'Demo Map'
/// title = 'Demo'
/// status = 'active'
/// experimental = true

// source structure
uses "http://hl7.org/fhir/StructureDefinition/Patient" alias Pat as source // trailing
uses "http://hl7.org/fhir/StructureDefinition/Basic" as target
imports "http://example.org/StructureMap/*"
let prefix = 'urn:uuid:';

// The main group
group Demo(source src : Pat, target tgt : Basic) <<types>> {
  src.id -> tgt.id;
}
"#
    ));
}

#[test]
fn rules() {
    insta::assert_json_snapshot!(map_json(
        r#"group G(source src, target tgt) extends Base <<type+>> {
  // copy the value
  src.value : string as v -> tgt.text = v;
  src.a.b.c as x -> tgt.p.q = x "deep-path";
  src.code 0..1 first default ('x') as c where (c.length() > 1) check ($this != '') log (c) -> tgt.code = c "constraints";
  src -> tgt.extension as ext, ext.url = 'http://example.org', ext.value = create('Reference') as r share refs, r.reference = reference(src) "creates";
  src.n as n -> tgt.n = (n + 1), tgt.m = evaluate(n, n * 2), tgt.k = 5, tgt.flag = true "evaluates";
  src.name as sn -> tgt.name as tn then {
    sn.family as f -> tn.family = f "family";
  } "names";
  src.contact as s -> tgt.contact as t then Contact(s, t, 'x') "dependent";
  src.legacy default "fallback" as l -> tgt.legacy = l "legacyDefault";
}
"#
    ));
}

#[test]
fn batch_identity() {
    insta::assert_json_snapshot!(map_json(
        r#"group G(source src, target tgt) {
  src -> tgt : id, active, gender;
  src -> tgt : birthDate, deceased "Others";
}
"#
    ));
}

#[test]
fn concept_map() {
    insta::assert_json_snapshot!(map_json(
        r#"/// status = 'active'
conceptmap "GenderMap" {
  prefix s = "http://hl7.org/fhir/administrative-gender"
  prefix t = "http://terminology.hl7.org/CodeSystem/v2-0001"

  unmapped for s = provided
  s:male - t:M
  s:female == t:F
  s:"other value" <= t:O
}

group G(source src, target tgt) {
  src.gender as v -> tgt.value = translate(v, '#GenderMap', 'code');
}
"#
    ));
}

#[test]
fn reports_unknown_transform_and_unnamed_multi_source() {
    let compiled = compile(
        "group G(source src, target tgt) {\n  src.a as a, src.b as b -> tgt.c = frobnicate(a);\n}\n",
    );
    let messages: Vec<_> = compiled
        .diagnostics
        .iter()
        .map(|d| d.message.as_str())
        .collect();
    assert_eq!(
        messages,
        vec![
            "unknown transform `frobnicate`",
            "a rule with several sources needs a name"
        ]
    );
}

#[test]
fn metadata_elements_with_properties_and_repeats() {
    let map = map_json(
        r#"/// url = 'http://example.org/StructureMap/Meta'
/// name = 'Meta'
/// jurisdiction =
/// jurisdiction.coding =
/// jurisdiction.coding.system = 'urn:iso:std:iso:3166'
/// jurisdiction.coding.code = 'AQ'
/// jurisdiction.text = 'Antarctica'
/// jurisdiction =
/// jurisdiction.coding.code = 'AU'
/// identifier.system = 'urn:ietf:rfc:3986'
/// identifier.value = 'urn:oid:1.2.3'
/// contact.name = 'Team'
/// contact.telecom.system = 'url'
/// contact.telecom.value = 'http://example.org'
/// contact.telecom.rank = 1
/// copyrightLabel = 'CC0'

group G(source src, target tgt) {
  src.id -> tgt.id;
}
"#,
    );
    assert_eq!(
        map["jurisdiction"],
        serde_json::json!([
            { "coding": [{ "system": "urn:iso:std:iso:3166", "code": "AQ" }], "text": "Antarctica" },
            { "coding": [{ "code": "AU" }] }
        ])
    );
    assert_eq!(
        map["identifier"],
        serde_json::json!([{ "system": "urn:ietf:rfc:3986", "value": "urn:oid:1.2.3" }])
    );
    assert_eq!(
        map["contact"],
        serde_json::json!([{
            "name": "Team",
            "telecom": [{ "system": "url", "value": "http://example.org", "rank": 1 }]
        }])
    );
    assert_eq!(map["copyrightLabel"], "CC0");
}

#[test]
fn metadata_errors() {
    let compiled = compile(
        "/// jurisdiction = 'x'\n/// copyrightLabel = 'a'\n/// copyrightLabel = 'b'\n/// identifier.nope = 1\n\ngroup G(source src, target tgt) {\n}\n",
    );
    let messages: Vec<&str> = compiled
        .diagnostics
        .iter()
        .map(|d| d.message.as_str())
        .collect();
    assert_eq!(
        messages,
        [
            "`jurisdiction` is a CodeableConcept: give it an empty value and set its properties",
            "`copyrightLabel` does not repeat; it is already set",
            "`identifier` has no property `nope`",
        ]
    );
}

#[test]
fn locations_of_groups_rules_sources_and_targets() {
    let text = "group G(source src, target tgt) {\n  src.a as a -> tgt.a = a \"one\";\n  src -> tgt : b, c \"batch\";\n  src.d as d then {\n    d.e as e -> tgt.é = e \"nested\";\n  } \"outer\";\n}\n";
    let compiled = compile(text);
    let index = fml_syntax::LineIndex::new(text);
    let at = |position: &str| {
        let span = compiled
            .locations
            .span(text, &index, position)
            .unwrap_or_else(|| panic!("no location for {position}"));
        (
            span.start_row,
            span.start_column,
            span.end_row,
            span.end_column,
        )
    };
    assert_eq!(at("StructureMap.group[0]"), (0, 0, 6, 1));
    assert_eq!(at("StructureMap.group[0].rule[0]"), (1, 2, 1, 32));
    assert_eq!(at("StructureMap.group[0].rule[0].source[0]"), (1, 2, 1, 12));
    assert_eq!(
        at("StructureMap.group[0].rule[0].target[0]"),
        (1, 16, 1, 25)
    );
    // The batch rule makes two rules, both at the batch rule.
    assert_eq!(at("StructureMap.group[0].rule[1]"), (2, 2, 2, 28));
    assert_eq!(at("StructureMap.group[0].rule[2].target[0]"), (2, 2, 2, 28));
    assert_eq!(
        at("StructureMap.group[0].rule[3].rule[0].target[0]"),
        (4, 16, 4, 25)
    );
}
