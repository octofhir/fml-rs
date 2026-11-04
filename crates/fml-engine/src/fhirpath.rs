//! FHIRPath through octofhir-fhirpath, typed by the embedded core definitions.

use std::collections::HashMap;
use std::sync::Arc;

use fml_check::Release;
use octofhir_fhir_model::ValidationProvider;
use octofhir_fhirpath::evaluator::functions::HasTemplateIdOfFunctionEvaluator;
use octofhir_fhirpath::{
    Collection, EvaluationContext, FhirPathEngine, FhirPathValue, create_function_registry,
};
use octofhir_fhirschema::{
    EmbeddedSchemaProvider, ModelFhirVersion, ModelProvider,
    create_validation_provider_from_embedded,
};

use crate::expressions::{Expressions, Item};

/// FHIRPath for one FHIR release.
pub struct FhirPathExpressions {
    engine: FhirPathEngine,
    provider: Arc<dyn ModelProvider + Send + Sync>,
    /// For `conformsTo()`.
    validation: Option<Arc<dyn ValidationProvider>>,
}

impl FhirPathExpressions {
    pub async fn new(release: Release) -> Result<Self, String> {
        Self::with_templates(release, HashMap::new()).await
    }

    /// FHIRPath whose `hasTemplateIdOf(profileUrl)` (CDA) knows the templates of these profiles,
    /// such as [`PackageDefinitions::template_profiles`](fml_check::PackageDefinitions) gives.
    pub async fn with_templates(
        release: Release,
        templates: HashMap<String, Vec<fml_check::TemplateId>>,
    ) -> Result<Self, String> {
        let version = match release {
            Release::R4 => ModelFhirVersion::R4,
            Release::R4B => ModelFhirVersion::R4B,
            Release::R5 => ModelFhirVersion::R5,
            Release::R6 => ModelFhirVersion::R6,
        };
        let embedded = Arc::new(EmbeddedSchemaProvider::new(version));
        let provider: Arc<dyn ModelProvider + Send + Sync> = embedded.clone();
        let mut registry = create_function_registry();
        if !templates.is_empty() {
            let resolver: HashMap<String, Vec<(String, Option<String>)>> = templates
                .into_iter()
                .map(|(url, ids)| {
                    let ids = ids.into_iter().map(|t| (t.root, t.extension)).collect();
                    (url, ids)
                })
                .collect();
            registry.register_pure_function(HasTemplateIdOfFunctionEvaluator::with_resolver(
                Arc::new(resolver),
            ));
        }
        let engine = FhirPathEngine::new(Arc::new(registry), provider.clone())
            .await
            .map_err(|e| e.to_string())?;
        let validation = create_validation_provider_from_embedded(embedded)
            .await
            .ok();
        Ok(Self {
            engine,
            provider,
            validation,
        })
    }

    async fn collection(&self, item: &Item) -> Result<Collection, String> {
        Collection::from_json_typed_arc(
            Arc::new(item.json.clone()),
            item.type_name.as_deref(),
            Some(self.provider.clone()),
        )
        .await
        .map_err(|e| e.to_string())
    }
}

impl Expressions for FhirPathExpressions {
    async fn evaluate(
        &self,
        expression: &str,
        focus: Option<&Item>,
        variables: &[(String, Item)],
    ) -> Result<Vec<Item>, String> {
        let input = match focus {
            Some(item) => self.collection(item).await?,
            None => Collection::empty(),
        };
        let context = EvaluationContext::new(
            input,
            self.provider.clone(),
            None,
            self.validation.clone(),
            None,
        );
        for (name, item) in variables {
            let mut values = self.collection(item).await?.into_vec();
            let value = if values.len() == 1 {
                values.remove(0)
            } else {
                FhirPathValue::collection(values)
            };
            context.set_variable(name.clone(), value);
        }
        let result = self
            .engine
            .evaluate(expression, &context)
            .await
            .map_err(|e| e.to_string())?;
        Ok(result.value.into_vec().into_iter().map(item).collect())
    }
}

fn item(value: FhirPathValue) -> Item {
    let info = value.type_info();
    let type_name = if info.namespace.as_deref() == Some("System") {
        match info.type_name.as_str() {
            "String" => Some("string".to_owned()),
            "Boolean" => Some("boolean".to_owned()),
            "Integer" => Some("integer".to_owned()),
            "Long" => Some("integer64".to_owned()),
            "Decimal" => Some("decimal".to_owned()),
            "Date" => Some("date".to_owned()),
            "DateTime" => Some("dateTime".to_owned()),
            "Time" => Some("time".to_owned()),
            "Quantity" => Some("Quantity".to_owned()),
            _ => None,
        }
    } else {
        info.name.clone().or_else(|| Some(info.type_name.clone()))
    };
    Item {
        json: value.to_json_value(),
        type_name,
    }
}
