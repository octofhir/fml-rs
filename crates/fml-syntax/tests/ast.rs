//! The typed tree over examples from the Mapping Language IG.

use fml_syntax::ast::{GroupTypeMode, ListMode, ModelMode, ParameterMode, TransformValue};
use fml_syntax::parse;

#[test]
fn declarations() {
    let parse = parse(
        r#"/// url = 'http://example.org/StructureMap/Demo'
/// description = """
Multi-line **markdown**.
"""
map "http://example.org/StructureMap/Old" = "Old"
uses "http://hl7.org/fhir/StructureDefinition/Patient" alias Pat as source
uses "http://hl7.org/fhir/StructureDefinition/Basic" as target
imports "http://example.org/StructureMap/*"
let base = 'http://example.org/';

conceptmap "Gender" {
  prefix s = "http://hl7.org/fhir/administrative-gender"
  prefix t = "http://terminology.hl7.org/CodeSystem/v2-0001"
  s:male - t:M
  s:"other" == t:O
}

group Demo(source src : Pat, target tgt : Basic) extends Base <<type+>> {
  src.id -> tgt.id;
}
"#,
    );
    assert!(parse.diagnostics().is_empty(), "{:?}", parse.diagnostics());
    let file = parse.tree();

    let metadata: Vec<_> = file
        .metadata()
        .map(|m| (m.key().unwrap().text(), m.value().and_then(|v| v.value())))
        .collect();
    assert_eq!(
        metadata,
        vec![
            (
                "url".to_owned(),
                Some("http://example.org/StructureMap/Demo".to_owned())
            ),
            (
                "description".to_owned(),
                Some("\nMulti-line **markdown**.\n".to_owned())
            ),
        ]
    );

    let map = file.map_declaration().unwrap();
    assert_eq!(map.name().unwrap().value().as_deref(), Some("Old"));

    let uses: Vec<_> = file
        .uses()
        .map(|u| (u.url().unwrap().value().unwrap(), u.alias(), u.mode()))
        .collect();
    assert_eq!(uses[0].1.as_deref(), Some("Pat"));
    assert_eq!(uses[0].2, Some(ModelMode::Source));
    assert_eq!(uses[1].1, None);
    assert_eq!(uses[1].2, Some(ModelMode::Target));

    assert_eq!(
        file.imports()
            .next()
            .unwrap()
            .url()
            .unwrap()
            .value()
            .as_deref(),
        Some("http://example.org/StructureMap/*")
    );
    let constant = file.constants().next().unwrap();
    assert_eq!(constant.name().as_deref(), Some("base"));
    assert_eq!(
        constant.expression().unwrap().text(),
        "'http://example.org/'"
    );

    let concept_map = file.concept_maps().next().unwrap();
    assert_eq!(concept_map.prefixes().count(), 2);
    let entries: Vec<_> = concept_map
        .entries()
        .map(|e| {
            (
                e.source().unwrap().code().unwrap(),
                e.relation().unwrap(),
                e.target().unwrap().code().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        entries,
        vec![
            ("male".to_owned(), "-".to_owned(), "M".to_owned()),
            ("other".to_owned(), "==".to_owned(), "O".to_owned()),
        ]
    );

    let group = file.groups().next().unwrap();
    assert_eq!(group.name().as_deref(), Some("Demo"));
    assert_eq!(group.extends().as_deref(), Some("Base"));
    assert_eq!(group.type_mode(), Some(GroupTypeMode::TypeAndTypes));
    let parameters: Vec<_> = group
        .parameters()
        .map(|p| {
            (
                p.mode().unwrap(),
                p.name().unwrap(),
                p.type_annotation().and_then(|t| t.type_name()),
            )
        })
        .collect();
    assert_eq!(
        parameters,
        vec![
            (
                ParameterMode::Source,
                "src".to_owned(),
                Some("Pat".to_owned())
            ),
            (
                ParameterMode::Target,
                "tgt".to_owned(),
                Some("Basic".to_owned())
            ),
        ]
    );
}

#[test]
fn rules() {
    let parse = parse(
        r#"group G(source src, target tgt) {
  src.value : string 0..1 default ('x') first as v where (v.length() > 2) check (v != '') log (v) -> tgt.text = v "copy";
  src.name as n -> tgt.name as t share names then {
    n.family as f -> t.family = f;
  } "names";
  src -> tgt.extension as ext, ext.value = create('Reference') as ref, ref.reference = reference(src);
  src.a as a then Inner(a, tgt) "inner";
  src.`group!` -> tgt.`x y`;
  src.b where value = 'crick' -> tgt.strand = (src.c + 1) "legacyWhere";
  src.c -> tgt.c : id, extension;
}
"#,
    );
    assert!(parse.diagnostics().is_empty(), "{:?}", parse.diagnostics());
    let rules: Vec<_> = parse.tree().groups().next().unwrap().rules().collect();
    assert_eq!(rules.len(), 7);

    let first = &rules[0];
    let source = first.sources().next().unwrap();
    assert_eq!(source.path().unwrap().segments(), vec!["src", "value"]);
    assert_eq!(
        source.type_annotation().unwrap().type_name().as_deref(),
        Some("string")
    );
    assert_eq!(source.cardinality(), Some(("0".into(), "1".into())));
    assert_eq!(
        source
            .default_value()
            .unwrap()
            .expression()
            .unwrap()
            .unparenthesized()
            .text(),
        "'x'"
    );
    assert_eq!(source.list_mode(), Some(ListMode::First));
    assert_eq!(source.alias().as_deref(), Some("v"));
    assert_eq!(
        source.where_clause().unwrap().unparenthesized().text(),
        "v.length() > 2"
    );
    assert_eq!(source.check_clause().unwrap().text(), "(v != '')");
    assert_eq!(source.log_clause().unwrap().text(), "(v)");
    let target = first.targets().next().unwrap();
    assert_eq!(target.path().unwrap().text(), "tgt.text");
    assert!(matches!(target.transform(), Some(TransformValue::Name(n)) if n.text() == "v"));
    assert_eq!(first.name().as_deref(), Some("copy"));

    let shared = rules[1].targets().next().unwrap();
    assert_eq!(shared.list_mode(), Some(ListMode::Share("names".into())));
    let dependent = rules[1].dependent().unwrap();
    assert_eq!(dependent.rules().unwrap().rules().count(), 1);

    let targets: Vec<_> = rules[2].targets().collect();
    assert_eq!(targets.len(), 3);
    match targets[1].transform() {
        Some(TransformValue::Invocation(call)) => {
            assert_eq!(call.name().as_deref(), Some("create"));
            assert_eq!(call.arguments().next().unwrap().text(), "'Reference'");
        }
        other => panic!("expected create(), got {other:?}"),
    }
    assert_eq!(targets[1].alias().as_deref(), Some("ref"));

    let inner = rules[3].dependent().unwrap();
    let call = inner.invocations().next().unwrap();
    assert_eq!(call.name().as_deref(), Some("Inner"));
    assert_eq!(call.arguments().count(), 2);
    assert_eq!(rules[3].targets().count(), 0);

    let quoted = rules[4].sources().next().unwrap();
    assert_eq!(quoted.path().unwrap().segments(), vec!["src", "group!"]);

    let legacy = rules[5].sources().next().unwrap();
    assert_eq!(legacy.where_clause().unwrap().text(), "value = 'crick'");
    assert!(matches!(
        rules[5].targets().next().unwrap().transform(),
        Some(TransformValue::Expression(_))
    ));

    assert_eq!(rules[6].identity_fields(), vec!["id", "extension"]);
}

#[test]
fn recovers_from_errors() {
    let parse = parse(
        r#"group A(source s, target t) {
  s.a -> t.a
  s.b -> ;
  s.c -> t.c;
}

group B(source s, target t) {
  s.d -> t.d;
}
"#,
    );
    assert!(parse.has_errors());
    assert_eq!(
        parse.syntax().to_string().len(),
        parse.syntax().text().len().into()
    );
    let groups: Vec<_> = parse.tree().groups().collect();
    assert_eq!(groups.len(), 2);
    assert_eq!(groups[1].rules().count(), 1);
    let last = groups[0].rules().last().unwrap();
    assert_eq!(last.sources().next().unwrap().path().unwrap().text(), "s.c");
}
