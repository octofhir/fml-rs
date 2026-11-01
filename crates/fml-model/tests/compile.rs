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
