//! JSON of the logical model → XML.

use std::collections::BTreeMap;

use serde_json::{Map, Value};

use crate::datetime;
use crate::model::{Element, Model, Representation, Root, TypeDef};
use crate::{Error, Issue, IssueKind, TYPE_KEY, XSI_NS};

/// XML written from JSON, and what could not be written.
#[derive(Debug, Clone)]
pub struct Written {
    pub xml: String,
    /// Properties the model does not have, values of the wrong shape, types it does not know.
    pub issues: Vec<Issue>,
}

/// Writes JSON of the logical model as an XML document whose root element has the type at
/// `root_type` (a canonical URL or a type name). Elements are written in definition order,
/// attributes as attributes, the `"@type"` of a value as `xsi:type`.
pub fn write(json: &Value, model: &Model, root_type: &str) -> Result<Written, Error> {
    let root = model.root(root_type).ok_or_else(|| Error {
        message: format!("the model has no type or element `{root_type}`"),
        position: None,
    })?;
    let (namespace, name) = match root {
        Root::Type(ty) => (
            ty.namespace.clone(),
            ty.xml_name.clone().unwrap_or_else(|| ty.name.clone()),
        ),
        Root::Element(_, element) => (element.namespace.clone(), element.xml_name.clone()),
    };
    let mut writer = Writer {
        model,
        default_ns: namespace.clone(),
        prefixes: BTreeMap::new(),
        issues: Vec::new(),
    };
    let mut body = String::new();
    let path = root.path();
    match root {
        Root::Type(ty) => {
            let shape = Shape::of(ty);
            writer.element(
                &mut body,
                0,
                namespace.as_deref(),
                &name,
                json,
                &shape,
                None,
                &path,
            );
        }
        Root::Element(ty, element) => {
            let shape = Shape::of(ty);
            writer.child(
                &mut body,
                0,
                element,
                json,
                &shape,
                namespace.as_deref(),
                &path,
            );
        }
    }
    // Namespace declarations go on the root, now that it is known which are used.
    let mut declarations = String::new();
    if let Some(ns) = &writer.default_ns {
        declarations.push_str(" xmlns=\"");
        escape_attribute(ns, &mut declarations);
        declarations.push('"');
    }
    for (ns, prefix) in &writer.prefixes {
        declarations.push_str(&format!(" xmlns:{prefix}=\""));
        escape_attribute(ns, &mut declarations);
        declarations.push('"');
    }
    let at = body[1..]
        .find([' ', '>', '/'])
        .map_or(body.len(), |i| i + 1);
    body.insert_str(at, &declarations);
    let mut xml = String::with_capacity(body.len() + 40);
    xml.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    xml.push_str(&body);
    xml.push('\n');
    Ok(Written {
        xml,
        issues: writer.issues,
    })
}

struct Shape<'m> {
    elements: &'m [Element],
    owner: String,
}

impl<'m> Shape<'m> {
    fn of(ty: &'m TypeDef) -> Self {
        Shape {
            elements: &ty.elements,
            owner: ty.url.clone(),
        }
    }
}

/// A child element to write.
enum Child<'v, 'm> {
    Element(&'m Element, &'v Value, String),
    Text(&'v str),
}

struct Writer<'m> {
    model: &'m Model,
    default_ns: Option<String>,
    /// Namespace → prefix, for namespaces other than the default.
    prefixes: BTreeMap<String, String>,
    issues: Vec<Issue>,
}

impl<'m> Writer<'m> {
    fn issue(&mut self, kind: IssueKind, path: &str, message: String) {
        self.issues.push(Issue {
            kind,
            path: path.to_owned(),
            position: None,
            message,
        });
    }

    /// A qualified name for an element or attribute in `namespace`.
    fn qname(&mut self, namespace: Option<&str>, local: &str, attribute: bool) -> String {
        match namespace {
            None => local.to_owned(),
            Some(ns) if !attribute && Some(ns) == self.default_ns.as_deref() => local.to_owned(),
            Some(ns) => format!("{}:{local}", self.prefix(ns)),
        }
    }

