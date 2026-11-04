//! XML for FHIR logical models.
//!
//! FHIR maps run on JSON. Logical models whose instances are XML — CDA is the best known — say in
//! their StructureDefinitions how the XML looks: namespaces and names, which elements are
//! attributes or text, where `xsi:type` gives a type, which elements group others that appear
//! directly in the parent. This crate reads such XML into the JSON of the model, so a map can
//! run over it, and writes JSON of the model back as XML. It knows nothing about any particular
//! model; everything comes from the definitions.
//!
//! ```text
//! <value xsi:type="PQ" value="120" unit="mm[Hg]"/>   ⇄   "value": { "@type": "PQ", "value": 120, "unit": "mm[Hg]" }
//! ```
//!
//! The JSON form:
//! - property names are the model's element names (`sdtcCategory`, not `sdtc:category`);
//! - attributes are primitives: `boolean` and numbers as JSON booleans and numbers, a repeating
//!   attribute (`use="H WP"`) as an array;
//! - text content is the `xmlText` element, markup (narrative) a string of XML;
//! - a value whose type `xsi:type` gives, or whose type differs from the element's, names it in
//!   `"@type"` (the model's type name, `IVL_TS`);
//! - a choice group is an array of one-property objects, in document order: an address is
//!   `"item": [{ "streetAddressLine": … }, { "city": … }]`;
//! - dates and times declared with a date format are read into the FHIR lexical form at the
//!   precision written (`20200115093000-0500` → `2020-01-15T09:30:00-05:00`) and written back the
//!   same.
//!
//! Reading also records where each element was (line and column) by its path in the JSON, the
//! path the engine's debug trace uses, and reports what it could not read into the model instead
//! of dropping it silently. [`compare`] tells whether two documents say the same.

mod compare;
mod datetime;
mod model;
mod read;
mod write;

pub use compare::{Difference, compare, compare_fragments};
pub use datetime::{from_compact, to_compact};
pub use model::{Element, Model, Representation, Root, TypeDef};
pub use read::{Document, ReadOptions, read, read_with};
pub use write::{Written, write};

/// The property that names the type of a value in the JSON form.
pub const TYPE_KEY: &str = "@type";

/// The XML Schema instance namespace (`xsi:type`).
pub const XSI_NS: &str = "http://www.w3.org/2001/XMLSchema-instance";

/// The `xml:` namespace.
pub const XML_NS: &str = "http://www.w3.org/XML/1998/namespace";

/// A line and column in the XML, both from 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Position {
    pub line: u32,
    pub column: u32,
}

impl std::fmt::Display for Position {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.line, self.column)
    }
}

/// XML that cannot be read or written at all.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    pub message: String,
    pub position: Option<Position>,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.position {
            Some(position) => write!(f, "{position}: {}", self.message),
            None => f.write_str(&self.message),
        }
    }
}

impl std::error::Error for Error {}

/// What kind of content could not be carried over.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IssueKind {
    /// An element the model does not have.
    UnknownElement,
    /// An attribute the model does not have.
    UnknownAttribute,
    /// A type (`xsi:type`, `"@type"`) the model does not have, or one that does not fit.
    UnknownType,
    /// A polymorphic element without a type.
    MissingType,
    /// A value that does not fit its type, kept as text.
    InvalidValue,
    /// Text where the model has none.
    UnexpectedText,
    /// More values than the element allows.
    TooMany,
}

/// Content left out or changed while reading or writing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Issue {
    pub kind: IssueKind,
    /// The JSON path of the element it concerns.
    pub path: String,
    /// Where in the XML, when reading.
    pub position: Option<Position>,
    pub message: String,
}

impl std::fmt::Display for Issue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.position {
            Some(position) => write!(f, "{position} {}: {}", self.path, self.message),
            None => write!(f, "{}: {}", self.path, self.message),
        }
    }
}
