//! Maps over a logical model read from XML: the HL7 CDA model (hl7.cda.uv.core) and C-CDA
//! templates (hl7.cda.us.ccda), both CC0, in both directions.
//!
//! - Values whose type `xsi:type` gives (`"@type"` in JSON) have that type: type filters select
//!   them, `create('CD')` into such an element makes one, and the output names the type again.
//! - Profiles applied by template id (C-CDA templates): a type filter with such a profile selects
//!   the values that carry one of its templates, and default groups are chosen the same way.
//! - The source model is R5 and the target R4 in the same map.
#![cfg(feature = "fhirpath")]

use std::path::Path;

use fml_check::{CoreDefinitions, PackageDefinitions, Release};
use fml_engine::{Engine, FhirPathExpressions};
use serde_json::{Value, json};

fn bundle(name: &str) -> Vec<Value> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(name);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let bundle: Value = serde_json::from_str(&text).unwrap();
    bundle["resources"].as_array().unwrap().clone()
}

fn resources() -> Vec<Value> {
    let mut all = bundle("../fml-xml/tests/data/cda-core.json");
    all.extend(bundle("tests/data/ccda-templates.json"));
    all
}

fn definitions() -> PackageDefinitions<CoreDefinitions> {
    let mut definitions = PackageDefinitions::new(CoreDefinitions::new(Release::R4));
    for resource in resources() {
        definitions.add(&resource).unwrap();
    }
    definitions
}

fn model() -> fml_xml::Model {
    let mut model = fml_xml::Model::new();
    for resource in resources() {
        model.add(&resource);
    }
    model
}

async fn run(map: &str, source: &Value) -> Value {
    run_with(map, source, true).await
}

async fn run_with(map: &str, source: &Value, template_versions: bool) -> Value {
    let compiled = fml_model::compile(map);
    assert!(!compiled.has_errors(), "{:?}", compiled.diagnostics);
    let definitions = definitions();
    let expressions = FhirPathExpressions::new(Release::R4).await.unwrap();
    Engine::new(&definitions, expressions)
        .with_template_versions(template_versions)
        .transform(&compiled.map, source)
        .await
        .unwrap_or_else(|e| panic!("{e}"))
        .target
}

const CONCERN: &str = r#"<act xmlns="urn:hl7-org:v3" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" classCode="ACT" moodCode="EVN">
  <templateId root="2.16.840.1.113883.10.20.22.4.30" extension="2015-08-01"/>
  <id root="36e3e930-7b14-11db-9fe1-0800200c9a66"/>
  <code code="CONC" codeSystem="2.16.840.1.113883.5.6"/>
  <statusCode code="active"/>
  <effectiveTime><low value="20080501"/></effectiveTime>
  <entryRelationship typeCode="SUBJ">
    <observation classCode="OBS" moodCode="EVN">
      <templateId root="2.16.840.1.113883.10.20.22.4.7"/>
      <templateId root="2.16.840.1.113883.10.20.22.4.7" extension="2014-06-09"/>
      <id root="4adc1020-7b14-11db-9fe1-0800200c9a66"/>
      <code code="ASSERTION" codeSystem="2.16.840.1.113883.5.4"/>
      <statusCode code="completed"/>
      <effectiveTime><low value="200805011030-0500"/></effectiveTime>
      <value xsi:type="CD" code="419511003" displayName="Propensity to adverse reaction to drug" codeSystem="2.16.840.1.113883.6.96"/>
      <entryRelationship typeCode="MFST" inversionInd="true">
        <observation classCode="OBS" moodCode="EVN">
          <templateId root="2.16.840.1.113883.10.20.22.4.9" extension="2014-06-09"/>
          <code code="ASSERTION" codeSystem="2.16.840.1.113883.5.4"/>
          <value xsi:type="CD" code="247472004" displayName="Hives" codeSystem="2.16.840.1.113883.6.96"/>
        </observation>
      </entryRelationship>
      <entryRelationship typeCode="SUBJ" inversionInd="true">
        <observation classCode="OBS" moodCode="EVN">
          <templateId root="2.16.840.1.113883.10.20.22.4.8" extension="2014-06-09"/>
          <code code="SEV" codeSystem="2.16.840.1.113883.5.4"/>
          <value xsi:type="CD" code="6736007" displayName="Moderate" codeSystem="2.16.840.1.113883.6.96"/>
        </observation>
      </entryRelationship>
    </observation>
  </entryRelationship>
