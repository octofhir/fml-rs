//! The server driven as an editor drives it.

use std::sync::Arc;

use fml_check::{CoreDefinitions, Release};
use fml_lsp::Server;
use serde_json::{Value, json};

const URI: &str = "file:///map.fml";

const MAP: &str = r#"/// url = 'http://example.org/StructureMap/PatientToBasic'
/// name = 'PatientToBasic'
uses "http://hl7.org/fhir/StructureDefinition/Patient" alias P as source
uses "http://hl7.org/fhir/StructureDefinition/Basic" alias B as target

group Main(source src : P, target tgt : B) {
  src.name as n -> tgt.code as c then Name(n, c);
  src.id as id -> tgt.id = id;
}

group Name(source n, target c) {
  n.family as f where (f.length() > 1) -> c.text = truncate(f, 10);
}
"#;

struct Editor {
    server: Server,
    next: i64,
}

impl Editor {
    fn open(text: &str) -> (Self, Value) {
        let mut server = Server::new(Arc::new(CoreDefinitions::new(Release::R5)));
        server.handle(&json!({"jsonrpc": "2.0", "id": 0, "method": "initialize", "params": {"capabilities": {}}}));
        let replies = server.handle(&json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didOpen",
            "params": {"textDocument": {"uri": URI, "languageId": "fml", "version": 1, "text": text}}
        }));
        (
            Self { server, next: 1 },
            replies[0]["params"]["diagnostics"].clone(),
        )
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        self.next += 1;
        let replies = self.server.handle(
            &json!({"jsonrpc": "2.0", "id": self.next, "method": method, "params": params}),
        );
        assert_eq!(replies.len(), 1);
        assert!(replies[0].get("error").is_none(), "{}", replies[0]);
        replies[0]["result"].clone()
    }

    fn at(&mut self, method: &str, line: u32, character: u32, extra: Value) -> Value {
        let mut params = json!({
            "textDocument": {"uri": URI},
            "position": {"line": line, "character": character}
        });
        if let Value::Object(extra) = extra {
            params.as_object_mut().unwrap().extend(extra);
        }
        self.request(method, params)
    }
}

/// The position of the `nth` occurrence of `needle` in `text`, plus `shift` characters.
fn position(text: &str, needle: &str, nth: usize, shift: u32) -> (u32, u32) {
    let offset = text.match_indices(needle).nth(nth).unwrap().0;
    let before = &text[..offset];
    let line = before.matches('\n').count() as u32;
    let column = (offset - before.rfind('\n').map_or(0, |i| i + 1)) as u32;
    (line, column + shift)
}

