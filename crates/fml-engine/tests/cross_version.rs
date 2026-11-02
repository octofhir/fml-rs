//! Converts FHIR examples between versions with the HL7 cross-version maps and reports how many
//! convert, and how many come out equal to the same example in the R5 specification. The
//! specification examples were kept in step between releases, so most should match; where one
//! does not, the map, the engine or the example changed. A measurement: it fails only when far
//! fewer examples convert than before.
//!
//! `FML_XVER` is the `input` folder of github.com/HL7/fhir-cross-version, `FML_XVER_EXAMPLES` a
//! folder of R4 JSON examples (hl7.org/fhir/R4/examples-json.zip), `FML_XVER_EXPECTED` the R5
//! examples (hl7.org/fhir/R5/examples-json.zip); all default to what `scripts/fetch-corpus.sh`
//! downloads into `corpus/`. Without the maps and the R4 examples it does nothing.
#![cfg(feature = "fhirpath")]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use fml_check::{CoreDefinitions, Definitions, PackageDefinitions, Release};
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

fn corpus_path(variable: &str, default: &str) -> PathBuf {
    std::env::var_os(variable)
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../corpus")
                .join(default)
        })
}

/// One direction of conversion.
struct Direction {
    /// `R4toR5`: the folder of maps.
    folder: &'static str,
    /// `4to5`: the end of the map names.
    suffix: &'static str,
    /// The release the expected examples are in, for messages.
    target: &'static str,
}

/// How many examples converted.
struct Outcome {
    converted: usize,
}

async fn convert(
    direction: &Direction,
    definitions: &dyn Definitions,
    release: Release,
    expected: &Path,
) -> Option<Outcome> {
    let xver = corpus_path("FML_XVER", "fhir-cross-version/input");
    let examples = corpus_path("FML_XVER_EXAMPLES", "r4-examples");
    if !xver.join(direction.folder).is_dir() || !examples.is_dir() {
        eprintln!("no cross-version maps or R4 examples; run scripts/fetch-corpus.sh");
        return None;
    }
    let mut maps = MapSet::new();
    for path in files(&xver.join(direction.folder), "fml") {
        let compiled = fml_model::compile(&std::fs::read_to_string(&path).unwrap());
        maps.add(compiled.map);
    }
    let mut concept_maps = ConceptMapSet::new();
    for folder in ["codes", "types", "elements", "resources"] {
        for path in files(&xver.join(folder), "json") {
            if let Ok(json) = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()) {
                concept_maps.add(json);
            }
        }
    }
    let engine = Engine::new(
        definitions,
        FhirPathExpressions::new(release).await.unwrap(),
    )
    .with_maps(&maps)
    .with_concept_maps(&concept_maps);

    let mut converted = 0;
    let mut compared = 0;
    let mut equal = 0;
    let mut equal_without_text = 0;
    let mut differences: BTreeMap<String, Vec<String>> = BTreeMap::new();
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
        let url = format!(
            "http://hl7.org/fhir/uv/xver/StructureMap/{resource_type}{}",
            direction.suffix
        );
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
                let name = path.file_name().unwrap();
                if let Ok(text) = std::fs::read_to_string(expected.join(name))
                    && let Ok(want) = serde_json::from_str::<Value>(&text)
                {
                    compared += 1;
                    if result.target == want {
                        equal += 1;
                    }
                    let mut paths = Vec::new();
                    diff(
                        resource_type,
                        &without_text(&result.target),
                        &without_text(&want),
                        &mut paths,
                    );
                    if paths.is_empty() {
                        equal_without_text += 1;
                    }
                    // Definitional resources are regenerated for each release; their
                    // differences are noise for this comparison.
                    if CONFORMANCE.contains(&resource_type) {
                        continue;
                    }
                    for found in paths {
                        differences
                            .entry(found)
                            .or_default()
                            .push(name.to_string_lossy().into_owned());
                    }
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
        "{}: {} examples, {converted} converted ({with_warnings} with values left out), {failed} failed, {no_map} without a map",
        direction.folder,
        examples.len()
    );
    eprintln!(
        "compared with the {} examples: {compared}; equal {equal}, equal apart from the narrative {equal_without_text}",
        direction.target
    );
    let mut ranked: Vec<_> = differences.into_iter().collect();
    ranked.sort_by_key(|(_, files)| std::cmp::Reverse(files.len()));
    eprintln!("  where the other examples differ (element, examples):");
    for (path, files) in ranked.iter().take(40) {
        eprintln!("  {:4}  {path}   e.g. {}", files.len(), files[0]);
    }
    let mut ranked: Vec<_> = errors.into_iter().collect();
    ranked.sort_by_key(|(_, files)| std::cmp::Reverse(files.len()));
    for (message, files) in ranked.iter().take(25) {
        eprintln!("  {:4}  {message}\n        e.g. {}", files.len(), files[0]);
    }
    Some(Outcome { converted })
}

