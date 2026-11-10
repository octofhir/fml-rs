//! Внешний сервер в FHIRPath и типы ресурсов после resolve() (fhirpath-lab#97).
#![cfg(feature = "fhirpath")]

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use fml_check::{CoreDefinitions, Release};
use fml_engine::{Engine, Expressions, FhirPathExpressions, Item, ServerProvider};
use octofhir_fhir_model::error::Result;
use serde_json::{Value, json};

#[derive(Debug, Default)]
struct Server {
    reads: Mutex<Vec<(String, String)>>,
}

fn body_structure() -> Value {
    json!({
        "resourceType": "BodyStructure",
        "id": "abc",
        "extension": [{"url": "http://example.org/row-group", "valueString": "member"}]
    })
}

#[async_trait]
impl ServerProvider for Server {
    async fn read(&self, resource_type: &str, id: &str) -> Result<Option<Value>> {
        self.reads
            .lock()
            .unwrap()
            .push((resource_type.into(), id.into()));
        Ok((resource_type == "BodyStructure" && id == "abc").then(body_structure))
    }

    async fn create(&self, _: &Value) -> Result<Option<Value>> {
        panic!("unexpected create")
    }

    async fn update(&self, _: &Value) -> Result<Option<Value>> {
        panic!("unexpected update")
    }

    async fn delete(&self, _: &Value) -> Result<bool> {
        panic!("unexpected delete")
    }

    async fn search(&self, _: bool, _: &Value) -> Result<Option<Value>> {
        panic!("unexpected search")
    }

    async fn patch(&self, _: &Value) -> Result<Option<Value>> {
        panic!("unexpected patch")
    }

    async fn capabilities(&self, _: Option<&str>) -> Result<Option<Value>> {
        panic!("unexpected capabilities")
    }

    async fn validate(&self, _: &Value, _: &str, _: &Value) -> Result<Option<Value>> {
        panic!("unexpected validate")
    }

    async fn transform(&self, _: &Value, _: &Value) -> Result<Option<Value>> {
        panic!("unexpected transform")
    }

    async fn everything(&self, _: &str, _: &str, _: &Value) -> Result<Option<Value>> {
        panic!("unexpected everything")
    }

    async fn apply(&self, _: &Value, _: &str, _: &Value) -> Result<Option<Value>> {
        panic!("unexpected apply")
    }

    fn base_url(&self) -> &str {
        "http://example.org/fhir"
    }
}

#[tokio::test]
async fn external_resolution_is_opt_in_and_preserves_resource_types() {
    let focus = Item {
        json: json!({"reference": "BodyStructure/abc"}),
        type_name: Some("Reference".into()),
    };
    let expressions = FhirPathExpressions::new(Release::R6).await.unwrap();
    assert!(
        expressions
            .evaluate("resolve()", Some(&focus), &[])
            .await
            .unwrap()
            .is_empty()
    );

    let server = Arc::new(Server::default());
    let expressions = expressions.with_server_provider(server.clone());
    let result = expressions
        .evaluate("resolve()", Some(&focus), &[])
        .await
        .unwrap();
    assert_eq!(
        result,
        vec![Item {
            json: body_structure(),
            type_name: Some("BodyStructure".into())
        }]
    );
    for (expression, expected) in [
        ("resolve().id.toString()", json!("abc")),
        ("(resolve() as BodyStructure).id.toString()", json!("abc")),
        (
            "resolve().ofType(BodyStructure).id.toString()",
            json!("abc"),
        ),
        ("resolve().extension.count().toString()", json!("1")),
        ("resolve() is BodyStructure", json!(true)),
        ("resolve().is(BodyStructure)", json!(true)),
        ("resolve().ofType(Patient)", Value::Null),
    ] {
        let result = expressions
            .evaluate(expression, Some(&focus), &[])
            .await
            .unwrap();
        if expected.is_null() {
            assert!(result.is_empty(), "{expression}: {result:?}");
        } else {
            assert_eq!(result.len(), 1, "{expression}");
            assert_eq!(result[0].json, expected, "{expression}");
        }
    }
    assert!(
        server
            .reads
            .lock()
            .unwrap()
            .iter()
            .all(|(ty, id)| ty == "BodyStructure" && id == "abc")
    );
    assert!(!server.reads.lock().unwrap().is_empty());
}

