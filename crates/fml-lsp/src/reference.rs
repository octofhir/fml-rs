//! What the editor tells about the language itself: the transforms, with their signatures and
//! descriptions from the Mapping Language IG (transforms.html), the keywords, and the FHIRPath
//! functions offered after a `.` in an expression.

pub(crate) struct Transform {
    pub name: &'static str,
    /// The parameters, as the IG names them; `[name]` is optional, `name...` repeats.
    pub parameters: &'static [&'static str],
    pub result: &'static str,
    pub description: &'static str,
}

impl Transform {
    pub fn signature(&self) -> String {
        format!(
            "{}({}) : {}",
            self.name,
            self.parameters.join(", "),
            self.result
        )
    }

    /// A snippet that calls the transform with placeholders for the required parameters.
    pub fn snippet(&self) -> String {
        let required: Vec<String> = self
            .parameters
            .iter()
            .filter(|p| !p.starts_with('['))
            .enumerate()
            .map(|(i, p)| format!("${{{}:{}}}", i + 1, p.trim_end_matches("...")))
            .collect();
        format!("{}({})", self.name, required.join(", "))
    }
}

pub(crate) const TRANSFORMS_PAGE: &str =
    "https://build.fhir.org/ig/HL7/mapping-language-ig/transforms.html";

pub(crate) const TRANSFORMS: &[Transform] = &[
    Transform {
        name: "create",
        parameters: &["type"],
        result: "any",
        description: "Creates a new instance of the specified type using the standard API. Where structure definitions have been provided, the `type` parameter must be a string which is a known type of a root element.",
    },
    Transform {
        name: "copy",
        parameters: &["source"],
        result: "any",
        description: "Simply copies the source to the target as-is. This is only allowed when the types in source and target match - typically for primitive types.",
    },
    Transform {
        name: "truncate",
        parameters: &["source", "length"],
        result: "String",
        description: "Returns the first `length` characters of the `source` string. The `source` must be some string type that has some meaningful length property.",
    },
    Transform {
        name: "escape",
        parameters: &["source", "format1", "format2"],
        result: "String",
        description: "Changes the internal escaping of a string element from `format1` to `format2`.",
    },
    Transform {
        name: "cast",
        parameters: &["source", "[type]"],
        result: "any",
        description: "Casts `source` from one type to another. The target `type` can be left as implicit if there is one and only one target type known.",
    },
    Transform {
        name: "append",
        parameters: &["source..."],
        result: "String",
        description: "Concatenates all provided source values together. Each source parameter is an element or string.",
    },
    Transform {
        name: "translate",
        parameters: &["source", "map_uri", "output"],
        result: "any",
        description: "Uses the translate operation to translate coded values. `output` is `code`, `system`, `display`, `Coding` or `CodeableConcept`.",
    },
    Transform {
        name: "reference",
        parameters: &["source"],
        result: "String",
        description: "Returns a string that references the provided tree properly. This produces a reference string appropriate for the target context.",
    },
    Transform {
        name: "dateOp",
        parameters: &["parameters..."],
        result: "any",
        description: "Performs a date operation. A StructureMap transform code; the Mapping Language IG does not describe its parameters.",
    },
    Transform {
        name: "uuid",
        parameters: &[],
        result: "String",
        description: "Generates a random UUID (in lowercase). This function takes no parameters.",
    },
    Transform {
        name: "pointer",
        parameters: &["resource"],
        result: "String",
        description: "Returns the appropriate string to put in a Reference that refers to the resource provided as a parameter.",
    },
    Transform {
        name: "evaluate",
        parameters: &["context", "expression"],
        result: "any",
        description: "Executes the supplied FHIRPath expression in the context of the first parameter and uses the value it returns.",
    },
    Transform {
        name: "cc",
        parameters: &["system", "code", "[display]"],
        result: "CodeableConcept",
        description: "Creates a CodeableConcept from the parameters provided. Can also be called with a single `text` parameter to create a CodeableConcept with only a text value.",
    },
    Transform {
        name: "c",
        parameters: &["system", "code", "[display]"],
        result: "Coding",
        description: "Creates a Coding from the parameters provided.",
    },
    Transform {
        name: "qty",
        parameters: &["value", "unit", "[system]", "[code]"],
        result: "Quantity",
        description: "Creates a Quantity from the parameters provided. Can also be called with a single `text` parameter, the natural representation such as `[comparator]value[space]unit`.",
    },
    Transform {
        name: "id",
        parameters: &["system", "value", "[type]"],
        result: "Identifier",
        description: "Creates an Identifier from the parameters provided. The optional `type` is a code from the identifier type value set.",
    },
    Transform {
        name: "cp",
        parameters: &["[system]", "value"],
        result: "ContactPoint",
        description: "Creates a ContactPoint from the parameters provided. If no `system` is provided, the system should be inferred from the content of the value.",
    },
    Transform {
        name: "toDateTime",
        parameters: &["source", "[format]"],
        result: "DateTime",
        description: "Parses a string into a DateTime value. Without `format`, the FHIR dateTime format is assumed.",
    },
    Transform {
        name: "unixToDateTime",
        parameters: &["source", "[timezone]"],
        result: "DateTime",
        description: "Converts a Unix timestamp (ticks since epoch) into a DateTime value in the given timezone, or UTC.",
    },
    Transform {
        name: "toDate",
        parameters: &["source", "[format]"],
        result: "Date",
        description: "Parses a string into a Date value. Without `format`, the FHIR date format is assumed.",
    },
    Transform {
        name: "unixToDate",
        parameters: &["source", "[timezone]"],
        result: "Date",
        description: "Converts a Unix timestamp (ticks since epoch) into a Date value in the given timezone, or UTC.",
    },
    Transform {
        name: "toTime",
        parameters: &["source", "[format]"],
        result: "Time",
        description: "Parses a string into a Time value. Without `format`, the FHIR time format is assumed.",
    },
    Transform {
        name: "unixToTime",
        parameters: &["source", "[timezone]"],
        result: "Time",
        description: "Converts a Unix timestamp (ticks since epoch) into a Time value in the given timezone, or UTC.",
    },
];