#[tokio::test]
async fn r4_examples_to_r5() {
    let definitions = CoreDefinitions::new(Release::R5);
    let direction = Direction {
        folder: "R4toR5",
        suffix: "4to5",
        target: "R5",
    };
    let expected = corpus_path("FML_XVER_EXPECTED", "r5-examples");
    let Some(outcome) = convert(&direction, &definitions, Release::R5, &expected).await else {
        return;
    };
    // 2,863 at the time of writing; the 17 failures come from a ConceptMap in the maps that maps
    // `unmatched` to no usable relationship. A little room for changes in the upstream maps.
    assert!(
        outcome.converted >= 2_850,
        "only {} R4 examples converted to R5",
        outcome.converted
    );
}

/// R4 to STU3: STU3 has no embedded definitions, so they come from the `hl7.fhir.r3.core`
/// package (`FML_R3_CORE`, its `package` folder), under the `http://hl7.org/fhir/3.0/` URLs the
/// maps use.
#[tokio::test]
async fn r4_examples_to_r3() {
    let core = corpus_path("FML_R3_CORE", "r3-core/package");
    if !core.is_dir() {
        eprintln!("no STU3 core package; run scripts/fetch-corpus.sh");
        return;
    }
    let mut definitions = PackageDefinitions::new(CoreDefinitions::new(Release::R4));
    let (added, problems) = definitions.add_release_folder(&core, "3.0").unwrap();
    eprintln!("STU3 definitions: {added}, {} unreadable", problems.len());
    for problem in problems.iter().take(5) {
        eprintln!("  {problem}");
    }
    let direction = Direction {
        folder: "R4toR3",
        suffix: "4to3",
        target: "STU3",
    };
    let expected = corpus_path("FML_XVER_EXPECTED_R3", "r3-examples");
    convert(&direction, &definitions, Release::R4, &expected).await;
}

/// Resource types the specification regenerates for each release.
const CONFORMANCE: &[&str] = &[
    "CapabilityStatement",
    "CodeSystem",
    "CompartmentDefinition",
    "ConceptMap",
    "GraphDefinition",
    "ImplementationGuide",
    "MessageDefinition",
    "NamingSystem",
    "OperationDefinition",
    "Questionnaire",
    "SearchParameter",
    "StructureDefinition",
    "StructureMap",
    "TerminologyCapabilities",
    "ValueSet",
];

/// The resource without its narrative, which the specification regenerates for each release.
fn without_text(resource: &Value) -> Value {
    let mut resource = resource.clone();
    if let Some(object) = resource.as_object_mut() {
        object.remove("text");
    }
    resource
}

/// The element paths where `ours` and `want` differ, without list indexes (`Patient.name.given`).
/// A path names the element that is missing, extra or different; nothing below it is listed.
fn diff(path: &str, ours: &Value, want: &Value, out: &mut Vec<String>) {
    match (ours, want) {
        (Value::Object(a), Value::Object(b)) => {
            for key in a.keys().chain(b.keys().filter(|k| !a.contains_key(*k))) {
                let child = format!("{path}.{}", key.trim_start_matches('_'));
                match (a.get(key), b.get(key)) {
                    (Some(x), Some(y)) => diff(&child, x, y, out),
                    (Some(_), None) => out.push(format!("{child} (extra)")),
                    (None, Some(_)) => out.push(format!("{child} (missing)")),
                    (None, None) => {}
                }
            }
        }
        (Value::Array(a), Value::Array(b)) if a.len() == b.len() => {
            for (x, y) in a.iter().zip(b) {
                diff(path, x, y, out);
            }
        }
        (Value::Array(_), Value::Array(_)) => out.push(format!("{path} (count)")),
        (a, b) if a != b => out.push(path.to_owned()),
        _ => {}
    }
    out.dedup();
}
