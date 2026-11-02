//! Rules of the Mapping Language IG the reference engine does not follow: dependent groups
//! together with nested rules, source cardinality, `evaluate` with several values, unique group
//! names, `cast` namespaces and values, and constants that refer to themselves.
#![cfg(feature = "fhirpath")]

use fml_check::{CoreDefinitions, Release};
use fml_engine::{Engine, FhirPathExpressions, MapSet};
use serde_json::{Value, json};

const HEADER: &str = r#"/// url = 'http://example.org/StructureMap/main'
/// name = 'main'
uses "http://hl7.org/fhir/StructureDefinition/Patient" alias Patient as source
uses "http://hl7.org/fhir/StructureDefinition/Patient" alias Patient as target
imports "http://example.org/StructureMap/lib*"
"#;

async fn run_with(maps: &MapSet, body: &str, source: Value) -> Result<Value, String> {
    let compiled = fml_model::compile(&format!("{HEADER}\n{body}"));
    assert!(!compiled.has_errors(), "{:?}", compiled.diagnostics);
    let definitions = CoreDefinitions::new(Release::R5);
    let expressions = FhirPathExpressions::new(Release::R5).await.unwrap();
    Engine::new(&definitions, expressions)
        .with_maps(maps)
        .transform(&compiled.map, &source)
        .await
        .map(|t| t.target)
        .map_err(|e| e.to_string())
}

async fn run(body: &str, source: Value) -> Result<Value, String> {
    run_with(&MapSet::new(), body, source).await
}

fn patient() -> Value {
    json!({
        "resourceType": "Patient",
        "id": "p1",
        "name": [{ "family": "Doe", "given": ["Jane", "Q"] }]
    })
}

#[tokio::test]
async fn dependent_groups_run_before_nested_rules() {
    let body = r#"group Main(source src : Patient, target tgt : Patient) {
  src.name as n -> tgt.name as t then Family(n, t), Given(n, t) {
    n.family as f -> t.text = f "text";
  } "name";
}

group Family(source n, target t) {
  n.family as f -> t.family = f "family";
}

group Given(source n, target t) {
  n.given first as g -> t.given = g "given";
}
"#;
    assert_eq!(
        run(body, patient()).await.unwrap(),
        json!({
            "resourceType": "Patient",
            "name": [{ "text": "Doe", "family": "Doe", "given": ["Jane"] }]
        })
    );
}

#[tokio::test]
async fn source_cardinality_outside_its_range_is_an_error() {
    let body = r#"group Main(source src : Patient, target tgt : Patient) {
  src.name 0..1 as n -> tgt.name = n "oneName";
  src.name.given 1..1 as g -> tgt.id = 'x' "oneGiven";
}
"#;
    let two_names =
        json!({ "resourceType": "Patient", "name": [{ "family": "A" }, { "family": "B" }] });
    assert_eq!(
        run(body, two_names).await,
        Err("rule `oneName`: `src.name` has 2 values, outside 0..1".to_owned())
    );
    let error = run(body, patient()).await.unwrap_err();
    assert!(error.contains("has 2 values, outside 1..1"), "{error}");
}

#[tokio::test]
async fn evaluate_with_several_values_makes_one_target_each() {
    let body = r#"group Main(source src : Patient, target tgt : Patient) {
  src.name as n -> tgt.name as t, t.given = (n.given.upper()) "given";
}
"#;
    assert_eq!(
        run(body, patient()).await.unwrap(),
        json!({ "resourceType": "Patient", "name": [{ "given": ["JANE", "Q"] }] })
    );
    let single = r#"group Main(source src : Patient, target tgt : Patient) {
  src.name as n -> tgt.name as t, t.family = (n.given) "family";
}
"#;
    let error = run(single, patient()).await.unwrap_err();
    assert!(
        error.contains("`HumanName.family` does not repeat"),
        "{error}"
    );
    let named = r#"group Main(source src : Patient, target tgt : Patient) {
  src.name as n -> tgt.name as t, t.given = (n.given) as g "given";
}
"#;
    let error = run(named, patient()).await.unwrap_err();
    assert!(error.contains("variable `g` can only name one"), "{error}");
}

#[tokio::test]
async fn a_group_name_in_several_imported_maps_is_an_error() {
    let mut maps = MapSet::new();
    for suffix in ["A", "B"] {
        let library = fml_model::compile(&format!(
            "/// url = 'http://example.org/StructureMap/lib{suffix}'\n/// name = 'lib{suffix}'\n\ngroup Copy(source s, target t) {{\n  s.family as f -> t.family = f \"family\";\n}}\n"
        ));
        maps.add(library.map);
    }
    let body = r#"group Main(source src : Patient, target tgt : Patient) {
  src.name as n -> tgt.name as t then Copy(n, t) "name";
}
"#;
    let error = run_with(&maps, body, patient()).await.unwrap_err();
    assert!(
        error.contains("group `Copy` is in several imported maps"),
        "{error}"
    );
}

