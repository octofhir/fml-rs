//! XML → JSON of the logical model.

use std::collections::HashMap;

use roxmltree::{Document as XmlDocument, Node, ParsingOptions};
use serde_json::{Map, Number, Value};

use crate::datetime;
use crate::model::{Element, Model, Representation, Root, TypeDef};
use crate::{Error, Issue, IssueKind, Position, TYPE_KEY, XML_NS, XSI_NS};

/// A document read from XML.
#[derive(Debug, Clone)]
pub struct Document {
    /// The content as JSON of the logical model: property names are the model's element names,
    /// attributes are primitives, the type `xsi:type` states is in `"@type"`.
    pub json: Value,
    /// The canonical URL of the root element's type.
    pub root_type: String,
    /// Where each element is in the XML, by its path in the JSON (`ClinicalDocument.component
    /// .structuredBody.component[2].section`), the form the engine's trace uses.
    pub positions: HashMap<String, Position>,
    /// What could not be read into the model: unknown elements and attributes, values that do
    /// not fit their type.
    pub issues: Vec<Issue>,
}

impl Document {
    /// The position of an element by its path.
    pub fn position(&self, path: &str) -> Option<Position> {
        self.positions.get(path).copied()
    }
}

/// How to read a document.
#[derive(Debug, Clone, Default)]
pub struct ReadOptions<'a> {
    /// The root: a canonical URL or type name, or either followed by `.element` for a fragment
    /// whose root is an element declared inline (`StructuredBody.component`). Without it, the
    /// type whose root element name matches.
    pub root: Option<&'a str>,
    /// Namespaces for a fragment that uses prefixes it does not declare, or no namespace at all:
    /// `("", default)` and `(prefix, uri)` pairs, in scope around the document.
    pub namespaces: Vec<(&'a str, &'a str)>,
}

/// Reads an XML document. `root_type` names the type of the root element (a canonical URL or a
/// type name); without it, the type is the one whose root element name matches.
pub fn read(xml: &str, model: &Model, root_type: Option<&str>) -> Result<Document, Error> {
    read_with(
        xml,
        model,
        &ReadOptions {
            root: root_type,
            namespaces: Vec::new(),
        },
    )
}

/// Reads an XML document with [`ReadOptions`].
pub fn read_with(xml: &str, model: &Model, options: &ReadOptions<'_>) -> Result<Document, Error> {
    let parsing = ParsingOptions {
        allow_dtd: false,
        ..ParsingOptions::default()
    };
    // A fragment is read inside an element that declares the namespaces; the XML declaration,
    // which may only come first, is blanked so line numbers stay.
    let (text, shift) = if options.namespaces.is_empty() {
        (std::borrow::Cow::Borrowed(xml), 0)
    } else {
        let (wrapped, shift) = wrap_fragment(xml, &options.namespaces);
        (std::borrow::Cow::Owned(wrapped), shift)
    };
    let doc = XmlDocument::parse_with_options(&text, parsing).map_err(|e| {
        let pos = e.pos();
        Error {
            message: format!("not well-formed XML: {e}"),
            position: Some(Position {
                line: pos.row,
                column: if pos.row == 1 {
                    pos.col.saturating_sub(shift)
                } else {
                    pos.col
                },
            }),
        }
    })?;
    let root = if shift > 0 {
        doc.root_element()
            .children()
            .find(|n| n.is_element())
            .ok_or_else(|| Error {
                message: "the fragment has no element".to_owned(),
                position: None,
            })?
    } else {
        doc.root_element()
    };
    let resolved = root_of(model, root, options.root)?;
    let expected_name = match resolved {
        Root::Element(_, element) => Some(element.xml_name.as_str()),
        Root::Type(ty) => ty.xml_name.as_deref(),
    };
    if let Some(expected) = expected_name
        && expected != root.tag_name().name()
    {
        return Err(Error {
            message: format!(
                "the root element is `{}`, not `{}`",
                root.tag_name().name(),
                expected
            ),
            position: None,
        });
    }
    let mut reader = Reader {
        model,
        doc: &doc,
        shift,
        positions: HashMap::new(),
        issues: Vec::new(),
    };
    let path = resolved.path();
    reader.at(root, &path);
    let json = match resolved {
        Root::Type(ty) => reader.complex(root, &Shape::of(ty), &path),
        Root::Element(ty, element) => {
            let context = Shape::of(ty);
            reader
                .element_value(root, element, &context, &path)
                .unwrap_or(Value::Null)
        }
    };
    Ok(Document {
        json,
        root_type: resolved.spec(),
        positions: reader.positions,
        issues: reader.issues,
    })
}

