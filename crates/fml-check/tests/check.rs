//! What the checker reports, against the embedded core definitions.
#![cfg(feature = "fhirschema")]

use fml_check::{CoreDefinitions, Release, check};
use fml_syntax::{LineIndex, parse};

fn diagnostics(source: &str, release: Release) -> String {
    let parse = parse(source);
    assert!(parse.diagnostics().is_empty(), "{:?}", parse.diagnostics());
    let index = LineIndex::new(source);
    check(&parse.tree(), &CoreDefinitions::new(release))
        .iter()
        .map(|d| {
            let at = index.line_col(source, d.range.start());
            format!("{}:{} {}", at.line + 1, at.col + 1, d.message)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn reports_what_cannot_work() {
    let source = r#"uses "http://hl7.org/fhir/StructureDefinition/Patient" alias P as source
uses "http://hl7.org/fhir/StructureDefinition/Basic" alias B as target
uses "http://hl7.org/fhir/StructureDefinition/NoSuchThing" as source

group Main(source src : P, target tgt : B) {
  src.nmae -> tgt.code;
  src.deceased : string as d -> tgt.subject;
  src.name as n -> tgt.extension as e, e.value = create('Quantity'), e.url = create('DomainResource');
  src.gender as g -> src.gender = g "writesSource";
  src.contact as c -> tgt.extension as e then Contact(c, e) "contact";
  src.birthDate as b -> tgt.created = truncate(b) "arity";
  src.id as i -> tgt.id = copy(missing) "unknownVariable";
  src.link as l then Missing(l) "unknownGroup";
}

group Contact(source src, target tgt) {
  src.relationshp -> tgt.url;
  src.name as n -> tgt.value = create('HumanName') "name";
}
"#;
    insta::assert_snapshot!(diagnostics(source, Release::R5), @r"
    3:6 unknown structure `http://hl7.org/fhir/StructureDefinition/NoSuchThing`
    6:7 `Patient` has no element `nmae`
    7:16 `string` is not a type of `Patient.deceased`; it is boolean, dateTime
    8:85 `DomainResource` is abstract and cannot be created
    9:22 `src` is a source variable; a target writes into a target variable
    11:39 `truncate` takes 2 argument(s), found 1
    12:32 unknown variable `missing`
    13:22 group `Missing` is not declared and nothing is imported
    17:7 `Patient.contact` has no element `relationshp`
    ");
}

#[test]
fn accepts_valid_maps() {
    let source = r#"uses "http://hl7.org/fhir/4.0/StructureDefinition/Bundle" alias BundleR4 as source
uses "http://hl7.org/fhir/5.0/StructureDefinition/Bundle" alias BundleR5 as target
let fallback = 'none';

group Bundle(source src : BundleR4, target tgt : BundleR5) extends Resource <<type+>> {
  src.type -> tgt.type;
  src.entry as s -> tgt.entry as t then {
    s.fullUrl -> t.fullUrl;
    s.resource -> t.resource;
    s.response as rs -> t.response as rt then {
      rs.outcome : OperationOutcome -> rt.outcome "outcome";
      rs.status as v -> rt.status = truncate(v, fallback) "status";
    } "response";
  } "entry";
  src.signature as s -> tgt.signature as t then Signature(s, t) "signature";
}

group Signature(source src, target tgt) {
  src.when -> tgt.when;
  src.who -> tgt.who;
  src.data as d -> tgt.data = d "data";
}

group Resource(source src, target tgt) {
  src.id -> tgt.id;
  src.meta -> tgt.meta;
}

group string(source src : string, target tgt : string) <<type+>> {
  src.value as v -> tgt.value = v "value";
}
"#;
    assert_eq!(diagnostics(source, Release::R5), "");
}

#[test]
fn untyped_groups_are_checked_with_their_call_types() {
    let source = r#"uses "http://hl7.org/fhir/StructureDefinition/Questionnaire" as source
uses "http://hl7.org/fhir/StructureDefinition/Questionnaire" as target

group Main(source src : Questionnaire, target tgt : Questionnaire) {
  src.item as s -> tgt.item as t then Item(s, t) "item";
}

group Item(source src, target tgt) {
  src.linkId -> tgt.linkId;
  src.item as s -> tgt.item as t then Item(s, t) "nested";
  src.answer -> tgt.answer;
}
"#;
    insta::assert_snapshot!(
        diagnostics(source, Release::R4),
        @r"
    11:7 `Questionnaire.item` has no element `answer`
    11:21 `Questionnaire.item` has no element `answer`
    "
    );
}