#[tokio::test]
async fn cast_takes_fhir_and_system_types_and_checks_values() {
    let body = r#"group Main(source src : Patient, target tgt : Patient) {
  src.id as v -> tgt.id = cast(v, 'FHIR.id') "id";
  src -> tgt.multipleBirth = cast('2', 'System.Integer') "birth";
  src -> tgt.birthDate = cast('2001-02-03', 'date') "date";
}
"#;
    assert_eq!(
        run(body, patient()).await.unwrap(),
        json!({ "resourceType": "Patient", "id": "p1", "birthDate": "2001-02-03", "multipleBirthInteger": 2 })
    );
    for (value, type_name) in [
        ("0", "positiveInt"),
        ("2001-13-01", "date"),
        ("12:00", "time"),
    ] {
        let body = format!(
            "group Main(source src : Patient, target tgt : Patient) {{\n  src -> tgt.id = cast('{value}', '{type_name}') \"bad\";\n}}\n"
        );
        let error = run(&body, patient()).await.unwrap_err();
        assert!(
            error.contains("cannot cast"),
            "{value} {type_name}: {error}"
        );
    }
}

#[tokio::test]
async fn a_constant_that_refers_to_itself_is_an_error() {
    let body = r#"let a = b + 1;
let b = a + 1;

group Main(source src : Patient, target tgt : Patient) {
  src -> tgt.id = (a.toString()) "id";
}
"#;
    let error = run(body, patient()).await.unwrap_err();
    assert!(error.contains("refers to itself"), "{error}");
}

#[tokio::test]
async fn first_and_last_targets_keep_their_place_in_a_list() {
    let body = r#"group Main(source src : Patient, target tgt : Patient) {
  src.name as n -> tgt.name = n first "firsts";
  src.name last as n -> tgt.name = n last "lasts";
  src.id as v -> tgt.name as out then {
    v -> out.text = v "text";
  } "middle";
}
"#;
    let source = json!({
        "resourceType": "Patient",
        "id": "middle",
        "name": [{ "family": "Alpha" }, { "family": "Beta" }]
    });
    assert_eq!(
        run(body, source).await.unwrap()["name"],
        json!([{ "family": "Alpha" }, { "family": "Beta" }, { "text": "middle" }, { "family": "Beta" }])
    );
}

#[tokio::test]
async fn two_rules_cannot_both_be_first_or_last() {
    for mode in ["first", "last"] {
        let body = format!(
            "group Main(source src : Patient, target tgt : Patient) {{\n  src.name first as n -> tgt.name = n {mode} \"a\";\n  src.name last as n -> tgt.name = n {mode} \"b\";\n}}\n"
        );
        let error = run(&body, patient()).await.unwrap_err();
        assert!(
            error.contains(&format!("two rules put their values `{mode}`")),
            "{error}"
        );
    }
}

#[tokio::test]
async fn last_values_move_with_their_primitive_extensions() {
    let body = r#"group Main(source src : Patient, target tgt : Patient) {
  src.name as n -> tgt.name as out then {
    n.given first as a -> out.given = a last "a";
    n.given last as b -> out.given = b "b";
  } "name";
}
"#;
    let source = json!({
        "resourceType": "Patient",
        "name": [{
            "given": ["A", "B"],
            "_given": [{ "extension": [{ "url": "http://example.org/note", "valueString": "one" }] }, null]
        }]
    });
    assert_eq!(
        run(body, source).await.unwrap()["name"],
        json!([{
            "given": ["B", "A"],
            "_given": [null, { "extension": [{ "url": "http://example.org/note", "valueString": "one" }] }]
        }])
    );
}

#[tokio::test]
async fn a_single_target_takes_one_value() {
    let body = r#"group Main(source src : Patient, target tgt : Patient) {
  src.name as n -> tgt.name = n single "names";
}
"#;
    let two_names =
        json!({ "resourceType": "Patient", "name": [{ "family": "A" }, { "family": "B" }] });
    let error = run(body, two_names).await.unwrap_err();
    assert!(error.contains("is `single`"), "{error}");
    let one_name = json!({ "resourceType": "Patient", "name": [{ "family": "A" }] });
    assert_eq!(
        run(body, one_name).await.unwrap()["name"],
        json!([{ "family": "A" }])
    );
}

