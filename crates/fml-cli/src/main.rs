//! `fml`: format and check FHIR Mapping Language files.

use std::io::{IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand, ValueEnum};
use fml_check::{CoreDefinitions, Release};
use fml_engine::{Engine, FhirPathExpressions, MapSet};
use fml_syntax::{Diagnostic, FormatError, FormatOptions, LineIndex, Severity, format, parse};

#[derive(Parser)]
#[command(name = "fml", version, about = "FHIR Mapping Language tools")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Format FML files in place.
    Fmt {
        /// Files or directories; `-` reads standard input and writes standard output.
        #[arg(required = true)]
        paths: Vec<PathBuf>,
        /// Report files that are not formatted instead of rewriting them.
        #[arg(long)]
        check: bool,
        /// Spaces per indentation level.
        #[arg(long, default_value_t = 2)]
        indent: usize,
        /// Indent with tabs.
        #[arg(long)]
        tabs: bool,
    },
    /// Report syntax errors and type errors.
    Check {
        /// Files or directories.
        #[arg(required = true)]
        paths: Vec<PathBuf>,
        #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
        format: OutputFormat,
        /// FHIR release that unversioned core URLs refer to: r4, r4b, r5 or r6.
        #[arg(long, default_value = "r5", value_parser = parse_release)]
        fhir: Release,
        /// Check syntax only.
        #[arg(long)]
        syntax_only: bool,
    },
    /// Print the syntax tree of a file.
    Parse { path: PathBuf },
    /// Compile an FML file to a StructureMap (JSON).
    Compile {
        path: PathBuf,
        /// Write the StructureMap here instead of standard output.
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Run a map (FML or StructureMap JSON) over a FHIR JSON input.
    Run {
        map: PathBuf,
        input: PathBuf,
        /// Write the result here instead of standard output.
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// FHIR release of the input and output: r4, r4b, r5 or r6.
        #[arg(long, default_value = "r5", value_parser = parse_release)]
        fhir: Release,
        /// Maps to import groups from: files or directories of FML or StructureMap JSON.
        #[arg(long = "maps")]
        maps: Vec<PathBuf>,
    },
    /// Print a StructureMap (JSON) as FML.
    Print {
        path: PathBuf,
        /// Write the FML here instead of standard output.
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
}

#[derive(Clone, Copy, ValueEnum)]
enum OutputFormat {
    Text,
    Json,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match cli.command {
        Command::Fmt {
            paths,
            check,
            indent,
            tabs,
        } => fmt(
            &paths,
            check,
            &FormatOptions {
                indent_width: indent,
                use_tabs: tabs,
            },
        ),
        Command::Check {
            paths,
            format,
            fhir,
            syntax_only,
        } => check(
            &paths,
            format,
            (!syntax_only).then(|| CoreDefinitions::new(fhir)),
        ),
        Command::Parse { path } => dump(&path),
        Command::Compile { path, output } => compile(&path, output.as_deref()),
        Command::Print { path, output } => print(&path, output.as_deref()),
        Command::Run {
            map,
            input,
            output,
            fhir,
            maps,
        } => run(&map, &input, output.as_deref(), fhir, &maps),
    };
    match result {
        Ok(code) => code,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::from(2)
        }
    }
}

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

/// Expands directories to the `.fml` and `.map` files inside them.
fn files(paths: &[PathBuf]) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    for path in paths {
        if path.is_dir() {
            walk(path, &mut out)?;
        } else if path.exists() {
            out.push(path.clone());
        } else {
            return Err(format!("{}: no such file or directory", path.display()).into());
        }
    }
    Ok(out)
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    let mut entries: Vec<_> = std::fs::read_dir(dir)?.collect::<std::io::Result<_>>()?;
    entries.sort_by_key(|e| e.path());
    for entry in entries {
        let path = entry.path();
        let name = entry.file_name();
        if name.to_string_lossy().starts_with('.') || name == "node_modules" || name == "target" {
            continue;
        }
        if path.is_dir() {
            walk(&path, out)?;
        } else if path.extension().is_some_and(|e| e == "fml" || e == "map") {
            out.push(path);
        }
    }
    Ok(())
}

