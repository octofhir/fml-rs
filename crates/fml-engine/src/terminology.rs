//! ConceptMaps for the `translate` transform.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use serde_json::Value;

/// A future a [`TranslateService`] returns.
pub type TranslateFuture<'a> =
    Pin<Box<dyn Future<Output = Result<Option<Translation>, String>> + 'a>>;

/// A terminology service that runs `ConceptMap/$translate` (transforms.md, "translate"), for the
/// ConceptMaps the engine does not hold itself: a FHIR server's own ConceptMaps, or an external
/// terminology server.
pub trait TranslateService {
    /// Translates `code` from `system` (when known) with the ConceptMap at `url`. `Ok(None)`: no
    /// translation; an error when the service fails or the translation is ambiguous.
    fn translate<'a>(
        &'a self,
        url: &'a str,
        system: Option<&'a str>,
        code: &'a str,
    ) -> TranslateFuture<'a>;
}

/// Finds ConceptMaps by canonical URL, for `translate(value, 'url', output)`.
pub trait ConceptMapResolver {
    fn resolve(&self, url: &str) -> Option<Arc<Value>>;
}

/// No ConceptMaps besides those contained in the map.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoConceptMaps;

impl ConceptMapResolver for NoConceptMaps {
    fn resolve(&self, _url: &str) -> Option<Arc<Value>> {
        None
    }
}

/// A fixed set of ConceptMaps (FHIR R4 or R5 JSON), found by `url`, `url|version`, and by the
/// canonical URLs they replace (`relatedArtifact` of type `predecessor`), so maps that still use
/// an old canonical find the renamed ConceptMap.
#[derive(Debug, Default, Clone)]
pub struct ConceptMapSet {
    by_url: HashMap<String, Arc<Value>>,
}

impl ConceptMapSet {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a ConceptMap; other resources are ignored. Returns whether it was one.
    pub fn add(&mut self, concept_map: Value) -> bool {
        if concept_map["resourceType"] != "ConceptMap" {
            return false;
        }
        let concept_map = Arc::new(concept_map);
        let mut keys = Vec::new();
        if let Some(url) = concept_map["url"].as_str() {
            keys.push(url.to_owned());
            if let Some(version) = concept_map["version"].as_str() {
                keys.push(format!("{url}|{version}"));
            }
        }
        for artifact in concept_map["relatedArtifact"]
            .as_array()
            .into_iter()
            .flatten()
        {
            if artifact["type"] == "predecessor"
                && let Some(resource) = artifact["resource"].as_str()
            {
                keys.push(resource.to_owned());
            }
        }
        for key in keys {
            self.by_url
                .entry(key)
                .or_insert_with(|| concept_map.clone());
        }
        true
    }

    pub fn len(&self) -> usize {
        self.by_url.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_url.is_empty()
    }
}

impl ConceptMapResolver for ConceptMapSet {
    fn resolve(&self, url: &str) -> Option<Arc<Value>> {
        self.by_url
            .get(url)
            .or_else(|| self.by_url.get(url.split('|').next().unwrap_or(url)))
            .cloned()
    }
}

/// The result of translating a code: the target system and the mapped target entry.
#[derive(Debug, Clone, PartialEq)]
pub struct Translation {
    pub system: Option<String>,
    pub code: Option<String>,
    pub display: Option<String>,
}

