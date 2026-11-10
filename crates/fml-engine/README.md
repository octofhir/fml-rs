# fml-engine

A StructureMap engine for the [FHIR Mapping Language](https://build.fhir.org/ig/HL7/mapping-language-ig/):
runs maps over FHIR JSON, the way the HL7 reference engine does.

- **Passes the reference tests.** Every JSON structure-mapping test of the
  [FHIR test cases](https://github.com/FHIR/fhir-test-cases) (12 of 12, R4B and R5).
- **Converts FHIR versions.** With the HL7 [cross-version maps](https://github.com/HL7/fhir-cross-version)
  and their ConceptMaps, it converts 2,863 of the 2,881 R4 specification examples that have a
  map to R5: R4 `medicationReference` becomes the R5 CodeableReference, Encounter `class` becomes a
  list of CodeableConcepts, and so on. A code a ConceptMap does not know is left out, as the
  reference engine does, and reported in `warnings`.
- **The whole language.** Groups and `extends`, source filters (type, `where`, `check`, `log`,
  list modes, `default`), targets that create or reuse elements (repeating, choice elements written
  as `valueReference`, `share`), dependent groups, default groups chosen by type, imported maps,
  primitives with their `id` and extensions, and rules with several sources.
- **Transforms:** `create`, `copy`, `evaluate`, `truncate`, `cast`, `append`, `translate`,
  `reference`, `pointer`, `uuid`, `cc`, `c`, `id`, `cp`, `qty`, `escape` (html, xml, json), and
  the IG's date and time transforms: `toDateTime`, `toDate`, `toTime` with format patterns
  (`dd/MM/yyyy HH:mm`, `MMMM d, yyyy`, …) and `unixToDateTime`, `unixToDate`, `unixToTime`.
  Time zones can be named, with `z` in a format or as the timezone of `unixTo*`: IANA ids
  (`America/New_York`), Windows names (`Eastern Standard Time`) and abbreviations that name one
  offset (`EST`, `CEST`). The result has the offset in force at that date, daylight saving
  included. Abbreviations shared by several zones (`CST`, `IST`) are refused.
  `dateOp` is listed among the StructureMap transforms, but neither the IG nor the transform codes
  define it ("Parameters to be documented"), so it is refused with that reason and `fml check`
  warns about it.
- **Produced resources.** Resources a map creates outside the target (a `create` with no target
  element) come back in `produced`, ready for a Bundle.
- **Follows the IG where it says more than the reference engine.** Named groups and nested rules
  after `then` both run; a source's `min..max` is enforced; `evaluate` with several values makes
  one target per value; a group name in several imported maps is an error; `cast` takes `FHIR.`
  and `System.` types and checks the value.
- **Debug trace.** `with_trace(true)` records the trace of the IG's debug page for
  `$transform?debug=true`: `enter` / `step` / `exit` events with their depth, the position in the
  StructureMap (`StructureMap.group[0].rule[2]`) and the variables in scope. `debug_parameters`
  writes the `Parameters` response with `traceStatus` (`complete`, `truncated`, `stopped`), the
  text range of each event (rows and UTF-16 columns, from `fml_model::Locations`), and an
  `outcome` with the failure and where it happened, why events were left out, and values left out
  of the result.
- **Safe to run on maps you did not write.** `Limits` bound the group depth, the rules run, the
  values created and the size of the trace; a flag cancels a run. Failures say what kind they are
  (`ErrorKind`) and where (`position`).
- **Pluggable.** Structure definitions come from [fml-check](https://crates.io/crates/fml-check)'s
  `Definitions`, FHIRPath from an `Expressions` implementation (the `fhirpath` feature uses
  [octofhir-fhirpath](https://crates.io/crates/octofhir-fhirpath)), imported maps from a
  `MapResolver`, ConceptMaps from a `ConceptMapResolver`, and any other ConceptMap through a
  `TranslateService` (`$translate` on a terminology server). ConceptMaps are found by URL and by
  the canonical URLs they replace. `uuid()` and new ids come from an `IdGenerator`
  (`SequentialIds` makes runs repeatable).

```rust
use fml_check::{CoreDefinitions, Release};
use fml_engine::{ConceptMapSet, Engine, FhirPathExpressions, Limits, MapSet};

let definitions = CoreDefinitions::new(Release::R5);
let expressions = FhirPathExpressions::new(Release::R5).await?;
let output = Engine::new(&definitions, expressions)
    .with_maps(&maps)                 // MapSet: StructureMaps for `imports`
    .with_concept_maps(&concept_maps) // ConceptMapSet: for translate()
    .with_limits(Limits::default())   // depth, rules, values, trace size
    .transform(&map, &input)
    .await?;
println!("{}", output.target);        // also: output.produced, output.warnings
```

## FHIR server access

With the `fhirpath` feature, `FhirPathExpressions::with_server_provider` accepts
an `Arc<dyn ServerProvider>` from `octofhir-fhir-model`, also exported as
`fml_engine::ServerProvider`. It connects the existing FHIRPath mechanism:
`resolve()` first looks in the current resource tree, then calls the provider's
`read` for a `Type/id` reference. Resolved resources retain their FHIR types.
Unresolved references return an empty result, as the [specification](https://hl7.org/fhir/R5/fhirpath.html#functions)
requires. This adapter does not add absolute or versioned reference resolution,
or canonical lookup.

Inside a FHIR server, the provider should call internal services with the caller's
permissions, rather than make HTTP requests back to the same server. Reuse the engine:

```rust
let shared_expressions = FhirPathExpressions::new(Release::R5).await?;
let expressions = shared_expressions.clone().with_server_provider(request_provider);
let output = Engine::new(&definitions, expressions)
    .transform(&map, &input)
    .await?;
```

Clones share the engine, schemas and expression cache, but configuring one clone's
provider does not change another clone's provider or evaluation context. Do not put
a caller-specific provider in the shared instance. This follows the [FML execution model](https://hl7.org/fhir/R5/mapping-language.html):
the host application supplies access to external data.

Standalone applications can use `octofhir_fhir_model::HttpServerProvider`, enabling
`http-client` **in the application**. The `fml-engine` library does not enable HTTP.
External requests are disabled without a provider.

The provider is also available through `%server`, including write operations. Use
a restricted provider for untrusted maps; engine limits do not replace authorization.
The current FHIRPath implementation turns `read` errors into empty results. If the
host must fail the operation on such an error, retain it in the provider and check
it after the map finishes.

Connecting a server does not restore the root context for `contained` resources:
`evaluate(reference, resolve())` still cannot see the original resource. For local
references, evaluate the path from the resource, for example
`evaluate(src, bodyStructure.reference.resolve())`.

Part of [fml-rs](https://github.com/octofhir/fml-rs). Licensed under MIT or Apache-2.0.