fn fmt(paths: &[PathBuf], check: bool, options: &FormatOptions) -> Result<ExitCode> {
    if paths.len() == 1 && paths[0] == Path::new("-") {
        let mut text = String::new();
        std::io::stdin().read_to_string(&mut text)?;
        return match format(&text, options) {
            Ok(formatted) => {
                std::io::stdout().write_all(formatted.as_bytes())?;
                Ok(ExitCode::SUCCESS)
            }
            Err(error) => {
                report_format_error(Path::new("<stdin>"), &text, &error);
                Ok(ExitCode::FAILURE)
            }
        };
    }
    let mut failed = false;
    let mut unformatted = 0;
    let mut changed = 0;
    for path in files(paths)? {
        let text = std::fs::read_to_string(&path)?;
        match format(&text, options) {
            Ok(formatted) if formatted == text => {}
            Ok(_) if check => {
                println!("{}", path.display());
                unformatted += 1;
            }
            Ok(formatted) => {
                std::fs::write(&path, formatted)?;
                changed += 1;
            }
            Err(error) => {
                report_format_error(&path, &text, &error);
                failed = true;
            }
        }
    }
    if check && unformatted > 0 {
        eprintln!("{unformatted} file(s) need formatting");
    } else if changed > 0 {
        eprintln!("formatted {changed} file(s)");
    }
    Ok(if failed || unformatted > 0 {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    })
}

fn report_format_error(path: &Path, text: &str, error: &FormatError) {
    match error {
        FormatError::Syntax(diagnostics) => {
            let mut stderr = std::io::stderr();
            let color = stderr.is_terminal();
            for diagnostic in diagnostics {
                let _ = render(&mut stderr, path, text, diagnostic, color);
            }
        }
        FormatError::Unstable(_) => eprintln!("{}: {error}", path.display()),
    }
}

fn parse_release(name: &str) -> std::result::Result<Release, String> {
    Release::parse(name)
        .ok_or_else(|| format!("unknown FHIR release `{name}`; use r4, r4b, r5 or r6"))
}

fn check(
    paths: &[PathBuf],
    output: OutputFormat,
    definitions: Option<CoreDefinitions>,
) -> Result<ExitCode> {
    let mut errors = 0;
    let mut files_checked = 0;
    let mut json = Vec::new();
    let mut stdout = std::io::stdout();
    let color = stdout.is_terminal();
    for path in files(paths)? {
        let text = std::fs::read_to_string(&path)?;
        files_checked += 1;
        let parse = parse(&text);
        let index = LineIndex::new(&text);
        let mut diagnostics = parse.diagnostics().to_vec();
        if let Some(definitions) = &definitions
            && !parse.has_errors()
        {
            diagnostics.extend(fml_check::check(&parse.tree(), definitions));
        }
        for diagnostic in &diagnostics {
            if diagnostic.severity == Severity::Error {
                errors += 1;
            }
            match output {
                OutputFormat::Text => render(&mut stdout, &path, &text, diagnostic, color)?,
                OutputFormat::Json => {
                    let start = index.line_col(&text, diagnostic.range.start());
                    let end = index.line_col(&text, diagnostic.range.end());
                    json.push(serde_json::json!({
                        "file": path.display().to_string(),
                        "severity": severity_name(diagnostic.severity),
                        "message": diagnostic.message,
                        "start": { "line": start.line + 1, "column": start.col_utf16 + 1 },
                        "end": { "line": end.line + 1, "column": end.col_utf16 + 1 },
                    }));
                }
            }
        }
    }
    match output {
        OutputFormat::Json => println!("{}", serde_json::to_string_pretty(&json)?),
        OutputFormat::Text => {
            eprintln!("checked {files_checked} file(s): {errors} error(s)");
        }
    }
    Ok(if errors > 0 {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    })
}

fn compile(path: &Path, output: Option<&Path>) -> Result<ExitCode> {
    let text = std::fs::read_to_string(path)?;
    let compiled = fml_model::compile(&text);
    let mut stderr = std::io::stderr();
    let color = stderr.is_terminal();
    for diagnostic in &compiled.diagnostics {
        render(&mut stderr, path, &text, diagnostic, color)?;
    }
    if compiled.has_errors() {
        return Ok(ExitCode::FAILURE);
    }
    let json = serde_json::to_string_pretty(&compiled.map)? + "\n";
    write_output(output, &json)?;
    Ok(ExitCode::SUCCESS)
}

fn print(path: &Path, output: Option<&Path>) -> Result<ExitCode> {
    let text = std::fs::read_to_string(path)?;
    let map: fml_model::StructureMap = serde_json::from_str(&text)
        .map_err(|e| format!("{}: not a StructureMap: {e}", path.display()))?;
    if map.resource_type != "StructureMap" {
        return Err(format!(
            "{}: expected a StructureMap, found {}",
            path.display(),
            map.resource_type
        )
        .into());
    }
    write_output(output, &fml_model::print(&map))?;
    Ok(ExitCode::SUCCESS)
}