/// Translates `code` (from `system`, when known) with a ConceptMap in R5 or R4 form, as the
/// reference engine does: the first element for the code, and among its targets the one whose
/// relationship is `related-to`, `equivalent` or `source-is-narrower-than-target` (or none);
/// several such targets are ambiguous. Without an element for the code, the group's unmapped
/// rule applies (`use-source-code`, `fixed`, R4 `provided`). `Ok(None)`: the ConceptMap does not
/// know the code, and the reference engine leaves the target unset. An element whose targets are
/// all unrelated is an error.
pub fn translate(
    concept_map: &Value,
    system: Option<&str>,
    code: &str,
) -> Result<Option<Translation>, String> {
    let base = |uri: &str| uri.split('|').next().unwrap_or(uri).to_owned();
    for group in concept_map["group"].as_array().into_iter().flatten() {
        if let (Some(system), Some(source)) = (system, group["source"].as_str())
            && base(source) != base(system)
        {
            continue;
        }
        let target_system = group["target"].as_str().map(base);
        if let Some(element) = group["element"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|element| element["code"].as_str() == Some(code))
        {
            let accepted: Vec<&Value> = element["target"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|target| usable(target))
                .collect();
            return match accepted.as_slice() {
                [] => Err(format!("`{code}` has no usable translation")),
                [target] => Ok(Some(Translation {
                    system: target_system,
                    code: target["code"].as_str().map(str::to_owned),
                    display: target["display"].as_str().map(str::to_owned),
                })),
                _ => Err(format!("`{code}` has several translations")),
            };
        }
        let unmapped = &group["unmapped"];
        match unmapped["mode"].as_str() {
            Some("use-source-code" | "provided") => {
                return Ok(Some(Translation {
                    system: target_system,
                    code: Some(code.to_owned()),
                    display: None,
                }));
            }
            Some("fixed") => {
                return Ok(Some(Translation {
                    system: target_system,
                    code: unmapped["code"].as_str().map(str::to_owned),
                    display: unmapped["display"].as_str().map(str::to_owned),
                }));
            }
            _ => {}
        }
    }
    Ok(None)
}

/// A target the reference engine translates to: an R5 relationship of `related-to`,
/// `equivalent` or `source-is-narrower-than-target`, the R4 equivalences that convert to those,
/// or no relationship at all.
fn usable(target: &Value) -> bool {
    match (
        target["relationship"].as_str(),
        target["equivalence"].as_str(),
    ) {
        (Some(relationship), _) => matches!(
            relationship,
            "related-to" | "equivalent" | "source-is-narrower-than-target"
        ),
        (None, Some(equivalence)) => matches!(
            equivalence,
            "relatedto" | "inexact" | "equivalent" | "equal" | "wider" | "specializes"
        ),
        (None, None) => true,
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn concept_map() -> Value {
        json!({
            "resourceType": "ConceptMap",
            "url": "http://example.org/ConceptMap/new",
            "version": "1.0",
            "relatedArtifact": [{ "type": "predecessor", "resource": "http://example.org/ConceptMap/old" }],
            "group": [{
                "source": "http://example.org/a|4.0.1",
                "target": "http://example.org/b",
                "element": [
                    { "code": "x", "target": [{ "code": "X", "display": "Ex", "relationship": "equivalent" }] },
                    { "code": "y", "target": [{ "code": "Y", "relationship": "not-related-to" }] },
                    { "code": "z", "target": [{ "code": "Z", "equivalence": "wider" }] }
                ],
                "unmapped": { "mode": "fixed", "code": "UNK" }
            }]
        })
    }

    #[test]
    fn finds_maps_by_url_version_and_predecessor() {
        let mut set = ConceptMapSet::new();
        assert!(set.add(concept_map()));
        assert!(!set.add(json!({ "resourceType": "ValueSet", "url": "http://example.org/vs" })));
        for url in [
            "http://example.org/ConceptMap/new",
            "http://example.org/ConceptMap/new|1.0",
            "http://example.org/ConceptMap/new|2.0",
            "http://example.org/ConceptMap/old",
        ] {
            assert!(set.resolve(url).is_some(), "{url}");
        }
    }

    #[test]
    fn translates_codes() {
        let map = concept_map();
        let x = translate(&map, Some("http://example.org/a"), "x")
            .unwrap()
            .unwrap();
        assert_eq!(x.code.as_deref(), Some("X"));
        assert_eq!(x.system.as_deref(), Some("http://example.org/b"));
        assert_eq!(x.display.as_deref(), Some("Ex"));
        assert_eq!(
            translate(&map, None, "z").unwrap().unwrap().code.as_deref(),
            Some("Z")
        );
        // The code is mapped, but only to an unrelated concept.
        assert!(translate(&map, None, "y").is_err());
        // An unmapped code takes the group's unmapped rule.
        assert_eq!(
            translate(&map, None, "q").unwrap().unwrap().code.as_deref(),
            Some("UNK")
        );
        assert_eq!(translate(&map, Some("http://other.org"), "x"), Ok(None));
    }
}
