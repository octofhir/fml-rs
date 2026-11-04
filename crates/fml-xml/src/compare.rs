//! Whether two XML documents say the same: the same elements and attributes in the same
//! namespaces, the same text; elements of one name in the same order (the order of differently
//! named siblings is the schema's business, and the writer always uses it). Prefixes, namespace declarations, attribute
//! order, comments, processing instructions, `xsi:schemaLocation` and whitespace between elements
//! do not count; an `xsi:type` compares by the type it names, not the prefix it is written with,
//! and numbers by value (`12.50` is `12.5`).

use roxmltree::{Document, Node, ParsingOptions};

use crate::{Error, XSI_NS};

/// A place where two documents differ.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Difference {
    /// The element, as `/ClinicalDocument/component/structuredBody/component[3]/section`.
    pub path: String,
    pub message: String,
}

impl std::fmt::Display for Difference {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.path, self.message)
    }
}

/// The differences between `expected` and `actual`; empty when they say the same.
pub fn compare(expected: &str, actual: &str) -> Result<Vec<Difference>, Error> {
    let expected = parse(expected, "expected")?;
    let actual = parse(actual, "actual")?;
    let mut differences = Vec::new();
    let a = expected.root_element();
    let b = actual.root_element();
    let path = format!("/{}", a.tag_name().name());
    compare_elements(a, b, &path, &mut differences);
    Ok(differences)
}

/// [`compare`] for fragments that use namespace prefixes they do not declare: both are read with
/// `namespaces` in scope (see [`ReadOptions::namespaces`](crate::ReadOptions)).
pub fn compare_fragments(
    expected: &str,
    actual: &str,
    namespaces: &[(&str, &str)],
) -> Result<Vec<Difference>, Error> {
    let (expected, _) = crate::read::wrap_fragment(expected, namespaces);
    let (actual, _) = crate::read::wrap_fragment(actual, namespaces);
    let expected = parse(&expected, "expected")?;
    let actual = parse(&actual, "actual")?;
    let mut differences = Vec::new();
    match (first_element(&expected), first_element(&actual)) {
        (Some(a), Some(b)) => {
            let path = format!("/{}", a.tag_name().name());
            compare_elements(a, b, &path, &mut differences);
        }
        _ => differences.push(Difference {
            path: "/".to_owned(),
            message: "a fragment has no element".to_owned(),
        }),
    }
    Ok(differences)
}

fn first_element<'a, 'input>(doc: &'a Document<'input>) -> Option<Node<'a, 'input>> {
    doc.root_element().children().find(|n| n.is_element())
}

fn parse<'input>(text: &'input str, which: &str) -> Result<Document<'input>, Error> {
    let options = ParsingOptions {
        allow_dtd: false,
        ..ParsingOptions::default()
    };
    Document::parse_with_options(text, options).map_err(|e| Error {
        message: format!("{which}: not well-formed XML: {e}"),
        position: None,
    })
}

type Name = (Option<String>, String);

fn name_of(node: Node<'_, '_>) -> Name {
    (
        node.tag_name().namespace().map(str::to_owned),
        node.tag_name().name().to_owned(),
    )
}

fn show(name: &Name) -> String {
    match &name.0 {
        Some(ns) => format!("{{{ns}}}{}", name.1),
        None => name.1.clone(),
    }
}

/// Attributes that count, as (name, value), sorted; `xsi:type` resolved to `{ns}name`.
fn attributes(node: Node<'_, '_>) -> Vec<(Name, String)> {
    let mut out: Vec<(Name, String)> = node
        .attributes()
        .filter(|a| !(a.namespace() == Some(XSI_NS) && a.name() == "schemaLocation"))
        .map(|a| {
            let name = (a.namespace().map(str::to_owned), a.name().to_owned());
            let value = if a.namespace() == Some(XSI_NS) && a.name() == "type" {
                let (prefix, local) = match a.value().split_once(':') {
                    Some((p, l)) => (Some(p), l),
                    None => (None, a.value()),
                };
                match node.lookup_namespace_uri(prefix) {
                    Some(ns) => format!("{{{ns}}}{local}"),
                    None => local.to_owned(),
                }
            } else {
                a.value().to_owned()
            };
            (name, value)
        })
        .collect();
    out.sort();
    out
}

