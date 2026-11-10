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
mod ids;
mod terminology;
mod timezone;
mod trace;
mod transforms;
mod tree;
mod windows_zones;

pub use engine::{Engine, Error, ErrorKind, Limits, MapResolver, MapSet, NoMaps, Transformed};
pub use expressions::{Expressions, Item, NoExpressions, bind_variables};
#[cfg(feature = "fhirpath")]
pub use fhirpath::FhirPathExpressions;
pub use ids::{IdGenerator, RandomIds, SequentialIds};
#[cfg(feature = "fhirpath")]
pub use octofhir_fhir_model::ServerProvider;
pub use terminology::{
    ConceptMapResolver, ConceptMapSet, NoConceptMaps, TranslateFuture, TranslateService,
    Translation, translate,
};
pub use trace::{
    DebugReport, Phase, SpanLookup, Trace, TraceEvent, TraceStatus, TraceVariable,
    debug_parameters, no_spans,
};
pub use tree::TYPE_KEY;
