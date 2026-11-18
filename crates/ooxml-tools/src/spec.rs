//! A tool's definition: [`ToolSpec`], made by `#[derive(ToolArgs)]` from one Rust struct.
//!
//! Owns: the tool's name, description, group and flags; its arguments' schema with each
//! property's intent rules; the views today's Python `Tool` derives from its declarations
//! (the paths of refs, datasets, sources and facts; argument aliases; defaults; the kind
//! rules), the canonical schema for the document kinds a toolbox serves, the definition's
//! own checks ([`ToolSpec::problems`]) and its export for the Python checker.
//!
//! Must not: hold handlers, know providers, or dispatch calls (the session does, lane P0d).

use crate::places::Places;
use crate::schema::{Kind, ObjectNode, Property, Schema, StringNode};
use serde_json::{Map, Value};

/// What a tool does to its documents and where it may run. Defaults as today's `@tool`:
/// strict, batchable, a retry key, not changing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(clippy::struct_excessive_bools)] // each is a declared, independent fact of the tool
pub struct Flags {
    /// Whether a call changes its document.
    pub mutates: bool,
    /// Whether it is sent in strict mode.
    pub strict: bool,
    /// Whether it may run inside `batch`.
    pub batchable: bool,
    /// Whether a tool `batch` refuses may run inside `read_batch`.
    pub read_batchable: bool,
    /// Whether a changing call's `key` argument is a retry key.
    pub retry_key: bool,
}

impl Default for Flags {
    fn default() -> Self {
        Self {
            mutates: false,
            strict: true,
            batchable: true,
            read_batchable: false,
            retry_key: true,
        }
    }
}

/// One tool's definition.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolSpec {
    /// The name the model calls.
    pub name: String,
    /// The description (a Markdown file beside the tool).
    pub description: String,
    /// The group it is loaded with (`core`: always loaded).
    pub group: String,
    /// What it does and where it may run.
    pub flags: Flags,
    /// The role a call needs; `None`: the default role of a changing or a reading tool.
    pub role: Option<String>,
    /// Order for strict mode on Claude: lower first.
    pub priority: Option<i64>,
    /// For a tool that names no document: the argument whose value is the document kind.
    pub route: Option<String>,
    /// The arguments that name documents, in order.
    pub documents: Vec<String>,
    /// Sets of optional arguments of which exactly one must be given.
    pub exactly_one: Vec<Vec<String>>,
    /// The arguments' schema.
    pub arguments: ObjectNode,
    /// A placing tool's place: the names other placing tools give a place, read as its own
    /// ([`crate::places`]).
    pub places: Option<Places>,
}

/// Which strings a path list is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Holding {
    /// Refs.
    Refs,
    /// Datasets.
    Datasets,
    /// Sources.
    Sources,
    /// Facts.
    Facts,
}

/// An argument alias: in the object at `path` (`""`: the arguments; `find[]`: each item of
/// `find`), `alias` is read as `name`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Alias {
    /// Where the object is.
    pub path: String,
    /// The name a model writes.
    pub alias: String,
    /// The tool's own name.
    pub name: String,
}

impl Alias {
    /// The alias as today's `Tool.aliases` key writes it (`find[].range`).
    #[must_use]
    pub fn key(&self) -> String {
        crate::validate::join(&self.path, &self.alias)
    }
}

impl ToolSpec {
    /// A tool in the core group, with today's default flags, editing `doc`.
    #[must_use]
    pub fn new(name: &str, description: &str, arguments: Schema) -> Self {
        let arguments = match arguments.kind {
            Kind::Object(object) => object,
            _ => ObjectNode::default(),
        };
        Self {
            name: name.to_owned(),
            description: description.trim_end().to_owned(),
            group: "core".to_owned(),
            flags: Flags::default(),
            role: None,
            priority: None,
            route: None,
            documents: vec!["doc".to_owned()],
            exactly_one: Vec::new(),
            arguments,
            places: None,
        }
    }

    /// The arguments' schema as one node.
    #[must_use]
    pub fn schema(&self) -> Schema {
        Schema::new(Kind::Object(self.arguments.clone()))
    }

    /// The paths (`items[].target`, `targets[]`) of the string arguments that hold `what`,
    /// depth first in field order.
    #[must_use]
    pub fn paths(&self, what: Holding) -> Vec<String> {
        let mut found = Vec::new();
        holding(&self.arguments, "", what, &mut found);
        found
    }