    fn prefix(&mut self, namespace: &str) -> String {
        if let Some(prefix) = self.prefixes.get(namespace) {
            return prefix.clone();
        }
        let wanted = if namespace == XSI_NS {
            "xsi".to_owned()
        } else {
            let tail = namespace
                .rsplit([':', '/'])
                .find(|s| !s.is_empty())
                .unwrap_or("ns");
            if tail.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
                && tail.chars().all(|c| c.is_ascii_alphanumeric())
            {
                tail.to_owned()
            } else {
                "ns".to_owned()
            }
        };
        let mut prefix = wanted.clone();
        let mut n = 1;
        while prefix == "xml" || self.prefixes.values().any(|p| *p == prefix) {
            n += 1;
            prefix = format!("{wanted}{n}");
        }
        self.prefixes.insert(namespace.to_owned(), prefix.clone());
        prefix
    }

    #[allow(clippy::too_many_arguments)]
    fn element(
        &mut self,
        out: &mut String,
        depth: usize,
        namespace: Option<&str>,
        name: &str,
        value: &Value,
        shape: &Shape<'m>,
        xsi_type: Option<&TypeDef>,
        path: &str,
    ) {
        let Value::Object(object) = value else {
            self.issue(
                IssueKind::InvalidValue,
                path,
                format!("`{name}` needs an object, not `{value}`"),
            );
            return;
        };
        let tag = self.qname(namespace, name, false);
        out.push('<');
        out.push_str(&tag);
        if let Some(ty) = xsi_type {
            let type_name = self.qname(ty.namespace.as_deref(), &ty.name, false);
            let attr = self.qname(Some(XSI_NS), "type", true);
            out.push_str(&format!(" {attr}=\"{type_name}\""));
        }
        self.check_keys(object, shape, path);
        // Attributes in definition order.
        for element in shape.elements {
            if element.representation != Representation::Attribute {
                continue;
            }
            let Some(value) = object.get(&element.name) else {
                continue;
            };
            let attr_path = format!("{path}.{}", element.name);
            let Some(text) = self.attribute_text(value, element, &attr_path) else {
                continue;
            };
            let ns = element
                .explicit_namespace
                .then_some(element.namespace.as_deref())
                .flatten();
            let attr = self.qname(ns, &element.xml_name, true);
            out.push(' ');
            out.push_str(&attr);
            out.push_str("=\"");
            escape_attribute(&text, out);
            out.push('"');
        }
        let children = self.children(object, shape, path);
        if children.is_empty() {
            out.push_str("/>");
            return;
        }
        out.push('>');
        let mixed = children.iter().any(|c| matches!(c, Child::Text(_)));
        let element_ns = namespace.map(str::to_owned);
        for child in &children {
            match child {
                Child::Text(text) => escape_text(text, out),
                Child::Element(element, value, child_path) => {
                    if !mixed {
                        newline(out, depth + 1);
                    }
                    self.child(
                        out,
                        depth + 1,
                        element,
                        value,
                        shape,
                        element_ns.as_deref(),
                        child_path,
                    );
                }
            }
        }
        if !mixed {
            newline(out, depth);
        }
        out.push_str("</");
        out.push_str(&tag);
        out.push('>');
    }

    /// Properties the model does not have are reported, not written.
    fn check_keys(&mut self, object: &Map<String, Value>, shape: &Shape<'m>, path: &str) {
        for key in object.keys() {
            if key == TYPE_KEY {
                continue;
            }
            if !shape.elements.iter().any(|e| e.name == *key) {
                self.issue(
                    IssueKind::UnknownElement,
                    path,
                    format!("`{key}` is not an element of the model; left out"),
                );
            }
        }
    }

