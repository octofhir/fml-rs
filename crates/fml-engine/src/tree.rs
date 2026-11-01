//! The values a map works on: source and target content as one tree of typed nodes.
//!
//! Variables point at nodes, so a target created by one rule and filled by the next is the same
//! node. Each node knows its FHIR type where the definitions tell it, which the engine needs for
//! type filters (`src.value : string`), choice elements (`value` holding a Reference is written as
//! `valueReference`), repeating elements, and default groups chosen by type.

use fml_check::{Lookup, Type, Types};
use serde_json::{Map, Value};

/// A node in the [`Tree`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NodeId(usize);

#[derive(Debug, Clone)]
struct Node {
    ty: Type,
    /// The FHIR type when no definition is known for it (a FHIRPath result, for example).
    type_name: Option<String>,
    /// A resource: written with `resourceType`.
    resource: bool,
    data: Data,
}

#[derive(Debug, Clone)]
enum Data {
    /// A primitive value (`Null` until it is assigned), and its element's `id` and `extension`,
    /// which FHIR JSON writes as the `_name` sibling.
    Primitive(Value, Vec<Property>),
    Complex(Vec<Property>),
}

#[derive(Debug, Clone)]
struct Property {
    /// The element name; for a choice, without the type suffix (`value`).
    name: String,
    choice: bool,
    many: bool,
    /// Position in the definition, for writing elements in order.
    index: Option<usize>,
    values: Vec<NodeId>,
}

/// An element of a node, as the definitions describe it.
struct Slot {
    name: String,
    choice: bool,
    many: bool,
    index: Option<usize>,
    /// The element's type when it has a single one.
    ty: Type,
}

#[derive(Debug, Default, Clone)]
pub struct Tree {
    nodes: Vec<Node>,
}

impl Tree {
    fn push(&mut self, node: Node) -> NodeId {
        self.nodes.push(node);
        NodeId(self.nodes.len() - 1)
    }

    /// An empty node of a type: complex, or primitive when the type is a primitive.
    pub fn create(&mut self, ty: Type, type_name: Option<String>) -> NodeId {
        let data = if ty.is_primitive() {
            Data::Primitive(Value::Null, Vec::new())
        } else {
            Data::Complex(Vec::new())
        };
        self.push(Node {
            resource: ty.is_resource(),
            ty,
            type_name,
            data,
        })
    }

    /// A primitive node holding `value`.
    pub fn primitive(&mut self, value: Value, ty: Type, type_name: Option<String>) -> NodeId {
        self.push(Node {
            ty,
            type_name,
            resource: false,
            data: Data::Primitive(value, Vec::new()),
        })
    }

    pub fn fhir_type(&self, id: NodeId) -> Option<String> {
        let node = &self.nodes[id.0];
        node.ty.fhir_type().or_else(|| node.type_name.clone())
    }

    pub fn ty(&self, id: NodeId) -> &Type {
        &self.nodes[id.0].ty
    }

    pub fn is_primitive(&self, id: NodeId) -> bool {
        matches!(self.nodes[id.0].data, Data::Primitive(..))
    }

    /// The primitive value of a node, `None` for complex nodes.
    pub fn value(&self, id: NodeId) -> Option<&Value> {
        match &self.nodes[id.0].data {
            Data::Primitive(value, _) => Some(value),
            Data::Complex(_) => None,
        }
    }

    /// The primitive value as text, as FHIRPath's string conversion gives it.
    pub fn text(&self, id: NodeId) -> Option<String> {
        match self.value(id)? {
            Value::String(s) => Some(s.clone()),
            Value::Null => None,
            other => Some(other.to_string()),
        }
    }

    // ---- Loading ----------------------------------------------------------------------------

    /// Loads JSON as a node of type `ty`; nested elements get the types the definitions give.
    pub fn load(&mut self, json: &Value, ty: Type, types: &Types<'_>) -> NodeId {
        // `resourceType` marks a resource, unless the type has an element of that name
        // (ExampleScenario.instance.resourceType names the type of a resource it talks about).
        let marks_resource = json.get("resourceType").is_some()
            && !matches!(types.lookup(&ty, "resourceType"), Lookup::Found(_));
        if !marks_resource && json.is_object() {
            return self.load_object(json, ty, None, types);
        }
        let ty = match (&ty, json.get("resourceType").and_then(Value::as_str)) {
            // A resource in a `Resource`-typed element, or a top-level input: its own type.
            (_, Some(resource_type)) if ty.fhir_type().as_deref() != Some(resource_type) => {
                let context = ty
                    .structure()
                    .map(|s| s.url.clone())
                    .unwrap_or_else(|| "http://hl7.org/fhir/StructureDefinition/".to_owned());
                match types.structure(&fml_check::definitions::type_url(resource_type, &context)) {
                    Type::Unknown => ty,
                    known => known,
                }
            }
            _ => ty,
        };
        let type_name = json
            .get("resourceType")
            .and_then(Value::as_str)
            .map(str::to_owned);
        if !json.is_object() {
            return self.primitive(json.clone(), ty, type_name);
        }
        self.load_object(json, ty, type_name, types)
    }

