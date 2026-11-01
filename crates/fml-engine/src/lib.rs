//! StructureMap engine for the FHIR Mapping Language.
//!
//! Runs a [`StructureMap`](fml_model::StructureMap) over FHIR JSON, as the HL7 reference engine
//! does. Structure definitions come from [`fml_check::Definitions`]; FHIRPath from an
//! [`Expressions`] implementation, such as the one the `fhirpath` feature provides; imported maps
//! from a [`MapResolver`].

mod datetime;
mod engine;
mod expressions;
#[cfg(feature = "fhirpath")]
mod fhirpath;
mod terminology;
mod trace;
mod transforms;
mod tree;

pub use engine::{Engine, Error, MapResolver, MapSet, NoMaps, Transformed};
pub use expressions::{Expressions, Item, NoExpressions, bind_variables};
#[cfg(feature = "fhirpath")]
pub use fhirpath::FhirPathExpressions;
pub use terminology::{ConceptMapResolver, ConceptMapSet, NoConceptMaps, Translation, translate};
pub use trace::{Phase, Trace, TraceEvent, TraceStatus, TraceVariable, debug_parameters};
