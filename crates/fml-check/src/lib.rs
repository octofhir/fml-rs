//! Type checker for the FHIR Mapping Language.
//!
//! Follows every rule with the types of its variables and reports what cannot work: elements that
//! do not exist on the type they are read from or written to, a type annotation that is not one of
//! the element's types, unknown variables, writes into source variables, unknown or abstract types
//! in `create`, transforms with the wrong number of arguments, and calls to groups that are not
//! declared or take a different number of arguments.
//!
//! Groups without parameter types, such as the per-backbone groups of the cross-version maps, are
//! checked with the types they are called with.
//!
//! ```
//! # #[cfg(feature = "fhirschema")] {
//! use fml_check::{CoreDefinitions, Release, check};
//!
//! let parse = fml_syntax::parse(r#"
//! uses "http://hl7.org/fhir/StructureDefinition/Patient" as source
//! uses "http://hl7.org/fhir/StructureDefinition/Patient" as target
//!
//! group Copy(source src : Patient, target tgt : Patient) {
//!   src.nmae -> tgt.name;
//! }
//! "#);
//! let diagnostics = check(&parse.tree(), &CoreDefinitions::new(Release::R5));
//! assert_eq!(diagnostics[0].message, "`Patient` has no element `nmae`");
//! # }
//! ```

mod check;
#[cfg(feature = "fhirschema")]
mod core;
pub mod definitions;

pub use check::check;
#[cfg(feature = "fhirschema")]
pub use core::{CoreDefinitions, Release};
pub use definitions::{Definitions, Element, Kind, NoDefinitions, Structure};
