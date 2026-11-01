# fml-syntax

Parser and formatter for the [FHIR Mapping Language](https://build.fhir.org/ig/HL7/mapping-language-ig/)
(FML), in Rust.

- **Lossless.** The syntax tree keeps every byte of the input, comments and whitespace included:
  `parse(text).syntax().to_string() == text` for any input.
- **Error-tolerant.** A mistake becomes a diagnostic and an error node; the rest of the file still
  parses, so one typo does not hide the next one. Positions are given in bytes, lines and UTF-16
  columns.
- **A formatter that keeps comments.** One statement per line, consistent spacing, comments where
  they were. It re-parses its own output and refuses to return text that would change the map.
- **The real-world grammar.** The Mapping Language IG grammar, plus what maps written for FHIR
  R3–R5 use and the HL7 reference compiler accepts: `map "url" = "name"`, unparenthesised `where`,
  embedded `conceptmap` with `unmapped`, `<<type+>>` groups.

All 1,659 public maps we test against parse without errors, and formatting each of them twice gives
the same result as formatting it once.

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
let formatted = fml_syntax::format(source, &fml_syntax::FormatOptions::default())?;
```

Part of [fml-rs](https://github.com/octofhir/fml-rs). Licensed under MIT or Apache-2.0.