    /// The argument aliases: the arguments' own first, then each nested object's, level
    /// by level, each in field order.
    #[must_use]
    pub fn aliases(&self) -> Vec<Alias> {
        let mut found = Vec::new();
        let mut level: Vec<(&ObjectNode, String)> = vec![(&self.arguments, String::new())];
        while !level.is_empty() {
            let mut next = Vec::new();
            for (object, path) in level {
                for property in object.properties() {
                    for alias in &property.intent.aliases {
                        found.push(Alias {
                            path: path.clone(),
                            alias: alias.clone(),
                            name: property.name.clone(),
                        });
                    }
                    if let Some((child, child_path)) = nested_object(property, &path) {
                        next.push((child, child_path));
                    }
                }
            }
            level = next;
        }
        found
    }

    /// The required arguments read as a safe value when left out: `(name, value)`.
    #[must_use]
    pub fn defaults(&self) -> Vec<(&str, &Value)> {
        let properties = self.arguments.properties().iter();
        properties
            .filter_map(|p| p.intent.default.as_ref().map(|v| (p.name.as_str(), v)))
            .collect()
    }

    /// The arguments that apply to some document kinds only: `(name, kinds)`.
    #[must_use]
    pub fn kind_fields(&self) -> Vec<(&str, &[String])> {
        let properties = self
            .arguments
            .properties()
            .iter()
            .filter(|p| !p.intent.kinds.is_empty());
        properties
            .map(|p| (p.name.as_str(), p.intent.kinds.as_slice()))
            .collect()
    }

    /// The canonical schema for a toolbox whose handlers serve `served` kinds (`None`:
    /// every kind): without the arguments and enum values of kinds it serves none of, a
    /// one-kind tool's arguments described for its kind.
    #[must_use]
    pub fn canonical_for(&self, served: Option<&[&str]>) -> Value {
        let Some(served) = served else {
            return self.schema().canonical();
        };
        let serves = |kinds: &[String]| kinds.iter().any(|k| served.contains(&k.as_str()));
        let mut arguments = self.arguments.clone();
        for property in self.arguments.properties() {
            let intent = &property.intent;
            if !intent.kinds.is_empty() && !serves(&intent.kinds) {
                arguments.remove(&property.name);
                continue;
            }
            let Some(sent) = arguments.property_mut(&property.name) else {
                continue;
            };
            if let Some(node) = enum_holder(&mut sent.schema) {
                node.values.retain(|value| {
                    intent
                        .kind_values
                        .iter()
                        .find(|(v, _)| v == value)
                        .is_none_or(|(_, kinds)| serves(kinds))
                });
            }
            if let [kind] = served {
                if let Some((_, text)) = intent.kind_descriptions.iter().find(|(k, _)| k == kind) {
                    sent.schema.description = Some(text.clone());
                }
            }
        }
        Schema::new(Kind::Object(arguments)).canonical()
    }

    /// The definition as the Python checker and the equality test read it: every
    /// declaration today's `Tool` has, the validation schema, and the canonical schema for
    /// `served` kinds.
    #[must_use]
    pub fn export(&self, served: Option<&[&str]>) -> Value {
        let strings = |items: &[String]| Value::Array(items.iter().map(|s| Value::from(s.as_str())).collect());
        let mut out = Map::new();
        out.insert("name".into(), Value::from(self.name.as_str()));
        out.insert("description".into(), Value::from(self.description.as_str()));
        out.insert("group".into(), Value::from(self.group.as_str()));
        out.insert("mutates".into(), Value::Bool(self.flags.mutates));
        out.insert("strict".into(), Value::Bool(self.flags.strict));
        out.insert("batchable".into(), Value::Bool(self.flags.batchable));
        out.insert("read_batchable".into(), Value::Bool(self.flags.read_batchable));
        out.insert("retry_key".into(), Value::Bool(self.flags.retry_key));
        out.insert("role".into(), self.role.as_deref().map_or(Value::Null, Value::from));
        out.insert("priority".into(), self.priority.map_or(Value::Null, Value::from));
        out.insert("route".into(), self.route.as_deref().map_or(Value::Null, Value::from));
        out.insert("documents".into(), strings(&self.documents));
        out.insert(
            "exactly_one".into(),
            self.exactly_one.iter().map(|g| strings(g)).collect(),
        );
        for (key, what) in [
            ("refs", Holding::Refs),
            ("datasets", Holding::Datasets),
            ("sources", Holding::Sources),
            ("facts", Holding::Facts),
        ] {
            out.insert(key.into(), strings(&self.paths(what)));
        }
        let mut aliases: Map<String, Value> = self
            .aliases()
            .into_iter()
            .map(|a| (a.key(), Value::from(a.name)))
            .collect();
        if let Some(places) = &self.places {
            // a function in today's `Tool.aliases`: written as the kind of place it reads
            let read = Value::from(format!("place:{}", places.kind.name()));
            aliases.extend(places.names.iter().map(|name| (name.clone(), read.clone())));
        }
        out.insert("aliases".into(), Value::Object(aliases));
        let defaults = self.defaults().into_iter().map(|(n, v)| (n.to_owned(), v.clone()));
        out.insert("defaults".into(), Value::Object(defaults.collect()));
        self.export_kinds(&mut out);
        out.insert("schema".into(), self.schema().validation());
        out.insert("canonical".into(), self.canonical_for(served));
        Value::Object(out)
    }