/// Puts a fragment inside a `<fragment>` element that declares `namespaces`; the XML
/// declaration, which may only come first, is blanked so line numbers stay. Returns the text and
/// the columns the wrapper adds to the first line.
pub(crate) fn wrap_fragment(xml: &str, namespaces: &[(&str, &str)]) -> (String, u32) {
    let mut open = String::from("<fragment");
    for (prefix, uri) in namespaces {
        if prefix.is_empty() {
            open.push_str(" xmlns=\"");
        } else {
            open.push_str(&format!(" xmlns:{prefix}=\""));
        }
        crate::write::escape_attribute(uri, &mut open);
        open.push('"');
    }
    open.push('>');
    let body = match xml.find("<?xml") {
        Some(start) if xml[..start].trim().is_empty() => {
            let end = xml[start..].find("?>").map_or(start, |e| start + e + 2);
            let mut blanked = String::with_capacity(xml.len());
            blanked.push_str(&xml[..start]);
            blanked.extend(
                xml[start..end]
                    .chars()
                    .map(|c| if c == '\n' { '\n' } else { ' ' }),
            );
            blanked.push_str(&xml[end..]);
            blanked
        }
        _ => xml.to_owned(),
    };
    let shift = open.chars().count() as u32;
    (format!("{open}{body}</fragment>"), shift)
}

fn root_of<'m>(
    model: &'m Model,
    root: Node<'_, '_>,
    stated: Option<&str>,
) -> Result<Root<'m>, Error> {
    if let Some(stated) = stated {
        if let Some(ty) = model.by_type_name(root.tag_name().namespace(), stated) {
            return Ok(Root::Type(ty));
        }
        return model.root(stated).ok_or_else(|| Error {
            message: format!("the model has no type or element `{stated}`"),
            position: None,
        });
    }
    let name = root.tag_name().name();
    let candidates = model.roots(root.tag_name().namespace(), name);
    match candidates.as_slice() {
        [single] => Ok(Root::Type(single)),
        [] => Err(Error {
            message: format!(
                "no type of the model has the root element `{name}`{}; say which type or element it is",
                root.tag_name()
                    .namespace()
                    .map(|ns| format!(" in namespace `{ns}`"))
                    .unwrap_or_default()
            ),
            position: None,
        }),
        several => Err(Error {
            message: format!(
                "the root element `{name}` can be any of {}; say which type it is",
                several
                    .iter()
                    .map(|t| t.url.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            position: None,
        }),
    }
}

/// The elements a node can hold: those of a type, or those declared inline on an element.
struct Shape<'m> {
    elements: &'m [Element],
    /// The structure the elements belong to: type codes resolve relative to it.
    owner: String,
    namespace: Option<String>,
}

impl<'m> Shape<'m> {
    fn of(ty: &'m TypeDef) -> Self {
        Shape {
            elements: &ty.elements,
            owner: ty.url.clone(),
            namespace: ty.namespace.clone(),
        }
    }

    fn text_element(&self) -> Option<&'m Element> {
        self.elements
            .iter()
            .find(|e| e.representation == Representation::Text)
    }

    fn choice_group(&self) -> Option<&'m Element> {
        self.elements.iter().find(|e| e.choice_group)
    }
}

/// Where an XML child element goes in the JSON.
enum Slot<'m> {
    Direct(&'m Element),
    /// A member of a choice group: the group, then the member.
    Grouped(&'m Element, &'m Element),
}