    /// Loads a JSON object; `resource_type` is set when it is a resource.
    fn load_object(
        &mut self,
        json: &Value,
        ty: Type,
        resource_type: Option<String>,
        types: &Types<'_>,
    ) -> NodeId {
        let Value::Object(object) = json else {
            return self.primitive(json.clone(), ty, None);
        };
        let is_resource = resource_type.is_some();
        let id = self.push(Node {
            resource: is_resource || ty.is_resource(),
            ty: ty.clone(),
            type_name: resource_type,
            data: Data::Complex(Vec::new()),
        });
        for (key, value) in object {
            if (is_resource && key == "resourceType")
                || (key.starts_with('_') && object.contains_key(&key[1..]))
            {
                continue;
            }
            // `_name` without `name`: a primitive that has only an id or extensions.
            if let Some(base) = key.strip_prefix('_') {
                let (slot, child_ty) = self.slot_for_key(&ty, base, types);
                let elements: Vec<&Value> = match value {
                    Value::Array(items) => items.iter().collect(),
                    single => vec![single],
                };
                let children: Vec<NodeId> = elements
                    .into_iter()
                    .map(|element| {
                        let child = self.primitive(Value::Null, child_ty.clone(), None);
                        self.load_primitive_element(child, element, types);
                        child
                    })
                    .collect();
                let many = slot.many || value.is_array();
                self.properties_mut(id).push(Property {
                    name: slot.name,
                    choice: slot.choice,
                    many,
                    index: slot.index,
                    values: children,
                });
                continue;
            }
            let (slot, child_ty) = self.slot_for_key(&ty, key, types);
            let values: Vec<&Value> = match value {
                Value::Array(items) => items.iter().collect(),
                single => vec![single],
            };
            let mut children = Vec::with_capacity(values.len());
            for item in values {
                children.push(self.load(item, child_ty.clone(), types));
            }
            // The primitives' `id` and `extension`, from the `_name` sibling.
            if let Some(elements) = object.get(&format!("_{key}")) {
                let elements: Vec<&Value> = match elements {
                    Value::Array(items) => items.iter().collect(),
                    single => vec![single],
                };
                for (&child, element) in children.iter().zip(elements) {
                    self.load_primitive_element(child, element, types);
                }
            }
            let many = slot.many || value.is_array();
            self.properties_mut(id).push(Property {
                name: slot.name,
                choice: slot.choice,
                many,
                index: slot.index,
                values: children,
            });
        }
        id
    }

    /// Loads a primitive's `_name` object (`id`, `extension`) into the primitive node.
    fn load_primitive_element(&mut self, primitive: NodeId, element: &Value, types: &Types<'_>) {
        if !element.is_object() || !self.is_primitive(primitive) {
            return;
        }
        let holder = self.load(element, self.ty(primitive).clone(), types);
        let properties = std::mem::take(self.properties_mut(holder));
        *self.properties_mut(primitive) = properties;
    }

    /// The element a JSON key holds: `valueString` is the choice `value` with type `string`.
    fn slot_for_key(&self, ty: &Type, key: &str, types: &Types<'_>) -> (Slot, Type) {
        if let Lookup::Found(found) = types.lookup(ty, key) {
            if let Some(base) = &found.element.choice_of {
                let base_slot = match types.lookup(ty, base) {
                    Lookup::Found(base_found) => base_found.element.index,
                    _ => found.element.index,
                };
                let child = types.element_type(&found, None);
                return (
                    Slot {
                        name: base.clone(),
                        choice: true,
                        many: false,
                        index: base_slot,
                        ty: child.clone(),
                    },
                    child,
                );
            }
            let child = types.element_type(&found, None);
            return (
                Slot {
                    name: key.to_owned(),
                    choice: found.element.is_choice(),
                    many: found.element.array,
                    index: found.element.index,
                    ty: child.clone(),
                },
                child,
            );
        }
        (
            Slot {
                name: key.to_owned(),
                choice: false,
                many: false,
                index: None,
                ty: Type::Unknown,
            },
            Type::Unknown,
        )
    }

    // ---- Navigation -------------------------------------------------------------------------

    /// The elements of a node; for a primitive, those of its element (`id`, `extension`).
    fn properties(&self, id: NodeId) -> &[Property] {
        match &self.nodes[id.0].data {
            Data::Complex(properties) | Data::Primitive(_, properties) => properties,
        }
    }

