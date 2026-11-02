//! StructureMap for the FHIR Mapping Language: compile FML to a StructureMap resource.
//!
//! ```
//! let compiled = fml_model::compile(r#"
//! /// url = 'http://example.org/StructureMap/Copy'
//! /// name = 'Copy'
//! uses "http://hl7.org/fhir/StructureDefinition/Patient" as source
//! uses "http://hl7.org/fhir/StructureDefinition/Patient" as target
//!
//! group Copy(source src : Patient, target tgt : Patient) {
//!   src.name -> tgt.name;
//! }
//! "#);
//! assert!(!compiled.has_errors());
//! let rule = &compiled.map.group[0].rule[0];
//! assert_eq!(rule.name.as_deref(), Some("name"));
//! assert_eq!(rule.target[0].transform.as_deref(), Some("create"));
//! ```

mod canonical;
mod compile;
pub mod flavor;
mod metadata;
pub mod model;
mod print;

pub use compile::{
    AUTO_VARIABLE, BATCH_UNNAMED_PREFIX, Compiled, Locations, REFERENCES_CONTAINED_EXTENSION,
    SOURCE_ELEMENT_EXTENSION, Span, TARGET_ELEMENT_EXTENSION, compile, compile_tree, make_id,
    source_path,
};
pub use flavor::{R4Map, from_json, to_r4};
pub use model::StructureMap;
pub use print::print;
