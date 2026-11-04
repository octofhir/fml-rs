//! The XML layer over the HL7 CDA logical model (hl7.cda.uv.core 2.0.3-sd, CC0).

use std::path::Path;
use std::sync::OnceLock;

use fml_xml::{
    Document, IssueKind, Model, ReadOptions, compare, compare_fragments, read, read_with, write,
};
use serde_json::{Value, json};

const CDA: &str = "http://hl7.org/cda/stds/core/StructureDefinition/";

fn model() -> &'static Model {
    static MODEL: OnceLock<Model> = OnceLock::new();
    MODEL.get_or_init(|| {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/cda-core.json");
        let bundle: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        let mut model = Model::new();
        for resource in bundle["resources"].as_array().unwrap() {
            model.add(resource);
        }
        model
    })
}

/// Wraps a fragment in an observation, the way the tests read single elements.
fn observation(content: &str) -> String {
    format!(
        r#"<observation xmlns="urn:hl7-org:v3" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xmlns:sdtc="urn:hl7-org:sdtc" classCode="OBS" moodCode="EVN">{content}</observation>"#
    )
}

/// Reads, writes and compares: the written XML says the same as the input, and reading it again
/// gives the same JSON.
fn round_trip(xml: &str, root: Option<&str>) -> Value {
    let document = read(xml, model(), root).unwrap();
    assert!(
        document.issues.is_empty(),
        "issues reading: {:#?}",
        document.issues
    );
    let written = write(&document.json, model(), &document.root_type).unwrap();
    assert!(
        written.issues.is_empty(),
        "issues writing: {:#?}",
        written.issues
    );
    let differences = compare(xml, &written.xml).unwrap();
    assert!(
        differences.is_empty(),
        "differences:\n{}\n\nwritten:\n{}",
        differences
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n"),
        written.xml
    );
    let again = read(&written.xml, model(), root).unwrap();
    assert_eq!(again.json, document.json);
    document.json
}

#[test]
fn the_model_loads() {
    // 124 classes and data types; the 15 primitive profiles are constraints, not types.
    assert_eq!(model().len(), 124);
    let observation = model().get(&format!("{CDA}Observation")).unwrap();
    assert_eq!(observation.xml_name.as_deref(), Some("observation"));
    let value = observation.element("value").unwrap();
    assert!(value.type_attribute);
    assert!(value.many);
    assert!(
        model()
            .by_type_name(Some("urn:hl7-org:v3"), "IVL_TS")
            .is_some()
    );
}

#[test]
fn the_specification_example_round_trips() {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/clinicaldocument-example.xml");
    let xml = std::fs::read_to_string(path).unwrap();
    let json = round_trip(&xml, None);
    assert_eq!(json["code"]["code"], "11488-4");
    assert_eq!(
        json["title"]["xmlText"],
        "Good Health Clinic Consultation Note"
    );
    assert_eq!(json["effectiveTime"]["value"], "2000-04-07");
    // A section's narrative is kept as markup.
    let section = &json["component"]["structuredBody"]["component"][0]["section"];
    assert!(section["text"].as_str().unwrap().starts_with("<text"));
}

#[test]
fn positions_follow_the_json_paths() {
    let xml = observation(
        "\n  <code code=\"8480-6\"/>\n  <value xsi:type=\"PQ\" value=\"120\" unit=\"mm[Hg]\"/>",
    );
    let document = read(&xml, model(), None).unwrap();
    assert_eq!(document.root_type, format!("{CDA}Observation"));
    let code = document.position("Observation.code").unwrap();
    assert_eq!((code.line, code.column), (2, 3));
    let value = document.position("Observation.value[0]").unwrap();
    assert_eq!(value.line, 3);
}

#[test]
fn xsi_type_names_the_type() {
    let json = round_trip(
        &observation(
            r#"<value xsi:type="PQ" value="120.5" unit="mm[Hg]"/><value xsi:type="CD" code="1" codeSystem="2.16.840.1.113883.6.96"/>"#,
        ),
        None,
    );
    assert_eq!(
        json["value"],
        json!([
            { "@type": "PQ", "value": 120.5, "unit": "mm[Hg]" },
            { "@type": "CD", "code": "1", "codeSystem": "2.16.840.1.113883.6.96" }
        ])
    );
}

