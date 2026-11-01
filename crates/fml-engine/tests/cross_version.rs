//! Converts FHIR examples between versions with the HL7 cross-version maps and reports how many
//! convert. A measurement, not a pass/fail gate: it fails only on a panic.
//!
//! `FML_XVER` is the `input` folder of github.com/HL7/fhir-cross-version, `FML_XVER_EXAMPLES` a
//! folder of R4 JSON examples (hl7.org/fhir/R4/examples-json.zip); both default to what
//! `scripts/fetch-corpus.sh` downloads into `corpus/`. Without them it does nothing.
#![cfg(feature = "fhirpath")]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use fml_check::{CoreDefinitions, Release};
use fml_engine::{ConceptMapSet, Engine, FhirPathExpressions, MapResolver, MapSet};
use serde_json::Value;

fn files(dir: &Path, extension: &str) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|e| e == extension))
                .collect()
        })
        .unwrap_or_default();
    out.sort();
    out
}

#[tokio::test]
async fn r4_examples_to_r5() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus");
    let xver = std::env::var_os("FML_XVER")
        .map(PathBuf::from)
        .unwrap_or_else(|| corpus.join("fhir-cross-version/input"));
    let examples = std::env::var_os("FML_XVER_EXAMPLES")
        .map(PathBuf::from)
        .unwrap_or_else(|| corpus.join("r4-examples"));
    if !xver.join("R4toR5").is_dir() || !examples.is_dir() {
        eprintln!("no cross-version maps or R4 examples; run scripts/fetch-corpus.sh");
        return;
    }
    let mut maps = MapSet::new();
    for path in files(&xver.join("R4toR5"), "fml") {
        let compiled = fml_model::compile(&std::fs::read_to_string(&path).unwrap());
        maps.add(compiled.map);
    }
    let mut concept_maps = ConceptMapSet::new();
    for folder in ["codes", "types", "elements"] {
        for path in files(&xver.join(folder), "json") {
            if let Ok(json) = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()) {
                concept_maps.add(json);
            }
        }
    }
    let definitions = CoreDefinitions::new(Release::R5);
    let engine = Engine::new(
        &definitions,
        FhirPathExpressions::new(Release::R5).await.unwrap(),
    )
    .with_maps(&maps)
    .with_concept_maps(&concept_maps);

    let mut converted = 0;
    let mut with_warnings = 0;
    let mut no_map = 0;
    let mut errors: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let examples = files(&examples, "json");
    for path in &examples {
        let Ok(source) = serde_json::from_str::<Value>(&std::fs::read_to_string(path).unwrap())
        else {
            continue;
        };
        let Some(resource_type) = source["resourceType"].as_str() else {
            continue;
        };
        let url = format!("http://hl7.org/fhir/uv/xver/StructureMap/{resource_type}4to5");
        let Some(map) = maps.resolve(&url).into_iter().next() else {
            no_map += 1;
            continue;
        };
        match engine.transform(&map, &source).await {
            Ok(result) => {
                converted += 1;
                if !result.warnings.is_empty() {
                    with_warnings += 1;
                }
            }
            Err(error) => {
                // Group by message with the varying parts (quoted values) blanked.
                let mut key = String::new();
                let mut quoted = false;
                for c in error.message.chars() {
                    if c == '`' {
                        quoted = !quoted;
                        key.push_str(if quoted { "`…" } else { "`" });
                    } else if !quoted {
                        key.push(c);
                    }
                }
                errors.entry(key).or_default().push(format!(
                    "{} ({})",
                    path.file_name().unwrap().to_string_lossy(),
                    error.message
                ));
            }
        }
    }
    let failed: usize = errors.values().map(Vec::len).sum();
    eprintln!(
        "R4 → R5: {} examples, {converted} converted ({with_warnings} with values left out), {failed} failed, {no_map} without a map",
        examples.len()
    );
    let mut ranked: Vec<_> = errors.into_iter().collect();
    ranked.sort_by_key(|(_, files)| std::cmp::Reverse(files.len()));
    for (message, files) in ranked.iter().take(25) {
        eprintln!("  {:4}  {message}\n        e.g. {}", files.len(), files[0]);
    }
    // 2,863 at the time of writing; the 17 failures come from a ConceptMap in the maps that maps
    // `unmatched` to no usable relationship. A little room for changes in the upstream maps.
    assert!(
        converted >= 2_850,
        "only {converted} R4 examples converted to R5"
    );
}
