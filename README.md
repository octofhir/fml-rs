# fml-rs

FHIR Mapping Language (FML) tooling in Rust: parser, formatter, checker and
StructureMap engine, with a command-line tool `fml`.

> Early development. Nothing is published yet.

## Status

`fml-syntax`, `fml-model` and the `fml` CLI work today:

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

All 1,643 public maps we test against parse without errors, formatting them is idempotent, and
FML → StructureMap → FML → StructureMap gives back the same StructureMap for every one of them:
the [cross-version maps](https://github.com/HL7/fhir-cross-version), the
[R6 maps](https://github.com/brianpos/fhir-r6-maps), the Mapping Language IG examples and the
[FHIR test cases](https://github.com/FHIR/fhir-test-cases). Run the same check locally with
`scripts/fetch-corpus.sh && cargo test --test corpus -- --nocapture`.

## Usage

```sh
cargo install --path crates/fml-cli

fml check maps/                 # syntax errors, rustc-style; --format json for tools
fml fmt maps/                   # format in place
fml fmt --check maps/           # list unformatted files, exit 1 (CI)
cat map.fml | fml fmt -         # stdin to stdout
fml compile map.fml -o map.json # FML → StructureMap JSON
fml print map.json              # StructureMap JSON → FML
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
```

## Plan

| Crate | Purpose |
|---|---|
| `fml-syntax` | Lexer, lossless syntax tree, error-tolerant parser, diagnostics, formatter |
| `fml-model` | StructureMap model (R4, R5 and the Mapping Language IG), FML ⇄ StructureMap |
| `fml-check` | Type checking against FHIR structure definitions: paths, types, group signatures, imports |
| `fml-engine` | StructureMap execution: groups, dependent rules, list modes, transforms, FHIRPath, tracing |
| `fhir-convert` | FHIR version conversion built on the cross-version maps, with a loss report |
| `fml-cli` | The `fml` binary: `fmt`, `check`, `compile`, `print`, `run`, `convert` |

The grammar follows the [FHIR Mapping Language IG](https://build.fhir.org/ig/HL7/mapping-language-ig/)
and stays compatible with maps written for FHIR R4 and R5.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or
[MIT license](LICENSE-MIT) at your option.
