//! The structure-mapping tests of the FHIR test cases (github.com/FHIR/fhir-test-cases), as the
//! HL7 reference engine runs them. Tests with XML (CDA) output are skipped: JSON only.
//!
//! The test cases come from `corpus/fhir-test-cases` (`scripts/fetch-corpus.sh`) or
//! `FHIR_TEST_CASES`. Without them the test does nothing.
#![cfg(feature = "fhirpath")]

use std::path::{Path, PathBuf};

use fml_check::{CoreDefinitions, Release};
use fml_engine::{Engine, FhirPathExpressions};
use fml_model::StructureMap;
use serde_json::Value;

fn test_cases() -> PathBuf {
    match std::env::var_os("FHIR_TEST_CASES") {
        Some(dir) => PathBuf::from(dir),
        None => Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus/fhir-test-cases"),
    }
}

/// `<test name="…" source="…" map="…" output="…" />` entries of a manifest.
fn manifest(dir: &Path) -> Vec<(String, String, String, String)> {
    let Ok(text) = std::fs::read_to_string(dir.join("manifest.xml")) else {
        return Vec::new();
    };
    let attribute = |line: &str, name: &str| -> Option<String> {
        let start = line.find(&format!("{name}=\""))? + name.len() + 2;
        let end = start + line[start..].find('"')?;
        Some(line[start..end].to_owned())
    };
    text.lines()
        .filter(|line| line.trim_start().starts_with("<test "))
        .filter_map(|line| {
            Some((
                attribute(line, "name")?,
                attribute(line, "source")?,
                attribute(line, "map")?,
                attribute(line, "output")?,
            ))
        })
        .collect()
}

fn load_map(path: &Path) -> StructureMap {
    let text = std::fs::read_to_string(path).unwrap();
    if path.extension().is_some_and(|e| e == "json") {
        serde_json::from_str(&text).unwrap()
    } else {
        let compiled = fml_model::compile(&text);
        assert!(
            !compiled.has_errors(),
            "{}: {:?}",
            path.display(),
            compiled.diagnostics
        );
        compiled.map
    }
}

#[tokio::test]
async fn structure_mapping_test_cases() {
    let root = test_cases();
    let mut ran = 0;
    let mut failures = Vec::new();
    for (folder, release) in [("r5", Release::R5), ("r4b", Release::R4B)] {
        let dir = root.join(folder).join("structure-mapping");
        let tests = manifest(&dir);
        if tests.is_empty() {
            continue;
        }
        let definitions = CoreDefinitions::new(release);
        let expressions = FhirPathExpressions::new(release).await.unwrap();
        let engine = Engine::new(&definitions, expressions);
        for (name, source, map, output) in tests {
            if !output.ends_with(".json") {
                continue;
            }
            ran += 1;
            let map = load_map(&dir.join(&map));
            let source: Value =
                serde_json::from_str(&std::fs::read_to_string(dir.join(&source)).unwrap()).unwrap();
            let expected: Value =
                serde_json::from_str(&std::fs::read_to_string(dir.join(&output)).unwrap()).unwrap();
            match engine.transform(&map, &source).await {
                Ok(result) if result.target == expected => {}
                Ok(result) => failures.push(format!(
                    "{folder} {name}:\n  expected {expected}\n  got      {}",
                    result.target
                )),
                Err(error) => failures.push(format!("{folder} {name}: {error}")),
            }
        }
    }
    if ran == 0 {
        eprintln!("no FHIR test cases found; run scripts/fetch-corpus.sh");
        return;
    }
    eprintln!(
        "structure mapping: {} of {ran} passed",
        ran - failures.len()
    );
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}