struct Reader<'m, 'd> {
    model: &'m Model,
    doc: &'d XmlDocument<'d>,
    /// Columns the fragment wrapper adds to the first line.
    shift: u32,
    positions: HashMap<String, Position>,
    issues: Vec<Issue>,
}

impl<'m> Reader<'m, '_> {
    fn position(&self, node: Node<'_, '_>) -> Position {
        let pos = self.doc.text_pos_at(node.range().start);
        Position {
            line: pos.row,
            column: if pos.row == 1 {
                pos.col.saturating_sub(self.shift)
            } else {
                pos.col
            },
        }
    }

    fn at(&mut self, node: Node<'_, '_>, path: &str) {
        let position = self.position(node);
        self.positions.insert(path.to_owned(), position);
    }

    fn issue(&mut self, kind: IssueKind, path: &str, node: Node<'_, '_>, message: String) {
        let position = Some(self.position(node));
        self.issues.push(Issue {
            kind,
            path: path.to_owned(),
            position,
            message,
        });
    }

    /// An element's content: attributes, text and child elements.
    fn complex(&mut self, node: Node<'_, '_>, shape: &Shape<'m>, path: &str) -> Value {
        let mut object = Map::new();
        self.attributes(node, shape, path, &mut object);
        let text_element = shape.text_element();
        let group = shape.choice_group();
        let has_children = node.children().any(|c| c.is_element());
        let mut text = String::new();
        let mut text_seen = false;
        for child in node.children() {
            if child.is_text() {
                let content = child.text().unwrap_or_default();
                if let Some(group) = group
                    && group.child("xmlText").is_some()
                {
                    if !content.trim().is_empty() {
                        let index = push_many(&mut object, &group.name, {
                            let mut item = Map::new();
                            item.insert("xmlText".to_owned(), Value::String(content.to_owned()));
                            Value::Object(item)
                        });
                        let _ = index;
                    }
                    continue;
                }
                if text_element.is_some() {
                    text.push_str(content);
                    text_seen = true;
                } else if !content.trim().is_empty() {
                    self.issue(
                        IssueKind::UnexpectedText,
                        path,
                        child,
                        format!("text `{}` where the model has no text", content.trim()),
                    );
                }
                continue;
            }
            if !child.is_element() {
                continue;
            }
            let expected_ns = node.tag_name().namespace();
            let Some(slot) = self.slot(shape, child, expected_ns) else {
                let name = qualified(child);
                self.issue(
                    IssueKind::UnknownElement,
                    path,
                    child,
                    format!("element `{name}` is not in the model; left out"),
                );
                continue;
            };
            match slot {
                Slot::Direct(element) => {
                    let index = count(&object, &element.name);
                    let child_path = if element.many {
                        format!("{path}.{}[{index}]", element.name)
                    } else {
                        format!("{path}.{}", element.name)
                    };
                    if !element.many && object.contains_key(&element.name) {
                        self.issue(
                            IssueKind::TooMany,
                            &child_path,
                            child,
                            format!("`{}` repeats but the model allows one", element.xml_name),
                        );
                        continue;
                    }
                    self.at(child, &child_path);
                    let Some(value) = self.element_value(child, element, shape, &child_path) else {
                        continue;
                    };
                    if element.many {
                        push_many(&mut object, &element.name, value);
                    } else {
                        object.insert(element.name.clone(), value);
                    }
                }
                Slot::Grouped(group, member) => {
                    let index = count(&object, &group.name);
                    let child_path = format!("{path}.{}[{index}].{}", group.name, member.name);
                    self.at(child, &child_path);
                    let inner = Shape {
                        elements: &group.children,
                        owner: shape.owner.clone(),
                        namespace: shape.namespace.clone(),
                    };
                    let Some(value) = self.element_value(child, member, &inner, &child_path) else {
                        continue;
                    };
                    let mut item = Map::new();
                    item.insert(member.name.clone(), value);
                    push_many(&mut object, &group.name, Value::Object(item));
                }
            }
        }
        if let Some(text_element) = text_element
            && text_seen
            && (!has_children || !text.trim().is_empty())
        {
            object.insert(text_element.name.clone(), Value::String(text));
        }
        Value::Object(object)
    }

    fn attributes(
        &mut self,
        node: Node<'_, '_>,
        shape: &Shape<'m>,
        path: &str,
        object: &mut Map<String, Value>,
    ) {
        for attribute in node.attributes() {
            let namespace = attribute.namespace();
            if namespace == Some(XSI_NS) {
                // `xsi:type` is read with the element; the schema location is not content.
                continue;
            }
            let element = shape.elements.iter().find(|e| {
                e.representation == Representation::Attribute
                    && e.xml_name == attribute.name()
                    && namespace
                        == if e.explicit_namespace {
                            e.namespace.as_deref()
                        } else {
                            None
                        }
            });
            let Some(element) = element else {
                let name = match namespace {
                    Some(ns) if ns == XML_NS => format!("xml:{}", attribute.name()),
                    Some(ns) => format!("{{{ns}}}{}", attribute.name()),
                    None => attribute.name().to_owned(),
                };
                self.issue(
                    IssueKind::UnknownAttribute,
                    path,
                    node,
                    format!("attribute `{name}` is not in the model; left out"),
                );
                continue;
            };
            let attr_path = format!("{path}.{}", element.name);
            let value = if element.many {
                Value::Array(
                    attribute
                        .value()
                        .split_whitespace()
                        .map(|token| self.primitive(token, element, &attr_path, node))
                        .collect(),
                )
            } else {
                self.primitive(attribute.value(), element, &attr_path, node)
            };
            object.insert(element.name.clone(), value);
        }
    }

    /// An attribute value as the JSON primitive of its type.
    fn primitive(
        &mut self,
        text: &str,
        element: &Element,
        path: &str,
        node: Node<'_, '_>,
    ) -> Value {
        if element.date_format {
            return match datetime::from_compact(text.trim()) {
                Some(converted) => Value::String(converted),
                None => {
                    self.issue(
                        IssueKind::InvalidValue,
                        path,
                        node,
                        format!("`{text}` is not a timestamp (YYYYMMDDHHMMSS.UUUU[+|-ZZzz]); kept as it is"),
                    );
                    Value::String(text.to_owned())
                }
            };
        }
        let code = element.single_type().unwrap_or_default();
        let invalid = |reader: &mut Self, what: &str| {
            reader.issue(
                IssueKind::InvalidValue,
                path,
                node,
                format!("`{text}` is not a valid {what}; kept as text"),
            );
            Value::String(text.to_owned())
        };
        match code {
            "boolean" => match text.trim() {
                "true" => Value::Bool(true),
                "false" => Value::Bool(false),
                _ => invalid(self, "boolean"),
            },
            "integer" | "positiveInt" | "unsignedInt" | "integer64" => {
                match text.trim().parse::<i64>() {
                    Ok(n) => Value::Number(n.into()),
                    Err(_) => invalid(self, "integer"),
                }
            }
            "decimal" => match decimal(text.trim()) {
                Some(n) => Value::Number(n),
                None => invalid(self, "decimal"),
            },
            _ => Value::String(text.to_owned()),
        }
    }

    /// Finds the element a child element is.
    fn slot(
        &self,
        shape: &Shape<'m>,
        child: Node<'_, '_>,
        parent_ns: Option<&str>,
    ) -> Option<Slot<'m>> {
        let name = child.tag_name().name();
        let namespace = child.tag_name().namespace();
        let fits = |element: &Element| {
            element.representation != Representation::Attribute
                && element.representation != Representation::Text
                && !element.choice_group
                && element.xml_name == name
                && namespace == element.namespace.as_deref().or(parent_ns)
        };
        for element in shape.elements {
            if element.choice_group {
                if let Some(member) = element.children.iter().find(|m| fits(m)) {
                    return Some(Slot::Grouped(element, member));
                }
            } else if fits(element) {
                return Some(Slot::Direct(element));
            }
        }
        None
    }

    /// The value of a child element: markup as a string, otherwise an object of its type.
    fn element_value(
        &mut self,
        node: Node<'_, '_>,
        element: &'m Element,
        context: &Shape<'m>,
        path: &str,
    ) -> Option<Value> {
        if element.representation == Representation::Markup {
            let mut out = String::new();
            write_markup(node, node.tag_name().namespace(), &mut Vec::new(), &mut out);
            return Some(Value::String(out));
        }
        let model: &'m Model = self.model;
        let declared: Option<&'m TypeDef> = element
            .single_type()
            .and_then(|code| model.type_of(code, &context.owner))
            .map(|t| &**t);
        let stated = node
            .attributes()
            .find(|a| a.namespace() == Some(XSI_NS) && a.name() == "type")
            .map(|a| a.value().to_owned());
        let mut actual: Option<&'m TypeDef> = match &stated {
            Some(qname) => {
                let (prefix, local) = match qname.split_once(':') {
                    Some((prefix, local)) => (Some(prefix), local),
                    None => (None, qname.as_str()),
                };
                let namespace = node.lookup_namespace_uri(prefix);
                match model.by_type_name(namespace, local) {
                    Some(found) => Some(&**found),
                    None => {
                        self.issue(
                            IssueKind::UnknownType,
                            path,
                            node,
                            format!("xsi:type `{qname}` is not a type of the model"),
                        );
                        None
                    }
                }
            }
            None => None,
        };
        if actual.is_none()
            && let Some(default) = &element.default_type
        {
            actual = model.type_of(default, &context.owner).map(|t| &**t);
        }
        // Inline elements describe the content unless xsi:type names another type.
        if actual.is_none() && !element.children.is_empty() {
            let shape = Shape {
                elements: &element.children,
                owner: context.owner.clone(),
                namespace: element.namespace.clone(),
            };
            return Some(self.complex(node, &shape, path));
        }
        if actual.is_none() {
            actual = declared;
        }
        let Some(actual) = actual else {
            let candidates: Vec<&str> = element
                .types
                .iter()
                .map(|t| t.rsplit('/').next().unwrap_or(t))
                .collect();
            self.issue(
                IssueKind::MissingType,
                path,
                node,
                format!(
                    "`{}` can be {}; without xsi:type its content cannot be read",
                    element.xml_name,
                    candidates.join(", ")
                ),
            );
            return None;
        };
        if let Some(stated_type) = declared
            && !model.derives_from(actual, &stated_type.url)
        {
            self.issue(
                IssueKind::UnknownType,
                path,
                node,
                format!("xsi:type `{}` is not a `{}`", actual.name, stated_type.name),
            );
        }
        let shape = Shape::of(actual);
        let mut value = self.complex(node, &shape, path);
        // The type is named when the XML states it, the element is polymorphic, or the value is
        // a subtype of the element's type.
        let polymorphic = element.types.len() > 1;
        let differs = declared.is_none_or(|d| d.url != actual.url);
        if (stated.is_some() || polymorphic || differs)
            && let Value::Object(object) = &mut value
        {
            let mut typed = Map::with_capacity(object.len() + 1);
            typed.insert(TYPE_KEY.to_owned(), Value::String(actual.name.clone()));
            typed.extend(std::mem::take(object));
            *object = typed;
        }
        Some(value)
    }
}

