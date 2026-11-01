//! Type-checks the corpus. The cross-version maps (R2–R5) are expected to be clean: anything
//! reported there is a checker bug, so it fails the test. Other maps are summarised.
#![cfg(feature = "fhirschema")]

use std::path::{Path, PathBuf};

use fml_check::{CoreDefinitions, Release, check};
use fml_syntax::{LineIndex, parse};

fn corpus_roots() -> Vec<PathBuf> {
    match std::env::var_os("FML_CORPUS") {
        Some(paths) => std::env::split_paths(&paths).collect(),
        None => vec![Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus")],
    }
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if path
                .file_name()
                .is_some_and(|n| n == "node_modules" || n == ".git")
            {
                continue;
            }
            collect(&path, out);
        } else if path.extension().is_some_and(|e| e == "fml" || e == "map") {
            out.push(path);
        }
    }
}

#[test]
fn checks_the_corpus() {
    let mut files = Vec::new();
    for root in corpus_roots() {
        collect(&root, &mut files);
    }
    files.sort();
    if files.is_empty() {
        eprintln!("no corpus found; run scripts/fetch-corpus.sh");
        return;
    }
    let definitions = CoreDefinitions::new(Release::R5);
    let mut reported = Vec::new();
    let mut regressions = Vec::new();
    for path in &files {
        let Ok(text) = std::fs::read_to_string(path) else {
            continue;
        };
        let parse = parse(&text);
        if parse.has_errors() {
            continue;
        }
        let index = LineIndex::new(&text);
        for diagnostic in check(&parse.tree(), &definitions) {
            let at = index.line_col(&text, diagnostic.range.start());
            let line = format!(
                "{}:{}: {}",
                path.display(),
                at.line + 1,
                diagnostic.message
            );
            if path.to_string_lossy().contains("fhir-cross-version") {
                regressions.push(line);
            } else {
                reported.push(line);
            }
        }
    }
    eprintln!(
        "check: {} files, {} findings outside the cross-version maps",
        files.len(),
        reported.len()
    );
    for line in &reported {
        eprintln!("  {line}");
    }
    assert!(regressions.is_empty(), "{}", regressions.join("\n"));
}