    fn properties_mut(&mut self, id: NodeId) -> &mut Vec<Property> {
        match &mut self.nodes[id.0].data {
            Data::Complex(properties) | Data::Primitive(_, properties) => properties,
        }
    }

    /// The children named `name`. A choice is found by its name (`value`) or by the name with
    /// the type (`valueString`). A primitive's `value` is the primitive itself.
    pub fn children(&self, id: NodeId, name: &str) -> Vec<NodeId> {
        if name == "value" && self.is_primitive(id) {
            return vec![id];
        }
        let mut out = Vec::new();
        for property in self.properties(id) {
            if property.name == name {
                out.extend(&property.values);
            } else if property.choice && name.starts_with(&property.name) {
                for &value in &property.values {
                    let suffix = &name[property.name.len()..];
                    if self
                        .fhir_type(value)
                        .is_some_and(|t| capitalize(&t) == suffix)
                    {
                        out.push(value);
                    }
                }
            }
        }
        out
    }

    // ---- Writing ----------------------------------------------------------------------------

    /// The element `name` of `id`, as the definitions describe it.
    fn slot(&self, id: NodeId, name: &str, types: &Types<'_>) -> Result<Slot, String> {
        let ty = self.ty(id).clone();
        match types.lookup(&ty, name) {
            Lookup::Found(found) => Ok(Slot {
                name: name.to_owned(),
                choice: found.element.is_choice(),
                many: found.element.array,
                index: found.element.index,
                ty: types.element_type(&found, None),
            }),
            Lookup::Missing => Err(format!("`{}` has no element `{name}`", ty.display())),
            Lookup::Unknown => {
                // No definition: keep what is there. An element already holding several values
                // repeats.
                let existing = self.properties(id).iter().find(|p| p.name == name);
                Ok(Slot {
                    name: name.to_owned(),
                    choice: false,
                    many: existing.is_some_and(|p| p.many),
                    index: None,
                    ty: Type::Unknown,
                })
            }
        }
    }

    /// The target element `name` of `id` to write into: a new value for a repeating element, the
    /// existing (or a new) value for a single one.
    pub fn make_property(
        &mut self,
        id: NodeId,
        name: &str,
        types: &Types<'_>,
    ) -> Result<NodeId, String> {
        if self.is_primitive(id) && name == "value" {
            return Ok(id);
        }
        let slot = self.slot(id, name, types)?;
        if slot.choice {
            return Err(format!(
                "`{}.{name}` is a choice; give the type with a transform such as create('Type')",
                self.ty(id).display()
            ));
        }
        if !slot.many
            && let Some(existing) = self
                .properties(id)
                .iter()
                .find(|p| p.name == name)
                .and_then(|p| p.values.first().copied())
        {
            return Ok(existing);
        }
        let child = self.create(slot.ty.clone(), None);
        self.attach(id, slot, child);
        Ok(child)
    }

    /// Sets the element `name` of `id` to `value`: appended to a repeating element, replacing a
    /// single one. A primitive value takes the element's type.
    pub fn set_property(
        &mut self,
        id: NodeId,
        name: &str,
        value: NodeId,
        types: &Types<'_>,
    ) -> Result<(), String> {
        // A primitive's `value` is the primitive itself.
        if self.is_primitive(id) && name == "value" {
            let assigned = self.value(value).cloned().unwrap_or(Value::Null);
            if let Data::Primitive(current, _) = &mut self.nodes[id.0].data {
                *current = assigned;
            }
            return Ok(());
        }
        let slot = self.slot(id, name, types)?;
        if slot.choice && self.fhir_type(value).is_none() {
            return Err(format!(
                "cannot tell which type of `{}.{name}` the value has",
                self.ty(id).display()
            ));
        }
        if !slot.choice && self.is_primitive(value) && !matches!(slot.ty, Type::Unknown) {
            self.nodes[value.0].ty = slot.ty.clone();
        }
        if !slot.many {
            self.properties_mut(id).retain(|p| p.name != name);
        }
        self.attach(id, slot, value);
        Ok(())
    }

    fn attach(&mut self, id: NodeId, slot: Slot, child: NodeId) {
        let properties = self.properties_mut(id);
        match properties.iter_mut().find(|p| p.name == slot.name) {
            Some(property) => property.values.push(child),
            None => properties.push(Property {
                name: slot.name,
                choice: slot.choice,
                many: slot.many,
                index: slot.index,
                values: vec![child],
            }),
        }
    }

    /// Copies the content of `from` into `to`, as when a value is assigned to an existing target.
    pub fn assign(&mut self, to: NodeId, from: NodeId) {
        let copy = self.deep_copy(from);
        let node = self.nodes[copy.0].clone();
        let target = &mut self.nodes[to.0];
        target.data = node.data;
        if matches!(target.ty, Type::Unknown) {
            target.ty = node.ty;
            target.type_name = node.type_name;
        }
    }