/// A decimal as a JSON number: an integer when written without a fraction (`12`), otherwise a
/// float. JSON numbers do not keep trailing zeros (`12.50` reads as `12.5`).
fn decimal(text: &str) -> Option<Number> {
    if !is_decimal_text(text) {
        return None;
    }
    if !text.contains('.')
        && let Ok(n) = text.parse::<i64>()
    {
        return Some(n.into());
    }
    text.parse::<f64>()
        .ok()
        .filter(|n| n.is_finite())
        .and_then(Number::from_f64)
}

/// Whether text is a decimal in XML Schema's lexical space (no exponent, no `inf`).
fn is_decimal_text(text: &str) -> bool {
    let digits = text.strip_prefix(['+', '-']).unwrap_or(text);
    !digits.is_empty()
        && digits.bytes().all(|b| b.is_ascii_digit() || b == b'.')
        && digits.bytes().filter(|&b| b == b'.').count() <= 1
        && digits.bytes().any(|b| b.is_ascii_digit())
}

fn count(object: &Map<String, Value>, name: &str) -> usize {
    match object.get(name) {
        Some(Value::Array(items)) => items.len(),
        Some(_) => 1,
        None => 0,
    }
}

fn push_many(object: &mut Map<String, Value>, name: &str, value: Value) -> usize {
    let entry = object
        .entry(name.to_owned())
        .or_insert_with(|| Value::Array(Vec::new()));
    match entry {
        Value::Array(items) => {
            items.push(value);
            items.len() - 1
        }
        other => {
            let previous = std::mem::take(other);
            *other = Value::Array(vec![previous, value]);
            1
        }
    }
}