    fn export_kinds(&self, out: &mut Map<String, Value>) {
        let strings = |items: &[String]| Value::Array(items.iter().map(|s| Value::from(s.as_str())).collect());
        let properties = self.arguments.properties();
        let fields = properties.iter().filter(|p| !p.intent.kinds.is_empty());
        out.insert(
            "kind_fields".into(),
            fields.map(|p| (p.name.clone(), strings(&p.intent.kinds))).collect(),
        );
        let neutral = properties.iter().filter(|p| !p.intent.kind_neutral.is_empty());
        out.insert(
            "kind_neutral".into(),
            neutral
                .map(|p| (p.name.clone(), Value::Array(p.intent.kind_neutral.clone())))
                .collect(),
        );
        let values = properties.iter().filter(|p| !p.intent.kind_values.is_empty()).map(|p| {
            let map: Map<String, Value> = p
                .intent
                .kind_values
                .iter()
                .map(|(v, k)| (v.clone(), strings(k)))
                .collect();
            (p.name.clone(), Value::Object(map))
        });
        out.insert("kind_values".into(), values.collect());
        let texts = properties
            .iter()
            .filter(|p| !p.intent.kind_descriptions.is_empty())
            .map(|p| {
                let map: Map<String, Value> = p
                    .intent
                    .kind_descriptions
                    .iter()
                    .map(|(k, t)| (k.clone(), Value::from(t.as_str())))
                    .collect();
                (p.name.clone(), Value::Object(map))
            });
        out.insert("kind_descriptions".into(), texts.collect());
    }
}

/// The object a property holds (itself, or its array's items), with the path an alias
/// inside it is written under.
fn nested_object<'a>(property: &'a Property, path: &str) -> Option<(&'a ObjectNode, String)> {
    let name = crate::validate::join(path, &property.name);
    match &property.schema.kind {
        Kind::Object(object) => Some((object, name)),
        Kind::Array(array) => array.items.as_object().map(|object| (object, format!("{name}[]"))),
        _ => None,
    }
}

fn holding(object: &ObjectNode, prefix: &str, what: Holding, found: &mut Vec<String>) {
    for property in object.properties() {
        let mut path = crate::validate::join(prefix, &property.name);
        let mut node = &property.schema;
        if let Kind::Array(array) = &node.kind {
            path.push_str("[]");
            node = &array.items;
            while let Kind::Array(inner) = &node.kind {
                node = &inner.items;
            }
        }
        match &node.kind {
            Kind::String(_) if holds(property, what) => found.push(path),
            Kind::Object(child) => holding(child, &path, what, found),
            _ => {}
        }
    }
}

fn holds(property: &Property, what: Holding) -> bool {
    let holds = property.intent.holds;
    match what {
        Holding::Refs => holds.refs,
        Holding::Datasets => holds.datasets,
        Holding::Sources => holds.sources,
        Holding::Facts => holds.facts,
    }
}

/// The node holding a property's enum: its own, or its items'.
fn enum_holder(schema: &mut Schema) -> Option<&mut StringNode> {
    let kind = match schema.kind {
        Kind::Array(ref mut array) => &mut array.items.kind,
        ref mut other => other,
    };
    match kind {
        Kind::String(node) if !node.values.is_empty() => Some(node),
        _ => None,
    }
}
