use fml_check::{CoreDefinitions, Definitions, Release};
use fml_engine::{Expressions, FhirPathExpressions, Item};

fn main() {
    futures::executor::block_on(async {
        for release in [Release::R4, Release::R4B, Release::R5, Release::R6] {
            let definitions = CoreDefinitions::new(release);
            assert!(
                definitions
                    .structure("http://hl7.org/fhir/StructureDefinition/Patient")
                    .is_some()
            );
            let expressions = FhirPathExpressions::new(release).await.unwrap();
            let focus = Item {
                json: serde_json::json!({"resourceType": "Patient", "name": [{"given": ["Иван"]}]}),
                type_name: Some("Patient".to_owned()),
            };
            let result = expressions
                .evaluate("name.given", Some(&focus), &[])
                .await
                .unwrap();
            assert_eq!(result.len(), 1);
            assert_eq!(result[0].json, "Иван");
        }
    });
}