fn labels(list: &Value) -> Vec<String> {
    list["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["label"].as_str().unwrap().to_owned())
        .collect()
}

#[test]
fn a_clean_map_has_no_diagnostics() {
    let (_, diagnostics) = Editor::open(MAP);
    assert_eq!(diagnostics, json!([]));
}

#[test]
fn type_problems_are_published() {
    let text = MAP.replace("src.id as id", "src.idd as id");
    let (_, diagnostics) = Editor::open(&text);
    assert_eq!(diagnostics[0]["message"], "`Patient` has no element `idd`");
    assert_eq!(diagnostics[0]["severity"], 1);
    assert_eq!(
        diagnostics[0]["range"]["start"],
        json!({"line": 7, "character": 6})
    );
}

#[test]
fn completes_elements_while_the_rule_is_unfinished() {
    let text = MAP.replace("  src.id as id -> tgt.id = id;\n", "  src.\n");
    let (mut editor, _) = Editor::open(&text);
    let (line, character) = position(&text, "src.\n", 0, 4);
    let list = editor.at("textDocument/completion", line, character, json!({}));
    let names = labels(&list);
    assert!(
        names.starts_with(&["identifier".to_owned(), "active".to_owned()]),
        "{names:?}"
    );
    assert!(names.contains(&"birthDate".to_owned()) && names.contains(&"id".to_owned()));
    // A choice element is offered once: `deceased`, with its types after `:`.
    assert!(
        names.contains(&"deceased".to_owned()) && !names.contains(&"deceasedBoolean".to_owned())
    );

    // Through a path, in an untyped group: `n` is a HumanName from the call.
    let text = MAP.replace(
        "c.text = truncate(f, 10)",
        "c.text = truncate(f, 10), n.period.",
    );
    let (mut editor, _) = Editor::open(&text);
    let (line, character) = position(&text, "n.period.", 0, 9);
    let names = labels(&editor.at("textDocument/completion", line, character, json!({})));
    assert!(names.contains(&"start".to_owned()), "{names:?}");
}

#[test]
fn completes_variables_groups_transforms_and_types() {
    let (mut editor, _) = Editor::open(MAP);
    // After `then`.
    let (line, character) = position(MAP, "then Name", 0, 5);
    let list = editor.at("textDocument/completion", line, character, json!({}));
    assert_eq!(labels(&list), ["Main", "Name"]);
    assert_eq!(list["items"][1]["insertText"], "Name(${1:n}, ${2:c})");

    // After `=`: transforms and the variables of the rule.
    let (line, character) = position(MAP, "= id;", 0, 2);
    let names = labels(&editor.at("textDocument/completion", line, character, json!({})));
    assert!(names.contains(&"create".to_owned()) && names.contains(&"translate".to_owned()));
    assert!(names.contains(&"id".to_owned()) && names.contains(&"src".to_owned()));

    // After `:` in a parameter: the `uses` aliases.
    let (line, character) = position(MAP, ": P", 0, 2);
    let names = labels(&editor.at("textDocument/completion", line, character, json!({})));
    assert_eq!(names, ["P", "B"]);
}

#[test]
fn hovers_variables_elements_and_transforms() {
    let (mut editor, _) = Editor::open(MAP);
    let hover = |editor: &mut Editor, needle: &str, nth: usize, shift: u32| {
        let (line, character) = position(MAP, needle, nth, shift);
        let hover = editor.at("textDocument/hover", line, character, json!({}));
        hover["contents"]["value"]
            .as_str()
            .unwrap_or_default()
            .to_owned()
    };
    assert!(
        hover(&mut editor, "n.family", 0, 0)
            .starts_with("```fml\nn : HumanName\n```\nsource parameter")
    );
    assert!(
        hover(&mut editor, "n.family", 0, 3).starts_with("```fml\nHumanName.family : string\n```")
    );
    assert!(
        hover(&mut editor, "src.name", 0, 5)
            .starts_with("```fml\nPatient.name : HumanName [0..*]\n```")
    );
    assert!(hover(&mut editor, "truncate", 0, 1).contains("truncate(source, length) : String"));
    assert_eq!(
        hover(&mut editor, "then Name", 0, 6),
        "```fml\ngroup Name(source n, target c)\n```"
    );
}

#[test]
fn follows_and_renames_variables_and_groups() {
    let (mut editor, _) = Editor::open(MAP);

    // `f` after `truncate(` goes to `as f`.
    let (line, character) = position(MAP, "truncate(f", 0, 9);
    let definition = editor.at("textDocument/definition", line, character, json!({}));
    let (line, character) = position(MAP, "as f", 0, 3);
    assert_eq!(
        definition["range"]["start"],
        json!({"line": line, "character": character})
    );

    // The `P` of a parameter type goes to its `uses` alias.
    let (line, character) = position(MAP, ": P", 0, 2);
    let definition = editor.at("textDocument/definition", line, character, json!({}));
    assert_eq!(definition["range"]["start"]["line"], 2);

    // References of the group `Name`: its declaration and the call.
    let (line, character) = position(MAP, "group Name", 0, 7);
    let references = editor.at(
        "textDocument/references",
        line,
        character,
        json!({"context": {"includeDeclaration": true}}),
    );
    assert_eq!(references.as_array().unwrap().len(), 2);

    // Renaming `f` changes the alias and its two uses.
    let (line, character) = position(MAP, "as f", 0, 3);
    let prepared = editor.at("textDocument/prepareRename", line, character, json!({}));
    assert_eq!(
        prepared["start"],
        json!({"line": line, "character": character})
    );
    let edit = editor.at(
        "textDocument/rename",
        line,
        character,
        json!({"newName": "family"}),
    );
    assert_eq!(edit["changes"][URI].as_array().unwrap().len(), 3);

    // A keyword is not a name.
    editor.next += 1;
    let replies = editor.server.handle(&json!({
        "jsonrpc": "2.0", "id": 99, "method": "textDocument/rename",
        "params": {"textDocument": {"uri": URI}, "position": {"line": line, "character": character}, "newName": "then"}
    }));
    assert_eq!(replies[0]["error"]["message"], "`then` is a keyword");
}

#[test]
fn outline_signature_help_formatting_and_tokens() {
    let (mut editor, _) = Editor::open(MAP);
    let symbols = editor.request(
        "textDocument/documentSymbol",
        json!({"textDocument": {"uri": URI}}),
    );
    let names: Vec<&str> = symbols
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["P", "B", "Main", "Name"]);
    assert_eq!(symbols[2]["children"].as_array().unwrap().len(), 4);

    let (line, character) = position(MAP, "truncate(f, 10)", 0, 12);
    let help = editor.at("textDocument/signatureHelp", line, character, json!({}));
    assert_eq!(
        help["signatures"][0]["label"],
        "truncate(source, length) : String"
    );
    assert_eq!(help["activeParameter"], 1);

    let (line, character) = position(MAP, "Name(n, c)", 0, 5);
    let help = editor.at("textDocument/signatureHelp", line, character, json!({}));
    assert_eq!(
        help["signatures"][0]["label"],
        "group Name(source n, target c)"
    );

    let messy = MAP.replace("src.id as id -> tgt.id = id;", "src.id   as id->tgt.id=id;");
    let (mut editor, _) = Editor::open(&messy);
    let edits = editor.request(
        "textDocument/formatting",
        json!({"textDocument": {"uri": URI}, "options": {"tabSize": 2, "insertSpaces": true}}),
    );
    assert!(
        edits[0]["newText"]
            .as_str()
            .unwrap()
            .contains("src.id as id -> tgt.id = id;")
    );

    let (mut editor, _) = Editor::open(MAP);
    let tokens = editor.request(
        "textDocument/semanticTokens/full",
        json!({"textDocument": {"uri": URI}}),
    );
    let data = tokens["data"].as_array().unwrap();
    assert_eq!(data.len() % 5, 0);
    // The first token is the alias `P`, a type declaration (type 5, modifier 1).
    assert_eq!(
        &data[..5],
        &[json!(2), json!(61), json!(1), json!(5), json!(1)]
    );
}

#[test]
fn speaks_the_base_protocol() {
    let mut input = Vec::new();
    for message in [
        json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"capabilities": {}}}),
        json!({"jsonrpc": "2.0", "method": "initialized", "params": {}}),
        json!({"jsonrpc": "2.0", "id": 2, "method": "shutdown"}),
        json!({"jsonrpc": "2.0", "method": "exit"}),
    ] {
        let body = message.to_string();
        input.extend(format!("Content-Length: {}\r\n\r\n{body}", body.len()).into_bytes());
    }
    let mut output = Vec::new();
    let code = fml_lsp::run(
        Server::new(Arc::new(CoreDefinitions::new(Release::R5))),
        &mut input.as_slice(),
        &mut output,
    )
    .unwrap();
    assert_eq!(code, 0);
    let output = String::from_utf8(output).unwrap();
    assert_eq!(output.matches("Content-Length").count(), 2);
    assert!(output.contains("\"semanticTokensProvider\""));
}