fn qualified(node: Node<'_, '_>) -> String {
    match node.tag_name().namespace() {
        Some(ns) => format!("{{{ns}}}{}", node.tag_name().name()),
        None => node.tag_name().name().to_owned(),
    }
}

/// Writes an element and its content as markup. Elements in `default_ns` are written without a
/// prefix; others get the prefix the document used, declared where first needed.
fn write_markup(
    node: Node<'_, '_>,
    default_ns: Option<&str>,
    declared: &mut Vec<(String, String)>,
    out: &mut String,
) {
    let mark = declared.len();
    let mut declarations = String::new();
    let tag = name_in(
        node,
        node.tag_name().namespace(),
        node.tag_name().name(),
        default_ns,
        declared,
        &mut declarations,
        false,
    );
    out.push('<');
    out.push_str(&tag);
    let mut attributes = String::new();
    for attribute in node.attributes() {
        let name = name_in(
            node,
            attribute.namespace(),
            attribute.name(),
            default_ns,
            declared,
            &mut declarations,
            true,
        );
        attributes.push(' ');
        attributes.push_str(&name);
        attributes.push_str("=\"");
        crate::write::escape_attribute(attribute.value(), &mut attributes);
        attributes.push('"');
    }
    out.push_str(&declarations);
    out.push_str(&attributes);
    let mut children = node
        .children()
        .filter(|c| c.is_element() || c.is_text())
        .peekable();
    if children.peek().is_none() {
        out.push_str("/>");
    } else {
        out.push('>');
        for child in children {
            if child.is_text() {
                crate::write::escape_text(child.text().unwrap_or_default(), out);
            } else {
                write_markup(child, default_ns, declared, out);
            }
        }
        out.push_str("</");
        out.push_str(&tag);
        out.push('>');
    }
    declared.truncate(mark);
}