/// Child elements in order, and the text of the element: its text pieces trimmed and joined.
/// Whitespace-only text does not count, nor does where text sits between child elements (a
/// reference before or after the text of an `ED` says the same).
fn content<'a, 'input>(node: Node<'a, 'input>) -> (Vec<Node<'a, 'input>>, String) {
    let mut elements = Vec::new();
    let mut texts: Vec<&str> = Vec::new();
    let has_elements = node.children().any(|c| c.is_element());
    let mut whole = String::new();
    for child in node.children() {
        if child.is_element() {
            elements.push(child);
        } else if child.is_text() {
            let text = child.text().unwrap_or_default();
            whole.push_str(text);
            if !text.trim().is_empty() {
                texts.push(text.trim());
            }
        }
    }
    let text = if has_elements {
        texts.join(" ")
    } else if whole.trim().is_empty() {
        String::new()
    } else {
        whole
    };
    (elements, text)
}

fn compare_elements(a: Node<'_, '_>, b: Node<'_, '_>, path: &str, out: &mut Vec<Difference>) {
    let (name_a, name_b) = (name_of(a), name_of(b));
    if name_a != name_b {
        out.push(Difference {
            path: path.to_owned(),
            message: format!("element `{}` became `{}`", show(&name_a), show(&name_b)),
        });
        return;
    }
    let (attrs_a, attrs_b) = (attributes(a), attributes(b));
    for (name, value) in &attrs_a {
        match attrs_b.iter().find(|(n, _)| n == name) {
            None => out.push(Difference {
                path: path.to_owned(),
                message: format!("attribute `{}=\"{value}\"` is missing", show(name)),
            }),
            Some((_, other)) if !same_value(value, other) => out.push(Difference {
                path: path.to_owned(),
                message: format!("attribute `{}`: `{value}` became `{other}`", show(name)),
            }),
            Some(_) => {}
        }
    }
    for (name, value) in &attrs_b {
        if !attrs_a.iter().any(|(n, _)| n == name) {
            out.push(Difference {
                path: path.to_owned(),
                message: format!("attribute `{}=\"{value}\"` was added", show(name)),
            });
        }
    }
    let ((elements_a, text_a), (elements_b, text_b)) = (content(a), content(b));
    if text_a != text_b {
        out.push(Difference {
            path: path.to_owned(),
            message: match (text_a.is_empty(), text_b.is_empty()) {
                (false, true) => format!("text `{}` is missing", clip(&text_a)),
                (true, false) => format!("text `{}` was added", clip(&text_b)),
                _ => format!("text `{}` became `{}`", clip(&text_a), clip(&text_b)),
            },
        });
    }
    // Elements compare by name, in order within each name: the order of differently named
    // siblings is the schema's, not the content's.
    let groups = |elements: &[Node<'_, '_>]| {
        let mut groups: Vec<(Name, Vec<usize>)> = Vec::new();
        for (index, element) in elements.iter().enumerate() {
            let name = name_of(*element);
            match groups.iter_mut().find(|(n, _)| *n == name) {
                Some((_, indexes)) => indexes.push(index),
                None => groups.push((name, vec![index])),
            }
        }
        groups
    };
    let (groups_a, groups_b) = (groups(&elements_a), groups(&elements_b));
    for (name, indexes) in &groups_a {
        let others = groups_b
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, i)| i.as_slice())
            .unwrap_or_default();
        for (n, &index) in indexes.iter().enumerate() {
            match others.get(n) {
                Some(&other) => {
                    let child_path = format!("{path}/{}[{}]", name.1, n + 1);
                    compare_elements(elements_a[index], elements_b[other], &child_path, out);
                }
                None => out.push(Difference {
                    path: path.to_owned(),
                    message: format!("element `{}` [{}] is missing", show(name), n + 1),
                }),
            }
        }
        for n in indexes.len()..others.len() {
            out.push(Difference {
                path: path.to_owned(),
                message: format!("element `{}` [{}] was added", show(name), n + 1),
            });
        }
    }
    for (name, indexes) in &groups_b {
        if !groups_a.iter().any(|(n, _)| n == name) {
            out.push(Difference {
                path: path.to_owned(),
                message: format!(
                    "element `{}` was added ({} times)",
                    show(name),
                    indexes.len()
                ),
            });
        }
    }
}

/// Attribute values are the same when equal as text, or as decimal numbers (`12.50` and `12.5`:
/// the JSON form keeps numbers, not how they were written).
fn same_value(a: &str, b: &str) -> bool {
    if a == b {
        return true;
    }
    let number = |s: &str| {
        let s = s.trim();
        let digits = s.strip_prefix(['+', '-']).unwrap_or(s);
        (!digits.is_empty() && digits.bytes().all(|c| c.is_ascii_digit() || c == b'.'))
            .then(|| s.parse::<f64>().ok())
            .flatten()
    };
    matches!((number(a), number(b)), (Some(x), Some(y)) if x == y)
}

fn clip(text: &str) -> String {
    let text = text.trim();
    if text.chars().count() > 60 {
        format!("{}…", text.chars().take(60).collect::<String>())
    } else {
        text.to_owned()
    }
}