</act>"#;

const TO_FHIR: &str = r#"map "http://example.org/StructureMap/allergy-to-fhir" = "AllergyToFhir"

uses "http://hl7.org/cda/stds/core/StructureDefinition/Act" alias Act as source
uses "http://hl7.org/cda/stds/core/StructureDefinition/CD" alias CD as source
uses "http://hl7.org/cda/us/ccda/StructureDefinition/AllergyIntoleranceObservation" alias AllergyObservation as source
uses "http://hl7.org/cda/us/ccda/StructureDefinition/ReactionObservation" alias ReactionObservation as source
uses "http://hl7.org/cda/us/ccda/StructureDefinition/SeverityObservation" alias SeverityObservation as source
uses "http://hl7.org/fhir/StructureDefinition/AllergyIntolerance" alias AllergyIntolerance as target
uses "http://hl7.org/fhir/StructureDefinition/Annotation" alias Annotation as target

group Concern(source act : Act, target tgt : AllergyIntolerance) {
  act.entryRelationship as er then {
    er.observation : AllergyObservation as obs then Allergy(obs, tgt);
    er.observation : ReactionObservation as wrong -> tgt.note as n, n.text = 'a concern holds no reaction';
  };
}

group Allergy(source obs : AllergyObservation, target tgt : AllergyIntolerance) {
  obs.id as id -> tgt.identifier as i then {
    id.root as r -> i.system = 'urn:ietf:rfc:3986', i.value = append('urn:uuid:', r);
  };
  obs.effectiveTime as et then {
    et.low as low then { low.value as v -> tgt.onset = v; };
  };
  obs.value : CD as v -> tgt.code as cc then CdToConcept(v, cc);
  obs.entryRelationship as er then {
    er.observation : ReactionObservation as r -> tgt.reaction as reaction then {
      r.value : CD as v -> reaction.manifestation as m then CdToConcept(v, m);
    };
    er.observation -> tgt.note;
  };
}

group CdToConcept(source v : CD, target cc : CodeableConcept) {
  v -> cc.coding as coding then {
    v.code as code -> coding.code = code;
    v.displayName as d -> coding.display = d;
    v.codeSystem as s where (s = '2.16.840.1.113883.6.96') -> coding.system = 'http://snomed.info/sct';
  };
}

group SeverityNote(source o : SeverityObservation, target n : Annotation) <<types>> {
  o.value as v then { v.displayName as d -> n.text = append('severity: ', d); };
}

group ReactionNote(source o : ReactionObservation, target n : Annotation) <<types>> {
  o -> n.text = 'reaction';
}
"#;

#[tokio::test]
async fn cda_xml_to_fhir_by_templates() {
    let model = model();
    let document = fml_xml::read(CONCERN, &model, None).unwrap();
    assert!(document.issues.is_empty(), "{:#?}", document.issues);
    let result = run(TO_FHIR, &document.json).await;
    assert_eq!(
        result,
        json!({
            "resourceType": "AllergyIntolerance",
            "identifier": [{ "system": "urn:ietf:rfc:3986", "value": "urn:uuid:4adc1020-7b14-11db-9fe1-0800200c9a66" }],
            "code": { "coding": [{ "system": "http://snomed.info/sct", "code": "419511003", "display": "Propensity to adverse reaction to drug" }] },
            "onsetDateTime": "2008-05-01T10:30-05:00",
            "note": [{ "text": "reaction" }, { "text": "severity: Moderate" }],
            "reaction": [{ "manifestation": [{ "coding": [{ "system": "http://snomed.info/sct", "code": "247472004", "display": "Hives" }] }] }]
        })
    );
}

const TO_CDA: &str = r#"map "http://example.org/StructureMap/allergy-to-cda" = "AllergyToCda"

uses "http://hl7.org/fhir/StructureDefinition/AllergyIntolerance" alias AllergyIntolerance as source
uses "http://hl7.org/cda/stds/core/StructureDefinition/Observation" alias Observation as target