#[tokio::test]
async fn request_local_providers_do_not_leak_between_clones() {
    let shared = FhirPathExpressions::new(Release::R6).await.unwrap();
    let first_server = Arc::new(Server::default());
    let second_server = Arc::new(Server::default());
    let first = shared.clone().with_server_provider(first_server.clone());
    let second = shared.clone().with_server_provider(second_server.clone());
    let focus = Item {
        json: json!({"reference": "BodyStructure/abc"}),
        type_name: Some("Reference".into()),
    };
    let (first_result, second_result) = tokio::join!(
        first.evaluate("resolve().id", Some(&focus), &[]),
        second.evaluate("resolve().id", Some(&focus), &[]),
    );
    assert_eq!(first_result.unwrap()[0].json, "abc");
    assert_eq!(second_result.unwrap()[0].json, "abc");
    assert_eq!(first_server.reads.lock().unwrap().len(), 1);
    assert_eq!(second_server.reads.lock().unwrap().len(), 1);
    assert!(
        shared
            .evaluate("resolve()", Some(&focus), &[])
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn server_variable_uses_the_same_provider() {
    let server = Arc::new(Server::default());
    let expressions = FhirPathExpressions::new(Release::R6)
        .await
        .unwrap()
        .with_server_provider(server.clone());
    let result = expressions
        .evaluate("%server.read('BodyStructure', 'abc').id", None, &[])
        .await
        .unwrap();
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].json, "abc");
    assert_eq!(
        *server.reads.lock().unwrap(),
        vec![("BodyStructure".into(), "abc".into())]
    );
}

#[tokio::test]
async fn local_resolution_takes_precedence_and_unresolved_items_are_ignored() {
    let server = Arc::new(Server::default());
    let expressions = FhirPathExpressions::new(Release::R6)
        .await
        .unwrap()
        .with_server_provider(server.clone());
    let focus = Item {
        json: json!({
            "resourceType": "Observation",
            "contained": [body_structure()],
            "bodyStructure": [{"reference": {"reference": "#abc"}}]
        }),
        type_name: Some("Observation".into()),
    };
    let result = expressions
        .evaluate("bodyStructure.reference.resolve().id", Some(&focus), &[])
        .await
        .unwrap();
    assert_eq!(result[0].json, "abc");
    assert!(server.reads.lock().unwrap().is_empty());
    let result = expressions
        .evaluate(
            "('BodyStructure/missing' | '#missing').resolve()",
            None,
            &[],
        )
        .await
        .unwrap();
    assert!(result.is_empty());
    assert_eq!(
        *server.reads.lock().unwrap(),
        vec![("BodyStructure".into(), "missing".into())]
    );
}

#[tokio::test]
async fn resolved_values_pass_to_queried_typed_groups() {
    let definitions = CoreDefinitions::new(Release::R6);
    let expressions = FhirPathExpressions::with_templates(Release::R6, Default::default())
        .await
        .unwrap()
        .with_server_provider(Arc::new(Server::default()));
    let source = json!({
        "resourceType": "Observation",
        "bodyStructure": [{"reference": {"reference": "BodyStructure/abc"}}]
    });
    for expression in [
        "resolve()",
        "(resolve() as BodyStructure)",
        "resolve().ofType(BodyStructure)",
    ] {
        let text = format!(
            r#"/// url = 'http://example.org/StructureMap/resolve'
/// name = 'resolve'
uses "http://hl7.org/fhir/6.0/StructureDefinition/Observation" alias ObservationR6 as source
uses "http://hl7.org/fhir/4.0/StructureDefinition/Observation" alias ObservationR4 as target
uses "http://hl7.org/fhir/6.0/StructureDefinition/BodyStructure" alias BodyStructureQ as queried
group Main(source src : ObservationR6, target tgt : ObservationR4) {{
  src.bodyStructure as bs then {{
    bs.reference as r -> tgt.code as c, evaluate(r, {expression}) as row then Rows(row, c) "resolve";
  }} "bodyStructure";
}}
group Rows(source row : BodyStructureQ, target c) {{
  row.id as id -> c.text = id "id";
  row.extension as e -> c.coding as coding, coding.code = (e.value) "extension";
}}
"#
        );
        let parsed = fml_syntax::parse(&text);
        assert!(
            parsed.diagnostics().is_empty(),
            "{:?}",
            parsed.diagnostics()
        );
        let diagnostics = fml_check::check(&parsed.tree(), &definitions);
        assert!(diagnostics.is_empty(), "{expression}: {diagnostics:?}");
        let compiled = fml_model::compile(&text);
        assert!(!compiled.has_errors(), "{:?}", compiled.diagnostics);
        let transformed = Engine::new(&definitions, &expressions)
            .transform(&compiled.map, &source)
            .await
            .unwrap();
        assert_eq!(
            transformed.target,
            json!({
                "resourceType": "Observation",
                "code": {"text": "abc", "coding": [{"code": "member"}]}
            }),
            "{expression}"
        );
    }
}
