# fml-rs

FHIR Mapping Language (FML) tooling in Rust: parser, formatter, checker and
StructureMap engine, with a command-line tool `fml`.

> Early development. Nothing is published yet.

## Status

`fml-syntax`, `fml-model`, `fml-check`, `fml-engine` and the `fml` CLI work today:

- lossless syntax tree: every byte of the input, comments included, is kept;
- error-tolerant parser with precise diagnostics, so one mistake does not hide the rest of the file;
- formatter that keeps every comment and verifies its output parses back to the same map;
- grammar of the Mapping Language IG plus the forms used by maps written for R3–R5:
  `map "url" = "name"`, unparenthesised `where`, embedded `conceptmap`, `<<type+>>` groups.

`fml-model` compiles FML to a StructureMap the way the HL7 reference compiler
(`StructureMapUtilities` in org.hl7.fhir.core) does: generated rule names, the `vvv` variables of
the simple copy form, batch identity rules, embedded concept maps as contained ConceptMaps,
documentation from comments, FHIRPath stored in canonical form. It also prints a StructureMap back
as FML, including maps built by older IG Publisher versions.

`fml-check` follows every rule with the types of its variables, against the FHIR R4, R4B, R5 and
R6 definitions (including the version-specific `http://hl7.org/fhir/4.0/…` URLs of the
cross-version maps), and reports elements that do not exist, type annotations that are not a type
of the element, unknown variables, writes into source variables, unknown or abstract types in
`create`, wrong transform arity, and calls to undeclared groups or with the wrong number of
arguments. Untyped groups are checked with the types they are called with. The cross-version maps
check clean; elsewhere it finds real mistakes, such as the `tgt.identifer` typo in the FHIR test
cases or a group in the R6 maps that uses a parameter it does not declare.

`fml-engine` runs a StructureMap over FHIR JSON the way the reference engine does: extended
groups, source filters (type, `where`, `check`, `log`, list modes, defaults), targets that create
or reuse elements (repeating elements, choice elements written as `valueReference`, `share`),
dependent groups, default groups chosen by type for simple copies, imported maps, and the
transforms `create`, `copy`, `evaluate`, `truncate`, `cast`, `append`, `translate` (contained
ConceptMaps), `reference`, `pointer`, `uuid`, `cc`, `c`, `id`, `cp` and `qty`. A rule may also have
several sources. FHIRPath comes through a small trait; the `fhirpath` feature uses
[octofhir-fhirpath](https://github.com/octofhir/fhirpath-rs). It passes all JSON structure-mapping
tests of the FHIR test cases (the CDA ones need XML, which is out of scope).

All 1,643 public maps we test against parse without errors, formatting them is idempotent,
FML → StructureMap → FML → StructureMap gives back the same StructureMap for every one of them,
and the StructureMap is identical to the one the reference compiler builds (`scripts/reference.sh`
runs it), except for two places where we keep what the map says: whitespace-only string arguments
such as `append(a, ' ', b)`, which the reference JSON writer drops, and calendar durations such as
`5 days`, which it rewrites as the quantity `5 'days'`. The corpus:
the [cross-version maps](https://github.com/HL7/fhir-cross-version), the
[R6 maps](https://github.com/brianpos/fhir-r6-maps), the Mapping Language IG examples and the
[FHIR test cases](https://github.com/FHIR/fhir-test-cases). Run the same check locally with
`scripts/fetch-corpus.sh && cargo test --test corpus -- --nocapture`.

## Usage

```sh
cargo install --path crates/fml-cli

fml check maps/                 # syntax and type errors, rustc-style; --format json for tools
fml check --fhir r4 maps/       # unversioned core URLs mean R4 (default r5)
fml fmt maps/                   # format in place
fml fmt --check maps/           # list unformatted files, exit 1 (CI)
cat map.fml | fml fmt -         # stdin to stdout
fml compile map.fml -o map.json # FML → StructureMap JSON
fml print map.json              # StructureMap JSON → FML
fml run map.fml input.json      # run a map; --maps dir/ for imported maps, --fhir r4
fml parse map.fml               # print the syntax tree
```

As a library:

```rust
let parse = fml_syntax::parse(source);
for diagnostic in parse.diagnostics() {
    eprintln!("{}", diagnostic.message);
}
for group in parse.tree().groups() {
    for rule in group.rules() {
        // sources, targets, transforms, dependent rules…
    }
}
let formatted = fml_syntax::format(source, &Default::default())?;

let compiled = fml_model::compile(source);       // StructureMap + diagnostics
let json = serde_json::to_string_pretty(&compiled.map)?;
let fml = fml_model::print(&compiled.map);       // back to FML

let definitions = fml_check::CoreDefinitions::new(fml_check::Release::R5);
let expressions = fml_engine::FhirPathExpressions::new(fml_check::Release::R5).await?;
let output = fml_engine::Engine::new(&definitions, expressions)
    .transform(&compiled.map, &input)
    .await?;                                     // output.target is FHIR JSON
```

## Plan

| Crate | Purpose |
|---|---|
| `fml-syntax` | Lexer, lossless syntax tree, error-tolerant parser, diagnostics, formatter |
| `fml-model` | StructureMap model (R4, R5 and the Mapping Language IG), FML ⇄ StructureMap |
| `fml-check` | Type checking against FHIR structure definitions: paths, types, variables, group calls |
| `fml-engine` | StructureMap execution: groups, dependent rules, list modes, transforms, FHIRPath, tracing |
| `fhir-convert` | FHIR version conversion built on the cross-version maps, with a loss report |
| `fml-cli` | The `fml` binary: `fmt`, `check`, `compile`, `print`, `run`, `convert` |

The grammar follows the [FHIR Mapping Language IG](https://build.fhir.org/ig/HL7/mapping-language-ig/)
and stays compatible with maps written for FHIR R4 and R5.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or
[MIT license](LICENSE-MIT) at your option.
