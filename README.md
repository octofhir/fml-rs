# fml-rs

FHIR Mapping Language (FML) tooling in Rust: parser, formatter, checker and
StructureMap engine, with a command-line tool `fml`.

> Early development. Nothing is published yet.

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