    /// A copy of the subtree at `id`.
    pub fn deep_copy(&mut self, id: NodeId) -> NodeId {
        let node = self.nodes[id.0].clone();
        let copy_properties = |properties: Vec<Property>, tree: &mut Self| -> Vec<Property> {
            properties
                .into_iter()
                .map(|property| Property {
                    values: property
                        .values
                        .iter()
                        .map(|&value| tree.deep_copy(value))
                        .collect(),
                    ..property
                })
                .collect()
        };
        let data = match node.data {
            Data::Primitive(value, properties) => {
                Data::Primitive(value, copy_properties(properties, self))
            }
            Data::Complex(properties) => Data::Complex(copy_properties(properties, self)),
        };
        self.push(Node { data, ..node })
    }

    /// Sets a primitive child, such as a resource's `id`.
    pub fn set_text(
        &mut self,
        id: NodeId,
        name: &str,
        text: &str,
        types: &Types<'_>,
    ) -> Result<(), String> {
        let value = self.primitive(
            Value::String(text.to_owned()),
            Type::Unknown,
            Some("string".to_owned()),
        );
        self.set_property(id, name, value, types)
    }

    /// The nodes reachable from `root`, `root` included.
    pub fn reachable(&self, root: NodeId) -> std::collections::HashSet<NodeId> {
        let mut seen = std::collections::HashSet::new();
        let mut stack = vec![root];
        while let Some(node) = stack.pop() {
            if seen.insert(node) {
                for property in self.properties(node) {
                    stack.extend(&property.values);
                }
            }
        }
        seen
    }

    // ---- Output -----------------------------------------------------------------------------

    /// The node as FHIR JSON, elements in definition order.
    pub fn to_json(&self, id: NodeId) -> Value {
        let node = &self.nodes[id.0];
        match &node.data {
            Data::Primitive(value, _) => value.clone(),
            Data::Complex(properties) => {
                let mut object = Map::new();
                if (node.resource || node.ty.is_resource())
                    && let Some(name) = self.fhir_type(id)
                {
                    object.insert("resourceType".to_owned(), Value::String(name));
                }
                self.write_properties(properties, &mut object);
                Value::Object(object)
            }
        }
    }

    /// A primitive's `id` and `extension` as the `_name` object, if it has any.
    fn primitive_element(&self, id: NodeId) -> Option<Value> {
        match &self.nodes[id.0].data {
            Data::Primitive(_, properties) if !properties.is_empty() => {
                let mut object = Map::new();
                self.write_properties(properties, &mut object);
                (!object.is_empty()).then_some(Value::Object(object))
            }
            _ => None,
        }
    }

    fn write_properties(&self, properties: &[Property], object: &mut Map<String, Value>) {
        let mut ordered: Vec<&Property> = properties.iter().collect();
        ordered.sort_by_key(|p| p.index.unwrap_or(usize::MAX));
        for property in ordered {
            let values: Vec<(NodeId, Value, Option<Value>)> = property
                .values
                .iter()
                .map(|&value| (value, self.to_json(value), self.primitive_element(value)))
                .filter(|(_, json, element)| !is_empty(json) || element.is_some())
                .collect();
            if values.is_empty() {
                continue;
            }
            let mut put = |key: String, json: Value, element: Option<Value>| {
                if !json.is_null() {
                    object.insert(key.clone(), json);
                }
                if let Some(element) = element {
                    object.insert(format!("_{key}"), element);
                }
            };
            if property.choice {
                for (value, json, element) in values {
                    let suffix = self
                        .fhir_type(value)
                        .map(|t| capitalize(&t))
                        .unwrap_or_default();
                    put(format!("{}{suffix}", property.name), json, element);
                }
            } else if property.many {
                let any_element = values.iter().any(|(_, _, e)| e.is_some());
                let any_value = values.iter().any(|(_, json, _)| !json.is_null());
                let (jsons, elements): (Vec<Value>, Vec<Value>) = values
                    .into_iter()
                    .map(|(_, json, element)| (json, element.unwrap_or(Value::Null)))
                    .unzip();
                if any_value {
                    object.insert(property.name.clone(), Value::Array(jsons));
                }
                if any_element {
                    object.insert(format!("_{}", property.name), Value::Array(elements));
                }
            } else {
                let (_, json, element) = values.into_iter().last().expect("not empty");
                put(property.name.clone(), json, element);
            }
        }
    }
}

/// Values left out of the output: unassigned primitives and elements with no content.
fn is_empty(json: &Value) -> bool {
    match json {
        Value::Null => true,
        Value::Object(map) => map.is_empty(),
        Value::Array(items) => items.is_empty(),
        _ => false,
    }
}

pub(crate) fn capitalize(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}