#[tokio::test]
async fn a_default_stands_in_when_the_type_filter_leaves_nothing() {
    let body = r#"group Main(source src : Patient, target tgt : Patient) {
  src.active : string default ('unknown') as v -> tgt.gender = v "gender";
}
"#;
    assert_eq!(
        run(body, json!({ "resourceType": "Patient", "active": true }))
            .await
            .unwrap(),
        json!({ "resourceType": "Patient", "gender": "unknown" })
    );
}

/// `share` scopes, as the IG's editors settled them: a shared target lives in the rule list that
/// declares it (siblings and their nested rules reuse it, every match of the source too), each
/// dependent block and each run of a parent rule has its own, and called groups start afresh.
#[tokio::test]
async fn shared_targets_follow_their_scope() {
    let cases = [
        (
            r#"  src -> tgt.name as out share common then {
    src.name as name -> out.family = name.family "family";
  } "empty";
  src.id as id -> tgt.name as out share common then {
    id -> out.family = id "id";
  } "filled";"#,
            json!({ "resourceType": "Patient", "id": "filled-by-sibling" }),
            json!([{ "family": "filled-by-sibling" }]),
        ),
        (
            r#"  src -> tgt.name as a share common then {
    src.name as name -> a.family = name.family "family";
  } "empty";
  src -> tgt.name as b then {
    src -> b.family = 'B' "b";
  } "other";
  src.id as id -> tgt.name as a share common then {
    id -> a.family = id "id";
  } "filled";"#,
            json!({ "resourceType": "Patient", "id": "A" }),
            json!([{ "family": "A" }, { "family": "B" }]),
        ),
        (
            r#"  src.name as n -> tgt then {
    n.family as f -> tgt.name as out share name then {
      f -> out.family = f "family";
    } "a";
    n.family as f -> tgt.name as out share name then {
      f -> out.text = f "text";
    } "b";
  } "names";"#,
            json!({ "resourceType": "Patient", "name": [{ "family": "Alpha" }, { "family": "Beta" }] }),
            json!([{ "family": "Alpha", "text": "Alpha" }, { "family": "Beta", "text": "Beta" }]),
        ),
        (
            r#"  src.name as n -> tgt.name as out share common then {
    n.family as f -> out.given = f "given";
  } "names";"#,
            json!({ "resourceType": "Patient", "name": [{ "family": "Alpha" }, { "family": "Beta" }] }),
            json!([{ "given": ["Alpha", "Beta"] }]),
        ),
        (
            r#"  src -> tgt.name as parent share common then {
    src.name as n -> tgt.name as child share common then {
      n.family as f -> child.family = f "family";
    } "child";
  } "parent";
  src.id as id -> tgt.name as sibling share common then {
    id -> sibling.text = id "text";
  } "sibling";"#,
            json!({ "resourceType": "Patient", "id": "parent", "name": [{ "family": "Alpha" }, { "family": "Beta" }] }),
            json!([{ "family": "Beta", "text": "parent" }]),
        ),
        (
            r#"  src -> tgt then {
    src.id as v -> tgt.name as out share common then {
      v -> out.family = v "family";
    } "a";
  } "blockA";
  src -> tgt then {
    src.id as v -> tgt.name as out share common then {
      v -> out.text = v "text";
    } "b";
  } "blockB";
  src.id as v -> tgt.name as out share common then {
    v -> out.given = v "given";
  } "top";"#,
            json!({ "resourceType": "Patient", "id": "value" }),
            json!([{ "family": "value" }, { "text": "value" }, { "given": ["value"] }]),
        ),
    ];
    for (rules, source, names) in cases {
        let body =
            format!("group Main(source src : Patient, target tgt : Patient) {{\n{rules}\n}}\n");
        let target = run(&body, source).await.unwrap();
        assert_eq!(target["name"], names, "{rules}");
    }

    let body = r#"group Main(source src : Patient, target tgt : Patient) {
  src.id as v -> tgt.name as out share common then {
    v -> out.family = v "family";
  } "a";
  src -> tgt then Helper(src, tgt), Helper(src, tgt) "helpers";
  src.id as v -> tgt.name as out share common then {
    v -> out.text = v "text";
  } "b";
}

group Helper(source src : Patient, target tgt : Patient) {
  src.id as v -> tgt.name as out share common then {
    v -> out.given = v "given";
  } "helper";
}
"#;
    let target = run(body, json!({ "resourceType": "Patient", "id": "value" }))
        .await
        .unwrap();
    assert_eq!(
        target["name"],
        json!([{ "family": "value", "text": "value" }, { "given": ["value"] }, { "given": ["value"] }])
    );
}
