# fml

A command-line tool for the [FHIR Mapping Language](https://build.fhir.org/ig/HL7/mapping-language-ig/):
check, format, compile, print and run FML maps. A single binary, no JVM.

```sh
cargo install fml-cli
```

Or download a binary for Linux, macOS or Windows from the
[releases page](https://github.com/octofhir/fml-rs/releases).

```sh
fml check maps/                    # syntax and type errors, rustc-style
fml check --format json maps/      # the same for editors and CI
fml fmt maps/                      # format in place
fml fmt --check maps/              # list unformatted files, exit 1 (CI)
fml compile map.fml -o map.json    # FML → StructureMap JSON (--fhir r4 for the R4 form)
fml print map.json                 # StructureMap JSON (R4 or R5) → FML
fml run map.fml input.json         # run a map over a FHIR resource
fml run Patient4to5.fml patient.json --maps R4toR5/ --maps codes/
                                   # with imported groups and ConceptMaps for translate()
fml run map.fml in.json --debug    # the $transform debug trace, as Parameters
fml run map.fml in.json --bundle   # the target and the resources the map produced
fml check --package hl7.fhir.us.core@6.1.0 maps/
                                   # with the profiles and extensions of a FHIR package
fml lsp --fhir r4                  # language server for editors, over stdio
```

What each command is built on:

- `check`: the [fml-check](https://crates.io/crates/fml-check) type checker against FHIR R4, R4B,
  R5 and R6; `--fhir` picks the release unversioned core URLs refer to, `--package` adds a FHIR
  package (a folder, or `name@version` from the package registry).
- `fmt`: the [fml-syntax](https://crates.io/crates/fml-syntax) formatter, which keeps comments
  and refuses to write text that would change the map.
- `compile` and `print`: [fml-model](https://crates.io/crates/fml-model), which builds the same
  StructureMap as the HL7 reference compiler.
- `run`: the [fml-engine](https://crates.io/crates/fml-engine) StructureMap engine with FHIRPath;
  `--maps` and `--package` bring imported StructureMaps and ConceptMaps, `--debug` writes the
  trace, `--bundle` a collection Bundle with the resources the map produced.
- `lsp`: the [fml-lsp](https://crates.io/crates/fml-lsp) language server over standard input and
  output, with the `--fhir` and `--package` options of `check`. It is the default `lsp` feature;
  `--no-default-features` builds without it.

Part of [fml-rs](https://github.com/octofhir/fml-rs). Licensed under MIT or Apache-2.0.
