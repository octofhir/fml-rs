# fml-check

Type checker for the [FHIR Mapping Language](https://build.fhir.org/ig/HL7/mapping-language-ig/):
finds what a map gets wrong before it runs.

It follows every rule with the types of its variables and reports:

- elements that do not exist on the type they are read from or written to, including inherited
  ones, backbone elements and content references (`Questionnaire.item.item`);
- type annotations that are not a type of the element (`src.deceased : string`);
- unknown variables and writes into source variables;
- unknown or abstract types in `create('Type')`, and transforms with the wrong number of arguments;
- calls to groups that are not declared, or with the wrong number of arguments;
- element paths inside FHIRPath (`where`, `check`, `(…)`), as warnings, since a wrong path there
  gives an empty result instead of an error.

Groups without parameter types are checked with the types they are called with, which is how the
cross-version maps are written. Those maps (R2–R5) check clean; on other public maps it found a
typo in the FHIR test cases, a parameter a group uses without declaring it, and a condition that is
never true because it reads `system` and `code` from a CodeableConcept instead of its codings.

Definitions come through a small `Definitions` trait. With the `fhirschema` feature,
`CoreDefinitions` provides FHIR R4, R4B, R5 and R6 (6.0.0-ballot5), including the
version-specific URLs the cross-version maps use (`http://hl7.org/fhir/4.0/StructureDefinition/…`).
`PackageDefinitions` puts the StructureDefinitions of FHIR packages (profiles, extensions, logical
models) in front of them.

```rust
use fml_check::{CoreDefinitions, Release, check};

let parse = fml_syntax::parse(source);
for diagnostic in check(&parse.tree(), &CoreDefinitions::new(Release::R5)) {
    eprintln!("{:?}: {}", diagnostic.severity, diagnostic.message);
}
```

Part of [fml-rs](https://github.com/octofhir/fml-rs). Licensed under MIT or Apache-2.0.
