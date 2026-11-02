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
  Not yet: `dateOp`, which the IG lists without defining.
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

Part of [fml-rs](https://github.com/octofhir/fml-rs). Licensed under MIT or Apache-2.0.