pub(crate) fn transform(name: &str) -> Option<&'static Transform> {
    TRANSFORMS.iter().find(|t| t.name == name)
}

/// Words that start a declaration or a clause.
pub(crate) const KEYWORDS: &[&str] = &[
    "map",
    "uses",
    "alias",
    "as",
    "imports",
    "let",
    "group",
    "extends",
    "source",
    "target",
    "queried",
    "produced",
    "where",
    "check",
    "log",
    "then",
    "default",
    "first",
    "not_first",
    "last",
    "not_last",
    "only_one",
    "single",
    "share",
    "collate",
    "conceptmap",
    "prefix",
];

/// FHIRPath functions offered after a `.` in an expression, with their signatures.
pub(crate) const FHIRPATH_FUNCTIONS: &[(&str, &str)] = &[
    ("where", "where(criteria : expression) : collection"),
    ("select", "select(projection : expression) : collection"),
    ("exists", "exists([criteria : expression]) : Boolean"),
    ("empty", "empty() : Boolean"),
    ("all", "all(criteria : expression) : Boolean"),
    ("count", "count() : Integer"),
    ("first", "first() : collection"),
    ("last", "last() : collection"),
    ("tail", "tail() : collection"),
    ("single", "single() : collection"),
    ("distinct", "distinct() : collection"),
    ("ofType", "ofType(type : type specifier) : collection"),
    ("as", "as(type : type specifier) : collection"),
    ("is", "is(type : type specifier) : Boolean"),
    (
        "iif",
        "iif(criterion : expression, true-result : collection [, otherwise-result : collection]) : collection",
    ),
    ("not", "not() : Boolean"),
    ("hasValue", "hasValue() : Boolean"),
    ("toString", "toString() : String"),
    ("toInteger", "toInteger() : Integer"),
    ("startsWith", "startsWith(prefix : String) : Boolean"),
    ("endsWith", "endsWith(suffix : String) : Boolean"),
    ("contains", "contains(substring : String) : Boolean"),
    ("matches", "matches(regex : String) : Boolean"),
    (
        "replace",
        "replace(pattern : String, substitution : String) : String",
    ),
    (
        "substring",
        "substring(start : Integer [, length : Integer]) : String",
    ),
    ("length", "length() : Integer"),
    ("lower", "lower() : String"),
    ("upper", "upper() : String"),
    ("trim", "trim() : String"),
    ("join", "join([separator : String]) : String"),
    ("resolve", "resolve() : collection"),
    ("extension", "extension(url : String) : collection"),
    ("memberOf", "memberOf(valueset : String) : Boolean"),
];
