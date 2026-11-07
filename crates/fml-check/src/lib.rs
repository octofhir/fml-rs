//! Type checker for the FHIR Mapping Language.
//!
//! Follows every rule with the types of its variables and reports what cannot work: elements that
//! do not exist on the type they are read from or written to, a type annotation that is not one of
//! the element's types, unknown variables, writes into source variables, unknown or abstract types
//! in `create`, transforms with the wrong number of arguments, and calls to groups that are not
//! declared or take a different number of arguments. It also reports what the Mapping Language IG
//! requires of a map: a `url` and a `name`, groups that extend another with its parameters,
//! default groups with a typed source and target, and rule names used once.
//!
//! Groups without parameter types, such as the per-backbone groups of the cross-version maps, are
//! checked with the types they are called with.
//!
//! ```
//! # #[cfg(feature = "fhirschema")] {
//! use fml_check::{CoreDefinitions, Release, check};
//!
//! let parse = fml_syntax::parse(r#"
//! /// url = 'http://example.org/StructureMap/Copy'
//! /// name = 'Copy'
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

pub mod analysis;
mod check;
#[cfg(feature = "fhirschema")]
mod core;
pub mod definitions;
#[cfg(feature = "fhirschema")]
mod logical;
#[cfg(feature = "fhirschema")]
mod package;
pub mod types;

pub use analysis::{Analysis, Binding, BindingKind, Reference, Referent};
pub use check::{analyze, check};
#[cfg(feature = "fhirschema")]
pub use core::{CoreDefinitions, Release};
pub use definitions::{Definitions, Element, Kind, NoDefinitions, Structure, TemplateId};
#[cfg(feature = "fhirschema")]
pub use package::PackageDefinitions;
pub use types::{ElementRef, Lookup, Type, Types};