    /// The content of an element, in order: text and child elements in definition order, the
    /// members of a choice group in the order they are listed.
    fn children<'v>(
        &mut self,
        object: &'v Map<String, Value>,
        shape: &Shape<'m>,
        path: &str,
    ) -> Vec<Child<'v, 'm>> {
        let mut children = Vec::new();
        for element in shape.elements {
            let Some(value) = object.get(&element.name) else {
                continue;
            };
            match element.representation {
                Representation::Attribute => {}
                Representation::Text => match value {
                    Value::String(text) => children.push(Child::Text(text)),
                    other => self.issue(
                        IssueKind::InvalidValue,
                        &format!("{path}.{}", element.name),
                        format!("text needs a string, not `{other}`"),
                    ),
                },
                Representation::Element | Representation::Markup if element.choice_group => {
                    for (index, item) in as_list(value).into_iter().enumerate() {
                        let item_path = format!("{path}.{}[{index}]", element.name);
                        let Value::Object(item) = item else {
                            self.issue(
                                IssueKind::InvalidValue,
                                &item_path,
                                "a choice group item needs an object".to_owned(),
                            );
                            continue;
                        };
                        for (key, member_value) in item {
                            match element.child(key) {
                                Some(member) if member.representation == Representation::Text => {
                                    if let Value::String(text) = member_value {
                                        children.push(Child::Text(text));
                                    }
                                }
                                // A member made by a map may come as a list.
                                Some(member) => {
                                    for (n, value) in as_list(member_value).into_iter().enumerate()
                                    {
                                        let member_path = match member_value {
                                            Value::Array(_) => format!("{item_path}.{key}[{n}]"),
                                            _ => format!("{item_path}.{key}"),
                                        };
                                        children.push(Child::Element(member, value, member_path));
                                    }
                                }
                                None => self.issue(
                                    IssueKind::UnknownElement,
                                    &item_path,
                                    format!("`{key}` is not a member of `{}`", element.name),
                                ),
                            }
                        }
                    }
                }
                Representation::Element | Representation::Markup => {
                    let values = as_list(value);
                    if !element.many && values.len() > 1 {
                        self.issue(
                            IssueKind::TooMany,
                            &format!("{path}.{}", element.name),
                            format!("`{}` allows one value; the first is written", element.name),
                        );
                    }
                    let many = element.many;
                    for (index, value) in values.into_iter().enumerate() {
                        if !many && index > 0 {
                            break;
                        }
                        let child_path = if many {
                            format!("{path}.{}[{index}]", element.name)
                        } else {
                            format!("{path}.{}", element.name)
                        };
                        children.push(Child::Element(element, value, child_path));
                    }
                }
            }
        }
        children
    }

    #[allow(clippy::too_many_arguments)]
    fn child(
        &mut self,
        out: &mut String,
        depth: usize,
        element: &'m Element,
        value: &Value,
        context: &Shape<'m>,
        parent_ns: Option<&str>,
        path: &str,
    ) {
        let namespace = element.namespace.as_deref().or(parent_ns);
        if element.representation == Representation::Markup {
            match value {
                Value::String(markup) if markup.trim_start().starts_with('<') => {
                    out.push_str(markup.trim());
                }
                Value::String(text) => {
                    // Plain text: an element holding it.
                    let tag = self.qname(namespace, &element.xml_name, false);
                    out.push_str(&format!("<{tag}>"));
                    escape_text(text, out);
                    out.push_str(&format!("</{tag}>"));
                }
                other => self.issue(
                    IssueKind::InvalidValue,
                    path,
                    format!("markup needs a string, not `{other}`"),
                ),
            }
            return;
        }
        let model: &'m Model = self.model;
        let declared: Option<&'m TypeDef> = element
            .single_type()
            .and_then(|code| model.type_of(code, &context.owner))
            .map(|t| &**t);
        let stated = value.get(TYPE_KEY).and_then(Value::as_str);
        let actual: Option<&'m TypeDef> = match stated {
            Some(name) => {
                let found = element
                    .types
                    .iter()
                    .filter_map(|code| model.type_of(code, &context.owner))
                    .find(|t| t.name == name || t.url == name)
                    .or_else(|| model.get(name))
                    .or_else(|| model.by_type_name(None, name))
                    .map(|t| &**t);
                if found.is_none() {
                    self.issue(
                        IssueKind::UnknownType,
                        path,
                        format!("`{TYPE_KEY}` `{name}` is not a type of the model"),
                    );
                }
                found
            }
            None => None,
        };
        if actual.is_none() && !element.children.is_empty() {
            let shape = Shape {
                elements: &element.children,
                owner: context.owner.clone(),
            };
            self.element(
                out,
                depth,
                namespace,
                &element.xml_name,
                value,
                &shape,
                None,
                path,
            );
            return;
        }
        let default: Option<&'m TypeDef> = element
            .default_type
            .as_deref()
            .and_then(|code| model.type_of(code, &context.owner))
            .map(|t| &**t);
        let Some(ty) = actual.or(default).or(declared) else {
            self.issue(
                IssueKind::MissingType,
                path,
                format!(
                    "`{}` can be several types; give one in `{TYPE_KEY}`",
                    element.name
                ),
            );
            return;
        };
        // xsi:type when the value names its type, the element is polymorphic, or the value is a
        // subtype of what the element declares.
        // xsi:type unless the type goes without saying: the element's default type, or its only
        // type when the value does not name one.
        let write_type = match (default, declared) {
            (Some(default), _) => default.url != ty.url,
            (None, Some(declared)) => stated.is_some() || declared.url != ty.url,
            (None, None) => true,
        };
        let shape = Shape::of(ty);
        let xsi = write_type.then_some(ty);
        self.element(
            out,
            depth,
            namespace,
            &element.xml_name,
            value,
            &shape,
            xsi,
            path,
        );
    }

    /// The text of an attribute value.
    fn attribute_text(&mut self, value: &Value, element: &Element, path: &str) -> Option<String> {
        let one = |writer: &mut Self, value: &Value| -> Option<String> {
            let text = match value {
                Value::String(text) => text.clone(),
                Value::Bool(b) => b.to_string(),
                Value::Number(n) => n.to_string(),
                Value::Null => return None,
                other => {
                    writer.issue(
                        IssueKind::InvalidValue,
                        path,
                        format!("an attribute needs a primitive, not `{other}`"),
                    );
                    return None;
                }
            };
            if element.date_format {
                return match datetime::to_compact(&text) {
                    Some(compact) => Some(compact),
                    None => {
                        // Already compact, or not a date: written as it is.
                        if datetime::from_compact(&text).is_none() {
                            writer.issue(
                                IssueKind::InvalidValue,
                                path,
                                format!("`{text}` is not a date or time; written as it is"),
                            );
                        }
                        Some(text)
                    }
                };
            }
            Some(text)
        };
        match value {
            Value::Array(items) => {
                let parts: Vec<String> = items.iter().filter_map(|v| one(self, v)).collect();
                (!parts.is_empty()).then(|| parts.join(" "))
            }
            single => one(self, single),
        }
    }
}

fn as_list(value: &Value) -> Vec<&Value> {
    match value {
        Value::Array(items) => items.iter().collect(),
        single => vec![single],
    }
}

fn newline(out: &mut String, depth: usize) {
    out.push('\n');
    for _ in 0..depth {
        out.push_str("  ");
    }
}

pub(crate) fn escape_text(text: &str, out: &mut String) {
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '\r' => out.push_str("&#13;"),
            _ => out.push(c),
        }
    }
}

pub(crate) fn escape_attribute(text: &str, out: &mut String) {
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '"' => out.push_str("&quot;"),
            '\n' => out.push_str("&#10;"),
            '\r' => out.push_str("&#13;"),
            '\t' => out.push_str("&#9;"),
            _ => out.push(c),
        }
    }
}