/// The name to write for an element or attribute in `namespace`, adding a namespace declaration
/// when the prefix is not declared yet.
fn name_in(
    node: Node<'_, '_>,
    namespace: Option<&str>,
    local: &str,
    default_ns: Option<&str>,
    declared: &mut Vec<(String, String)>,
    declarations: &mut String,
    attribute: bool,
) -> String {
    let Some(namespace) = namespace else {
        if !attribute && default_ns.is_some() {
            // An element in no namespace inside a default namespace.
            if !declarations.contains("xmlns=\"\"") {
                declarations.push_str(" xmlns=\"\"");
            }
        }
        return local.to_owned();
    };
    if namespace == XML_NS {
        return format!("xml:{local}");
    }
    if !attribute && Some(namespace) == default_ns {
        return local.to_owned();
    }
    if let Some((prefix, _)) = declared.iter().rev().find(|(_, ns)| ns == namespace) {
        return format!("{prefix}:{local}");
    }
    let prefix = node
        .lookup_prefix(namespace)
        .filter(|p| !p.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| format!("ns{}", declared.len() + 1));
    declarations.push_str(&format!(" xmlns:{prefix}=\""));
    crate::write::escape_attribute(namespace, declarations);
    declarations.push('"');
    declared.push((prefix.clone(), namespace.to_owned()));
    format!("{prefix}:{local}")
}
