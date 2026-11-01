# fml-model

The StructureMap resource for the [FHIR Mapping Language](https://build.fhir.org/ig/HL7/mapping-language-ig/):
compile FML to a StructureMap, and print a StructureMap back as FML.

- **Matches the HL7 reference compiler.** The StructureMap JSON is the one `StructureMapUtilities`
  in org.hl7.fhir.core builds, byte for byte, on all 1,659 maps we test against: generated rule
  names, the variables of the simple copy form, batch identity rules, contained ConceptMaps,
  documentation from comments, FHIRPath in the reference parser's canonical form. Two documented
  differences keep what the map says: whitespace-only strings, which the reference JSON writer
  drops, and calendar durations (`5 days`), which it rewrites as `5 'days'`.
- **Prints FML back.** The printed map compiles to the same StructureMap, for every map in the
  corpus; it also reads maps built by older IG Publisher versions.
- **R4 and R5.** `to_r4` writes the R4 shape (`dependent.variable`, `defaultValue[x]`,
  `contextType`, ConceptMap `equivalence`) and reports what R4 cannot hold; `from_json` reads both.
- Elements the model does not name are kept, so reading and writing a StructureMap loses nothing.

```rust
let compiled = fml_model::compile(source);         // StructureMap + diagnostics
let json = serde_json::to_string_pretty(&compiled.map)?;
let r4 = fml_model::to_r4(&compiled.map);          // R4 JSON + what it could not hold
let fml = fml_model::print(&compiled.map);         // back to FML
```

Part of [fml-rs](https://github.com/octofhir/fml-rs). Licensed under MIT or Apache-2.0.