group Allergy(source ai : AllergyIntolerance, target obs : Observation) {
  ai -> obs.classCode = 'OBS', obs.moodCode = 'EVN';
  ai -> obs.templateId as t, t.root = '2.16.840.1.113883.10.20.22.4.7', t.extension = '2014-06-09';
  ai.onset : dateTime as d -> obs.effectiveTime as et, et.low as low, low.value = d;
  ai.code as c -> obs.value = create('CD') as v then {
    c.coding as coding then {
      coding.code as code -> v.code = code;
      coding.display as d -> v.displayName = d;
      coding.system as s where (s = 'http://snomed.info/sct') -> v.codeSystem = '2.16.840.1.113883.6.96';
    };
  };
  ai.reaction as r -> obs.entryRelationship as er, er.typeCode = 'MFST', er.inversionInd = true, er.observation as ro then {
    r -> ro.classCode = 'OBS', ro.moodCode = 'EVN';
    r.manifestation as m -> ro.value = create('CD') as v then {
      m.coding as coding then { coding.code as code -> v.code = code; };
    };
  };
}
"#;

#[tokio::test]
async fn fhir_to_cda_xml() {
    let allergy = json!({
        "resourceType": "AllergyIntolerance",
        "onsetDateTime": "2008-05-01T10:30:00-05:00",
        "code": { "coding": [{ "system": "http://snomed.info/sct", "code": "419511003", "display": "Propensity" }] },
        "reaction": [{ "manifestation": [{ "coding": [{ "system": "http://snomed.info/sct", "code": "247472004" }] }] }]
    });
    let result = run(TO_CDA, &allergy).await;
    assert_eq!(
        result["value"],
        json!([{ "@type": "CD", "code": "419511003", "displayName": "Propensity", "codeSystem": "2.16.840.1.113883.6.96" }])
    );
    let model = model();
    let written = fml_xml::write(
        &result,
        &model,
        "http://hl7.org/cda/stds/core/StructureDefinition/Observation",
    )
    .unwrap();
    assert!(written.issues.is_empty(), "{:#?}", written.issues);
    let expected = r#"<observation xmlns="urn:hl7-org:v3" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" classCode="OBS" moodCode="EVN">
  <templateId root="2.16.840.1.113883.10.20.22.4.7" extension="2014-06-09"/>
  <effectiveTime><low value="20080501103000-0500"/></effectiveTime>
  <value xsi:type="CD" code="419511003" codeSystem="2.16.840.1.113883.6.96" displayName="Propensity"/>
  <entryRelationship typeCode="MFST" inversionInd="true">
    <observation classCode="OBS" moodCode="EVN"><value xsi:type="CD" code="247472004"/></observation>
  </entryRelationship>
</observation>"#;
    let differences = fml_xml::compare(expected, &written.xml).unwrap();
    assert!(differences.is_empty(), "{differences:#?}\n{}", written.xml);
}

/// A template version other than the profile's: recognised only when versions do not count.
#[tokio::test]
async fn template_versions_can_be_ignored() {
    let model = model();
    let older = CONCERN.replace(
        r#"<templateId root="2.16.840.1.113883.10.20.22.4.7" extension="2014-06-09"/>"#,
        "",
    );
    let document = fml_xml::read(&older, &model, None).unwrap();
    let strict = run_with(TO_FHIR, &document.json, true).await;
    assert!(strict.get("code").is_none(), "{strict}");
    let any_version = run_with(TO_FHIR, &document.json, false).await;
    assert_eq!(any_version["code"]["coding"][0]["code"], "419511003");
}

/// A type filter names a logical model's type by its structure name (`PQ`, `IVL_TS`).
#[tokio::test]
async fn type_filters_name_logical_types() {
    let map = r#"map "http://example.org/StructureMap/values" = "Values"

uses "http://hl7.org/cda/stds/core/StructureDefinition/Observation" alias Observation as source
uses "http://hl7.org/fhir/StructureDefinition/Observation" alias FhirObservation as target

group Values(source obs : Observation, target tgt : FhirObservation) {
  obs.value : PQ as v -> tgt.value = create('Quantity') as q then {
    v.value as n -> q.value = n;
    v.unit as u -> q.unit = u;
  };
  obs.value : ST as v -> tgt.note as note then {
    v.xmlText as t -> note.text = t;
  };
}
"#;
    let model = model();
    let xml = r#"<observation xmlns="urn:hl7-org:v3" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" classCode="OBS" moodCode="EVN">
  <value xsi:type="PQ" value="120" unit="mm[Hg]"/>
  <value xsi:type="ST">high</value>
</observation>"#;
    let document = fml_xml::read(xml, &model, None).unwrap();
    let result = run(map, &document.json).await;
    assert_eq!(
        result["valueQuantity"],
        json!({ "value": 120, "unit": "mm[Hg]" })
    );
    assert_eq!(result["note"][0]["text"], "high");
}
