//! The schema of a tool's arguments, as typed nodes, and its two JSON forms.
//!
//! Owns: the node types of the common strict subset (string, integer, number, boolean,
//! array, object, and the free-form object of a tool never sent strict), the dispatcher-only
//! bound keywords beside them, each property's intent rules ([`Intent`]), and the JSON a
//! node is written as: the *validation* schema (with the bounds) and the *canonical* one
//! (without), key for key in the order today's Python registry writes them
//! (`python/ooxml-edit/src/ooxml_edit/tools/registry.py`, `string()` ... `build_schema()`).
//!
//! Must not: know any provider, any document format, or how a call is dispatched. The
//! subset's limits are checked by the Python checker on the exported schemas
//! (`tests/golden/`) and by the derive's own rules.

use serde_json::{Map, Number, Value};

/// A numeric bound or literal as written: a whole number stays one in the JSON (`1638`),
/// a fraction is written as a float (`0.06`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Bound {
    /// A whole number.
    Int(i64),
    /// A fraction.
    Float(f64),
}

impl Bound {
    /// The bound as a JSON number (a non-finite float, which no derive writes, as 0).
    #[must_use]
    pub fn to_json(self) -> Value {
        match self {
            Self::Int(n) => Value::from(n),
            Self::Float(f) => Number::from_f64(f).map_or(Value::from(0), Value::Number),
        }
    }

    /// The bound as a float, for comparing.
    #[must_use]
    #[allow(clippy::cast_precision_loss)] // bounds are small
    pub fn as_f64(self) -> f64 {
        match self {
            Self::Int(n) => n as f64,
            Self::Float(f) => f,
        }
    }
}

/// The `format` of a string: only the two both providers take.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// `YYYY-MM-DD`.
    Date,
    /// `YYYY-MM-DDThh:mm:ssZ`.
    DateTime,
}

impl Format {
    /// The keyword's value.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Date => "date",
            Self::DateTime => "date-time",
        }
    }
}

/// A string node.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct StringNode {
    /// The allowed values; empty: any string.
    pub values: Vec<String>,
    /// Values a model naturally writes for one of [`Self::values`] (`valueAliases`): read as
    /// it, ignoring case, with a note. Never sent.
    pub value_aliases: Vec<(String, String)>,
    /// `format`.
    pub format: Option<Format>,
    /// `minLength` (never sent).
    pub min_length: Option<u64>,
    /// `maxLength` (never sent).
    pub max_length: Option<u64>,
    /// `orInteger`: a whole number of at least this is taken in the string's place, kept a
    /// number. Never sent.
    pub or_integer: Option<i64>,
    /// `orNumber`: any finite number is taken in the string's place. Never sent.
    pub or_number: bool,
}

/// An integer or number node.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct NumberNode {
    /// `minimum` (never sent).
    pub minimum: Option<Bound>,
    /// `maximum` (never sent).
    pub maximum: Option<Bound>,
}

/// An array node.
#[derive(Debug, Clone, PartialEq)]
pub struct ArrayNode {
    /// The items' schema (described by the array's description).
    pub items: Schema,
    /// `minItems`: 0 or 1 only (sent).
    pub min_items: Option<u8>,
    /// `maxItems` (never sent).
    pub max_items: Option<u64>,
    /// `orSingle`: one item in the array's place is read as a one-item array.
    pub single_as_list: bool,
    /// `orMap`: an object `{a: b}` in the array's place is read as the pairs
    /// `[{first: a, second: b}]` (items an object of two required strings).
    pub map_as_pairs: bool,
}

/// One property of an object: its name, schema, whether it is required, and its intent
/// rules.
#[derive(Debug, Clone, PartialEq)]
pub struct Property {
    /// The name the model writes.
    pub name: String,
    /// Its schema.
    pub schema: Schema,
    /// Whether a call must give it.
    pub required: bool,
    /// What is read as it, and what it means for the dispatcher.
    pub intent: Intent,
}