#[test]
fn a_subtype_in_a_single_typed_element_is_named() {
    // Observation.effectiveTime is IVL_TS; xsi:type stating the same type changes nothing.
    let json = round_trip(
        &observation(
            r#"<code xsi:type="CE" code="1"/><effectiveTime><low value="20200115093000-0500"/></effectiveTime>"#,
        ),
        None,
    );
    assert_eq!(json["code"]["@type"], "CE");
    assert_eq!(
        json["effectiveTime"]["low"]["value"],
        "2020-01-15T09:30:00-05:00"
    );
    assert!(json["effectiveTime"].get("@type").is_none());
}

#[test]
fn the_default_type_applies_without_xsi_type() {
    let xml = r#"<substanceAdministration xmlns="urn:hl7-org:v3" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" classCode="SBADM" moodCode="EVN">
      <effectiveTime value="20200115"/>
      <effectiveTime xsi:type="PIVL_TS" institutionSpecified="true" operator="A"><period value="12" unit="h"/></effectiveTime>
    </substanceAdministration>"#;
    let document = read(xml, model(), None).unwrap();
    assert!(document.issues.is_empty(), "{:#?}", document.issues);
    let times = &document.json["effectiveTime"];
    assert_eq!(times[0]["@type"], "SXCM_TS");
    assert_eq!(times[0]["value"], "2020-01-15");
    assert_eq!(times[1]["@type"], "PIVL_TS");
    assert_eq!(times[1]["institutionSpecified"], true);
    assert_eq!(times[1]["period"]["value"], 12);
    // Written back, the default type goes without xsi:type, any other type with it.
    let written = write(&document.json, model(), &document.root_type).unwrap();
    assert!(
        written.xml.contains(r#"<effectiveTime value="20200115"/>"#),
        "{}",
        written.xml
    );
    assert!(
        written.xml.contains(r#"<effectiveTime xsi:type="PIVL_TS""#),
        "{}",
        written.xml
    );
}

#[test]
fn address_parts_keep_their_order() {
    let xml = r#"<patientRole xmlns="urn:hl7-org:v3" classCode="PAT">
      <id root="1.2.3"/>
      <addr use="HP WP"><streetAddressLine>1 Main St</streetAddressLine><city>Springfield</city>, <state>IL</state><postalCode>62701</postalCode></addr>
      <telecom use="HP" value="tel:+1(555)555-1212"/>
    </patientRole>"#;
    let json = round_trip(xml, None);
    assert_eq!(
        json["addr"][0],
        json!({
            "use": ["HP", "WP"],
            "item": [
                { "streetAddressLine": { "xmlText": "1 Main St" } },
                { "city": { "xmlText": "Springfield" } },
                { "xmlText": ", " },
                { "state": { "xmlText": "IL" } },
                { "postalCode": { "xmlText": "62701" } }
            ]
        })
    );
}

#[test]
fn names_and_sdtc_extensions() {
    let xml = r#"<patient xmlns="urn:hl7-org:v3" xmlns:sdtc="urn:hl7-org:sdtc">
      <name use="L"><given>Eve</given><given qualifier="CL">Evie</given><family>Everywoman</family></name>
      <administrativeGenderCode code="F" codeSystem="2.16.840.1.113883.5.1"/>
      <birthTime value="19750501"/>
      <sdtc:deceasedInd value="false"/>
      <raceCode code="2106-3" codeSystem="2.16.840.1.113883.6.238"/>
      <sdtc:raceCode code="2108-9" codeSystem="2.16.840.1.113883.6.238"/>
    </patient>"#;
    let json = round_trip(xml, None);
    assert_eq!(
        json["name"][0]["item"][1]["given"]["qualifier"],
        json!(["CL"])
    );
    assert_eq!(json["birthTime"]["value"], "1975-05-01");
    assert_eq!(json["sdtcDeceasedInd"]["value"], false);
    assert_eq!(json["sdtcRaceCode"][0]["code"], "2108-9");
}

#[test]
fn narrative_is_markup() {
    let xml = r##"<section xmlns="urn:hl7-org:v3">
      <title>Allergies</title>
      <text><table border="1"><tbody><tr><td ID="a1">Penicillin &amp; <content styleCode="Bold">hives</content></td></tr></tbody></table></text>
      <entry><act classCode="ACT" moodCode="EVN"><code nullFlavor="NA"/><text><reference value="#a1"/></text></act></entry>
    </section>"##;
    let json = round_trip(xml, None);
    assert_eq!(
        json["text"],
        r#"<text><table border="1"><tbody><tr><td ID="a1">Penicillin &amp; <content styleCode="Bold">hives</content></td></tr></tbody></table></text>"#
    );
    assert_eq!(json["entry"][0]["act"]["text"]["reference"]["value"], "#a1");
}

#[test]
fn what_the_model_lacks_is_reported() {
    let xml = observation(r#"<code code="1" bogus="x"/><unknownThing/><value xsi:type="NOPE"/>"#);
    let document = read(&xml, model(), None).unwrap();
    let kinds: Vec<IssueKind> = document.issues.iter().map(|i| i.kind).collect();
    assert!(kinds.contains(&IssueKind::UnknownAttribute), "{kinds:?}");
    assert!(kinds.contains(&IssueKind::UnknownElement), "{kinds:?}");
    assert!(kinds.contains(&IssueKind::UnknownType), "{kinds:?}");
    let unknown = document
        .issues
        .iter()
        .find(|i| i.kind == IssueKind::UnknownElement)
        .unwrap();
    assert_eq!(unknown.path, "Observation");
    assert!(unknown.position.is_some());
}

#[test]
fn writing_reports_what_the_model_lacks() {
    let json = json!({ "classCode": "OBS", "code": { "code": "1", "nope": true }, "extra": 1 });
    let written = write(&json, model(), "Observation").unwrap();
    assert_eq!(written.issues.len(), 2, "{:#?}", written.issues);
    assert!(written.xml.contains(r#"<code code="1"/>"#));
}

#[test]
fn compare_finds_differences() {
    let a = observation(r#"<code code="1"/><value xsi:type="PQ" value="1"/>"#);
    let b = r#"<x:observation xmlns:x="urn:hl7-org:v3" xmlns:i="http://www.w3.org/2001/XMLSchema-instance" moodCode="EVN" classCode="OBS">
        <x:code code="1"/>
        <x:value i:type="x:PQ" value="1"/>
      </x:observation>"#;
    assert!(compare(&a, b).unwrap().is_empty());
    let c = observation(r#"<code code="2"/><value xsi:type="PQ" value="1"/>"#);
    let differences = compare(&a, &c).unwrap();
    assert_eq!(differences.len(), 1);
    assert_eq!(differences[0].path, "/observation/code[1]");
}

/// Every XML file under `FML_XML_CORPUS` (the HL7 C-CDA and CDA example repositories, say) reads,
/// writes and compares the same. Skipped when the variable is not set.
#[test]
fn corpus_round_trips() {
    let Ok(root) = std::env::var("FML_XML_CORPUS") else {
        return;
    };
    let mut files = Vec::new();
    collect(Path::new(&root), &mut files);
    files.sort();
    assert!(!files.is_empty(), "no XML files under {root}");
    let (mut total, mut clean, mut failed, mut malformed) = (0, 0, Vec::new(), Vec::new());
    let mut issue_counts: std::collections::BTreeMap<String, (usize, String)> = Default::default();
    for file in &files {
        let Ok(xml) = std::fs::read_to_string(file) else {
            continue;
        };
        total += 1;
        let document = match read_fragment(&xml) {
            Ok(document) => document,
            Err(error) if error.message.starts_with("not well-formed") => {
                malformed.push(format!("{}: {error}", file.display()));
                continue;
            }
            // Not a document of the model (IG configuration, HTML includes).
            Err(error) if error.message.starts_with("no type of the model") => {
                total -= 1;
                continue;
            }
            Err(error) => {
                failed.push(format!("{}: read: {error}", file.display()));
                continue;
            }
        };
        for issue in &document.issues {
            let entry = issue_counts
                .entry(issue.message.clone())
                .or_insert_with(|| (0, format!("{} {issue}", file.display())));
            entry.0 += 1;
        }
        let written = write(&document.json, model(), &document.root_type).unwrap();
        let differences = compare_fragments(&xml, &written.xml, &NAMESPACES).unwrap();
        // What was reported while reading is what may differ.
        if document.issues.is_empty() && written.issues.is_empty() && differences.is_empty() {
            clean += 1;
        } else if !differences.is_empty() && document.issues.is_empty() {
            failed.push(format!(
                "{}: {} unexplained difference(s), first: {}",
                file.display(),
                differences.len(),
                differences[0]
            ));
        }
        let again = read(&written.xml, model(), Some(&document.root_type)).unwrap();
        if again.json != document.json {
            failed.push(format!(
                "{}: reading the written XML gives other JSON",
                file.display()
            ));
        }
    }
    let mut common: Vec<_> = issue_counts.into_iter().collect();
    common.sort_by_key(|c| std::cmp::Reverse(c.1.0));
    eprintln!(
        "corpus: {total} files, {clean} round-trip without issues, {} not well-formed",
        malformed.len()
    );
    for file in &malformed {
        eprintln!("  not well-formed: {file}");
    }
    for (message, (count, example)) in common.iter().take(25) {
        eprintln!("  {count:5}  {message}\n         e.g. {example}");
    }
    assert!(failed.is_empty(), "{}", failed.join("\n"));
}

const NAMESPACES: [(&str, &str); 4] = [
    ("", "urn:hl7-org:v3"),
    ("xsi", "http://www.w3.org/2001/XMLSchema-instance"),
    ("sdtc", "urn:hl7-org:sdtc"),
    ("voc", "urn:hl7-org:v3/voc"),
];

/// Reads a corpus file: a document, or a fragment that leaves out the namespace declarations
/// and whose root is an element of some type (`component`, `addr`): every element of that name
/// is tried, the reading with the fewest issues wins.
fn read_fragment(xml: &str) -> Result<Document, fml_xml::Error> {
    fn options(root: Option<&str>) -> ReadOptions<'_> {
        ReadOptions {
            root,
            namespaces: NAMESPACES.to_vec(),
        }
    }
    let first = match read_with(xml, model(), &options(None)) {
        Ok(document) => return Ok(document),
        Err(error) => error,
    };
    if first.message.starts_with("not well-formed") {
        return Err(first);
    }
    let local = root_name(xml).unwrap_or_default();
    let mut candidates: Vec<String> = model()
        .types()
        .flat_map(|ty| {
            ty.elements
                .iter()
                .filter(|e| e.xml_name == local)
                .map(|e| format!("{}.{}", ty.url, e.name))
                .collect::<Vec<_>>()
        })
        .collect();
    candidates.sort();
    candidates
        .iter()
        .filter_map(|root| read_with(xml, model(), &options(Some(root))).ok())
        .min_by_key(|document| document.issues.len())
        .ok_or(first)
}

/// The local name of the first element.
fn root_name(xml: &str) -> Option<String> {
    let mut rest = xml;
    loop {
        let at = rest.find('<')?;
        rest = &rest[at + 1..];
        if rest.starts_with('?') || rest.starts_with('!') {
            continue;
        }
        let name: String = rest
            .chars()
            .take_while(|c| !c.is_whitespace() && *c != '>' && *c != '/')
            .collect();
        return Some(name.rsplit(':').next().unwrap_or(&name).to_owned());
    }
}

fn collect(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect(&path, out);
        } else if path.extension().is_some_and(|e| e == "xml") {
            out.push(path);
        }
    }
}
