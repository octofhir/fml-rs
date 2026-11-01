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

mod compile;
pub mod model;
mod print;

pub use compile::{
    AUTO_VARIABLE, BATCH_UNNAMED_PREFIX, Compiled, SOURCE_ELEMENT_EXTENSION,
    TARGET_ELEMENT_EXTENSION, compile, compile_tree, make_id, source_path,
};
pub use model::StructureMap;
pub use print::print;