/// A StructureMap from FML or JSON.
fn load_map(path: &Path) -> Result<fml_model::StructureMap> {
    let text = std::fs::read_to_string(path)?;
    if path.extension().is_some_and(|e| e == "json") {
        return Ok(serde_json::from_str(&text)
            .map_err(|e| format!("{}: not a StructureMap: {e}", path.display()))?);
    }
    let compiled = fml_model::compile(&text);
    if compiled.has_errors() {
        let mut stderr = std::io::stderr();
        let color = stderr.is_terminal();
        for diagnostic in &compiled.diagnostics {
            render(&mut stderr, path, &text, diagnostic, color)?;
        }
        return Err(format!("{}: the map has errors", path.display()).into());
    }
    Ok(compiled.map)
}

fn run(
    map: &Path,
    input: &Path,
    output: Option<&Path>,
    release: Release,
    imports: &[PathBuf],
) -> Result<ExitCode> {
    let structure_map = load_map(map)?;
    let source: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(input)?)
        .map_err(|e| format!("{}: not JSON: {e}", input.display()))?;
    let mut maps = MapSet::new();
    for path in imports {
        let candidates = if path.is_dir() {
            let mut found = files(std::slice::from_ref(path))?;
            for entry in std::fs::read_dir(path)? {
                let entry = entry?.path();
                if entry.extension().is_some_and(|e| e == "json") {
                    found.push(entry);
                }
            }
            found
        } else {
            vec![path.clone()]
        };
        for candidate in candidates {
            maps.add(load_map(&candidate)?);
        }
    }
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .map_err(|e| format!("cannot start the runtime: {e}"))?;
    let definitions = CoreDefinitions::new(release);
    let transformed = runtime.block_on(async {
        let expressions = FhirPathExpressions::new(release).await?;
        Engine::new(&definitions, expressions)
            .with_maps(&maps)
            .transform(&structure_map, &source)
            .await
            .map_err(|e| e.to_string())
    })?;
    for message in &transformed.log {
        eprintln!("log: {message}");
    }
    write_output(
        output,
        &(serde_json::to_string_pretty(&transformed.target)? + "\n"),
    )?;
    Ok(ExitCode::SUCCESS)
}

fn write_output(output: Option<&Path>, text: &str) -> Result<()> {
    match output {
        Some(path) => std::fs::write(path, text)?,
        None => std::io::stdout().write_all(text.as_bytes())?,
    }
    Ok(())
}

fn dump(path: &Path) -> Result<ExitCode> {
    let text = std::fs::read_to_string(path)?;
    let parse = parse(&text);
    println!("{:#?}", parse.syntax());
    let mut stderr = std::io::stderr();
    let color = stderr.is_terminal();
    for diagnostic in parse.diagnostics() {
        render(&mut stderr, path, &text, diagnostic, color)?;
    }
    Ok(if parse.has_errors() {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    })
}

fn severity_name(severity: Severity) -> &'static str {
    match severity {
        Severity::Error => "error",
        Severity::Warning => "warning",
    }
}

/// Writes a diagnostic with the offending source line and a caret underline.
fn render(
    out: &mut impl Write,
    path: &Path,
    text: &str,
    diagnostic: &Diagnostic,
    color: bool,
) -> std::io::Result<()> {
    let index = LineIndex::new(text);
    let start = index.line_col(text, diagnostic.range.start());
    let line_text = text
        .lines()
        .nth(start.line as usize)
        .unwrap_or("")
        .trim_end_matches('\r');
    let (accent, bold, reset) = if !color {
        ("", "", "")
    } else if diagnostic.severity == Severity::Error {
        ("\x1b[31m", "\x1b[1m", "\x1b[0m")
    } else {
        ("\x1b[33m", "\x1b[1m", "\x1b[0m")
    };
    let column = start.col as usize;
    let width = usize::from(diagnostic.range.len())
        .min(line_text.len().saturating_sub(column))
        .max(1);
    let indent: String = line_text[..column.min(line_text.len())]
        .chars()
        .map(|c| if c == '\t' { '\t' } else { ' ' })
        .collect();
    let number = (start.line + 1).to_string();
    let gutter = " ".repeat(number.len());
    writeln!(
        out,
        "{accent}{bold}{}{reset}{bold}: {}{reset}",
        severity_name(diagnostic.severity),
        diagnostic.message
    )?;
    writeln!(
        out,
        "{gutter}--> {}:{}:{}",
        path.display(),
        start.line + 1,
        start.col_utf16 + 1
    )?;
    writeln!(out, "{gutter} |")?;
    writeln!(out, "{number} | {line_text}")?;
    writeln!(
        out,
        "{gutter} | {indent}{accent}{}{reset}",
        "^".repeat(width)
    )?;
    writeln!(out)
}