/// What the dispatcher accepts for a property beyond its schema, and what the property
/// holds for it (refs, sources, facts). Declared once on the field; never sent.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Intent {
    /// Argument names read as this one, with a note (`top` for `space_before`).
    pub aliases: Vec<String>,
    /// For a property that takes no names (a count): the argument a list of names given
    /// here can only mean, unless that one is given too (`columns: ["region", "total"]`
    /// read as `from_columns`; today's `names_as`), with a note.
    pub names_as: Option<String>,
    /// For a required property: the safe value a call that leaves it out is read as, with
    /// a note. It stays required in the schema, so a strict decoder sends it.
    pub default: Option<Value>,
    /// An empty value (`""`, `[]`, `{}`, `false`; not `0`) is read as not given, with a note.
    pub empty_is_absent: bool,
    /// The document kinds the property applies to; empty: every kind.
    pub kinds: Vec<String>,
    /// Values that ask nothing of a kind the property does not apply to, besides the empty
    /// ones: ignored there with a note instead of reaching the handler.
    pub kind_neutral: Vec<Value>,
    /// Enum values that apply to some kinds only: `(value, kinds)`.
    pub kind_values: Vec<(String, Vec<String>)>,
    /// The description for a tool that serves one kind only: `(kind, description)`.
    pub kind_descriptions: Vec<(String, String)>,
    /// What the property's strings may name, for the dispatcher.
    pub holds: Holds,
}

/// What a string property (or an array of strings) may name.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Holds {
    /// A ref, `$name` or `$name/rest`.
    pub refs: bool,
    /// A dataset, `d:<id>`.
    pub datasets: bool,
    /// Rows to take: a dataset or an address in another open document.
    pub sources: bool,
    /// References to one value, `{{d:by_topic/door/mean}}`, in its text.
    pub facts: bool,
}

/// An object node: its properties, the required ones first, each part in the order given.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ObjectNode {
    properties: Vec<Property>,
}

impl ObjectNode {
    /// An object of `properties`, reordered as strict decoding needs: the required ones
    /// first, then the optional ones, each in the order given (an optional property listed
    /// before a required one is lost once a strict decoder writes the required one).
    #[must_use]
    pub fn new(properties: Vec<Property>) -> Self {
        let (mut required, optional): (Vec<Property>, Vec<Property>) = properties.into_iter().partition(|p| p.required);
        required.extend(optional);
        Self { properties: required }
    }

    /// The properties, in schema order.
    #[must_use]
    pub fn properties(&self) -> &[Property] {
        &self.properties
    }

    /// The property named `name`.
    #[must_use]
    pub fn property(&self, name: &str) -> Option<&Property> {
        self.properties.iter().find(|p| p.name == name)
    }

    /// The property named `name`, to change (the canonical form for some kinds).
    pub fn property_mut(&mut self, name: &str) -> Option<&mut Property> {
        self.properties.iter_mut().find(|p| p.name == name)
    }

    /// Removes the property named `name`.
    pub fn remove(&mut self, name: &str) {
        self.properties.retain(|p| p.name != name);
    }
}

/// What kind of node a schema is.
#[derive(Debug, Clone, PartialEq)]
pub enum Kind {
    /// `string`.
    String(StringNode),
    /// `integer`.
    Integer(NumberNode),
    /// `number`.
    Number(NumberNode),
    /// `boolean`.
    Boolean,
    /// `array`.
    Array(Box<ArrayNode>),
    /// `object` with named properties.
    Object(ObjectNode),
    /// A free-form object (`additionalProperties: true`), only in a tool never sent strict.
    Free,
}

/// One schema node.
#[derive(Debug, Clone, PartialEq)]
pub struct Schema {
    /// The description; nested nodes whose name says it all go without.
    pub description: Option<String>,
    /// The node's kind.
    pub kind: Kind,
    /// `orNull`: `null` is taken for it (an array's item), kept null. Never sent.
    pub or_null: bool,
}

impl Schema {
    /// A node of `kind`, no description.
    #[must_use]
    pub fn new(kind: Kind) -> Self {
        Self {
            description: None,
            kind,
            or_null: false,
        }
    }

    /// A plain string.
    #[must_use]
    pub fn string() -> Self {
        Self::new(Kind::String(StringNode::default()))
    }

    /// A string of one of `values`.
    #[must_use]
    pub fn one_of(values: &[&str]) -> Self {
        let values = values.iter().map(|v| (*v).to_owned()).collect();
        Self::new(Kind::String(StringNode {
            values,
            ..StringNode::default()
        }))
    }

