# fml-xml

XML for FHIR logical models: read XML into the JSON a FHIR Mapping Language map runs on, and write
JSON of the model back as XML — as the model's StructureDefinitions describe it. Built for the
HL7 CDA model (`hl7.cda.uv.core`), and knows nothing about CDA: everything comes from the
definitions.

- **What the definitions say.** `xml-namespace` and `xml-name`, `representation` (`xmlAttr`,
  `xmlText`, `typeAttr`, `cdaText`), `xml-choice-group`, `elementdefinition-defaulttype` and
  `elementdefinition-date-format` — the extensions of the FHIR tools IG.
- **Types.** `xsi:type` becomes `"@type"` in JSON and back; the default type goes without it, a
  subtype of the element's type is named.
- **Dates.** Timestamps in the model's own form (`20200115093000-0500`) read into the FHIR form at
  the precision written (`2020-01-15T09:30:00-05:00`) and write back the same.
- **Narrative.** `Section.text` stays markup, a string of XML.
- **Positions.** Every element's line and column, by the path the engine's debug trace uses
  (`ClinicalDocument.component.structuredBody.component[2].section`).
- **Nothing silently dropped.** Elements and attributes outside the model, unknown types, values
  that do not fit come back as issues.
- **Fragments.** A section or entry without its document: say which namespaces are in scope and
  which element of which type the root is (`StructuredBody.component`).
- **Comparison.** `compare` tells whether two documents say the same (namespaces, prefixes,
  attribute order, comments, whitespace and the order of differently named siblings aside).

```rust
let mut model = fml_xml::Model::new();
model.add_folder(Path::new("hl7.cda.uv.core/package"))?;
let document = fml_xml::read(xml, &model, None)?;          // document.json, .positions, .issues
let written = fml_xml::write(&document.json, &model, &document.root_type)?;
assert!(fml_xml::compare(xml, &written.xml)?.is_empty());
```

On the HL7 C-CDA examples (446 files) and CDA examples, every document that is well-formed reads,
writes and reads again to the same JSON, with no difference the reader did not report.
`FML_XML_CORPUS=<folder> cargo test -p fml-xml` runs that check over any folder of XML.

The tests use the StructureDefinitions of `hl7.cda.uv.core` 2.0.3-sd (CC0-1.0, HL7 International)
trimmed to what the crate reads (`tests/data`).
