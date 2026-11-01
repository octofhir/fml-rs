//! Runs the parser and formatter over a corpus of real maps.
//!
//! `scripts/fetch-corpus.sh` downloads the corpus into `corpus/`; `FML_CORPUS` points elsewhere
//! (several directories separated by `:`). Without a corpus the test does nothing.

use std::path::{Path, PathBuf};

use fml_syntax::{FormatOptions, LineIndex, format, parse};

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
fn corpus() {
    let mut files = Vec::new();
    for root in corpus_roots() {
        collect(&root, &mut files);
    }
    files.sort();
    if files.is_empty() {
        eprintln!("no corpus found; run scripts/fetch-corpus.sh");
        return;
    }
    let options = FormatOptions::default();
    let mut parsed = 0;
    let mut problems = Vec::new();
    let mut syntax_errors = Vec::new();
    for path in &files {
        let Ok(text) = std::fs::read_to_string(path) else {
            continue;
        };
        let result = parse(&text);
        if result.syntax().to_string() != text {
            problems.push(format!("{}: tree is not lossless", path.display()));
            continue;
        }
        if result.has_errors() {
            let index = LineIndex::new(&text);
            let first = &result.diagnostics()[0];
            let at = index.line_col(&text, first.range.start());
            syntax_errors.push(format!(
                "{}:{}:{}: {}",
                path.display(),
                at.line + 1,
                at.col + 1,
                first.message
            ));
            continue;
        }
        parsed += 1;
        match format(&text, &options) {
            Ok(once) => match format(&once, &options) {
                Ok(twice) if twice == once => {}
                Ok(_) => problems.push(format!("{}: formatting is not idempotent", path.display())),
                Err(error) => {
                    problems.push(format!("{}: reformat failed: {error}", path.display()))
                }
            },
            Err(error) => problems.push(format!("{}: {error}", path.display())),
        }
    }
    eprintln!(
        "corpus: {} files, {} parsed ({:.1}%), {} with syntax errors",
        files.len(),
        parsed,
        100.0 * parsed as f64 / files.len() as f64,
        syntax_errors.len()
    );
    for error in &syntax_errors {
        eprintln!("  {error}");
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}