    /// An array of `items`.
    #[must_use]
    pub fn array(items: Schema) -> Self {
        Self::new(Kind::Array(Box::new(ArrayNode {
            items,
            min_items: None,
            max_items: None,
            single_as_list: false,
            map_as_pairs: false,
        })))
    }

    /// The node's `type` keyword.
    #[must_use]
    pub fn type_name(&self) -> &'static str {
        match self.kind {
            Kind::String(_) => "string",
            Kind::Integer(_) => "integer",
            Kind::Number(_) => "number",
            Kind::Boolean => "boolean",
            Kind::Array(_) => "array",
            Kind::Object(_) | Kind::Free => "object",
        }
    }

    /// The object node, if this is one.
    #[must_use]
    pub fn as_object(&self) -> Option<&ObjectNode> {
        match &self.kind {
            Kind::Object(object) => Some(object),
            _ => None,
        }
    }

    /// The validation schema: what the dispatcher checks a call against.
    #[must_use]
    pub fn validation(&self) -> Value {
        self.to_json(true)
    }

    /// The canonical schema: what a provider is sent (no bound keywords).
    #[must_use]
    pub fn canonical(&self) -> Value {
        self.to_json(false)
    }

    fn to_json(&self, bounds: bool) -> Value {
        let mut out = Map::new();
        out.insert("type".into(), Value::from(self.type_name()));
        if let Some(text) = &self.description {
            out.insert("description".into(), Value::from(text.as_str()));
        }
        match &self.kind {
            Kind::String(node) => string_keys(node, bounds, &mut out),
            Kind::Integer(node) | Kind::Number(node) => {
                if bounds {
                    put(&mut out, "minimum", node.minimum.map(Bound::to_json));
                    put(&mut out, "maximum", node.maximum.map(Bound::to_json));
                }
            }
            Kind::Boolean => {}
            Kind::Array(node) => array_keys(node, bounds, &mut out),
            Kind::Object(node) => object_keys(node, bounds, &mut out),
            Kind::Free => {
                out.insert("additionalProperties".into(), Value::Bool(true));
            }
        }
        if bounds && self.or_null {
            out.insert("orNull".into(), Value::Bool(true));
        }
        Value::Object(out)
    }
}

fn put(out: &mut Map<String, Value>, key: &str, value: Option<Value>) {
    if let Some(value) = value {
        out.insert(key.into(), value);
    }
}

fn string_keys(node: &StringNode, bounds: bool, out: &mut Map<String, Value>) {
    if !node.values.is_empty() {
        out.insert(
            "enum".into(),
            node.values.iter().map(|v| Value::from(v.as_str())).collect(),
        );
    }
    if bounds && !node.value_aliases.is_empty() {
        let aliases = node
            .value_aliases
            .iter()
            .map(|(a, v)| (a.clone(), Value::from(v.as_str())));
        out.insert("valueAliases".into(), Value::Object(aliases.collect()));
    }
    put(out, "format", node.format.map(|f| Value::from(f.as_str())));
    if bounds {
        put(out, "minLength", node.min_length.map(Value::from));
        put(out, "maxLength", node.max_length.map(Value::from));
        put(out, "orInteger", node.or_integer.map(Value::from));
        put(out, "orNumber", node.or_number.then_some(Value::Bool(true)));
    }
}

fn array_keys(node: &ArrayNode, bounds: bool, out: &mut Map<String, Value>) {
    out.insert("items".into(), node.items.to_json(bounds));
    put(out, "minItems", node.min_items.map(Value::from));
    if bounds {
        put(out, "maxItems", node.max_items.map(Value::from));
        put(out, "orSingle", node.single_as_list.then_some(Value::Bool(true)));
        put(out, "orMap", node.map_as_pairs.then_some(Value::Bool(true)));
    }
}

fn object_keys(node: &ObjectNode, bounds: bool, out: &mut Map<String, Value>) {
    let properties = node
        .properties
        .iter()
        .map(|p| (p.name.clone(), p.schema.to_json(bounds)));
    out.insert("properties".into(), Value::Object(properties.collect()));
    let required: Vec<Value> = node
        .properties
        .iter()
        .filter(|p| p.required)
        .map(|p| Value::from(p.name.as_str()))
        .collect();
    if !required.is_empty() {
        // An empty list says nothing and would be sent with every request.
        out.insert("required".into(), Value::Array(required));
    }
    out.insert("additionalProperties".into(), Value::Bool(false));
}
