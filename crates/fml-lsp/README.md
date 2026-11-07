# fml-lsp

Language server for the [FHIR Mapping Language](https://build.fhir.org/ig/HL7/mapping-language-ig/).
It works on the same syntax tree and type checker as `fml check`, so what the editor shows is what
the checker reports.

- **Diagnostics** as you type: syntax, compile and type problems. Type problems wait until the
  syntax errors are fixed.
- **Completion**:
  - elements after `.`, from the variable's type followed through the path, also in groups
    without parameter types and in rules that are not finished yet;
  - FHIRPath functions inside expressions;
  - variables in scope;
  - groups after `then`, with their parameters as placeholders;
  - transforms after `=`;
  - after `:`, the types of a choice element and the `uses` aliases;
  - keywords.
- **Hover**:
  - the type of a variable or element (`Patient.name : HumanName [0..*]`);
  - a group's signature;
  - a transform's signature and description, from the IG.
- **Go to definition**, **find references** and **rename** for variables and groups. A type
  annotation goes to its `uses` alias.
- **Signature help** for transforms and group calls.
- **Outline**: `uses`, constants, ConceptMaps, and groups with their parameters and rules.
- **Formatting** with the `fml fmt` formatter.
- **Semantic tokens**: variables, parameters, elements, groups, transforms and types each get a
  colour of their own. Declarations and constants are marked.

## Running it

`fml lsp` runs the server over standard input and output. It takes the same `--fhir` and
`--package` options as `fml check`:

```sh
fml lsp --fhir r4 --package hl7.fhir.us.core@6.1.0
```

Point any LSP client at that command, for files ending in `.fml` or `.map`.

## As a library

`Server` does no I/O. It takes a JSON-RPC message and returns what to send back: the response,
and notifications such as diagnostics. A host can carry the messages over any transport, such as
a WebSocket in a web console:

```rust
use std::sync::Arc;
use fml_lsp::Server;

let mut server = Server::new(Arc::new(definitions)); // any fml_check::Definitions
for reply in server.handle(&message) {
    send(reply);
}
```

`run` and `run_stdio` carry the messages over a reader and a writer instead.
`Server::set_definitions` swaps the definitions (when packages change) and checks the open
documents again.

With the `fhirschema` feature, `Server::with_release` serves the core definitions of FHIR R4,
R4B, R5 or R6.

Part of [fml-rs](https://github.com/octofhir/fml-rs). Licensed under MIT or Apache-2.0.
