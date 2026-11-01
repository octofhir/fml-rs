//! Compiles every map of the corpus, prints it back to FML and compiles that again: the two
//! StructureMaps must be equal. See `scripts/fetch-corpus.sh`; `FML_CORPUS` overrides the location.

use std::path::{Path, PathBuf};

use fml_model::{compile, print};

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
fn print_round_trips() {
    let mut files = Vec::new();
    for root in corpus_roots() {
        collect(&root, &mut files);
    }
    files.sort();
    if files.is_empty() {
        eprintln!("no corpus found; run scripts/fetch-corpus.sh");
        return;
    }
    let mut failures = Vec::new();
    let mut compiled_clean = 0;
    for path in &files {
        let Ok(text) = std::fs::read_to_string(path) else {
            continue;
        };
        let first = compile(&text);
        if first.has_errors() {
            continue;
        }
        compiled_clean += 1;
        let printed = print(&first.map);
        let second = compile(&printed);
        if second.has_errors() {
            failures.push(format!(
                "{}: printed FML does not compile: {:?}\n{printed}",
                path.display(),
                second.diagnostics
            ));
        } else if second.map != first.map {
            let a = serde_json::to_string_pretty(&first.map).unwrap();
            let b = serde_json::to_string_pretty(&second.map).unwrap();
            let line = a
                .lines()
                .zip(b.lines())
                .position(|(x, y)| x != y)
                .unwrap_or(0);
            failures.push(format!(
                "{}: map changed after printing, near line {}:\n  {}\n  {}",
                path.display(),
                line + 1,
                a.lines().nth(line).unwrap_or(""),
                b.lines().nth(line).unwrap_or("")
            ));
        } else if print(&second.map) != printed {
            failures.push(format!("{}: printing is not stable", path.display()));
        }
    }
    eprintln!(
        "corpus: {} files, {} compiled without errors, {} round-trip failures",
        files.len(),
        compiled_clean,
        failures.len()
    );
    for failure in failures.iter().take(15) {
        eprintln!("{failure}\n");
    }
    assert!(failures.is_empty());
}
