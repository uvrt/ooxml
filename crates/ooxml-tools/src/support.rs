//! What the code `#[derive(ToolArgs)]` writes calls: one property built from a field's type
//! and attributes ([`Field`]), and the fields of a validated object read ([`field`]).
//!
//! Owns: where each field attribute lands in the schema (a bound on a scalar, or on an
//! array's innermost items) and the definition problems of an attribute that does not fit
//! the field's type (returned by [`Field::build`], reported by [`crate::ToolArgs::problems`]).
//!
//! Must not: be called by hand; it is public for the derive's generated code only.

use crate::schema::{Bound, Format, Kind, Property, Schema};
use crate::types::{ArgError, ArgType};
use serde_json::{Map, Value};

/// One property being built from a field.
#[derive(Debug)]
pub struct Field {
    property: Property,
    problems: Vec<String>,
}

impl Field {
    /// The property `name` of type `T`, required unless `T` is an `Option`.
    #[must_use]
    pub fn of<T: ArgType>(name: &str) -> Self {
        let property = Property {
            name: name.to_owned(),
            schema: T::schema(),
            required: !T::OPTIONAL,
            intent: crate::schema::Intent::default(),
        };
        Self {
            property,
            problems: Vec::new(),
        }
    }

    /// The description (the field's doc comment).
    #[must_use]
    pub fn describe(mut self, text: &str) -> Self {
        self.property.schema.description = Some(text.to_owned());
        self
    }

    fn scalar(&mut self) -> &mut Schema {
        innermost(&mut self.property.schema)
    }

    fn misfit(&mut self, attribute: &str) {
        let kind = self.scalar().type_name();
        self.problems.push(format!(
            "{}: {attribute} does not apply to a {kind}",
            self.property.name
        ));
    }

    /// `minimum` (on a number, or an array of numbers' items).
    #[must_use]
    pub fn minimum(mut self, bound: Bound) -> Self {
        match &mut self.scalar().kind {
            Kind::Integer(node) | Kind::Number(node) => node.minimum = Some(bound),
            _ => self.misfit("minimum"),
        }
        self
    }

    /// `maximum`.
    #[must_use]
    pub fn maximum(mut self, bound: Bound) -> Self {
        match &mut self.scalar().kind {
            Kind::Integer(node) | Kind::Number(node) => node.maximum = Some(bound),
            _ => self.misfit("maximum"),
        }
        self
    }

    /// `minLength`.
    #[must_use]
    pub fn min_length(mut self, length: u64) -> Self {
        match &mut self.scalar().kind {
            Kind::String(node) => node.min_length = Some(length),
            _ => self.misfit("min_length"),
        }
        self
    }

    /// `maxLength`.
    #[must_use]
    pub fn max_length(mut self, length: u64) -> Self {
        match &mut self.scalar().kind {
            Kind::String(node) => node.max_length = Some(length),
            _ => self.misfit("max_length"),
        }
        self
    }

    /// `format`.
    #[must_use]
    pub fn format(mut self, format: Format) -> Self {
        match &mut self.scalar().kind {
            Kind::String(node) => node.format = Some(format),
            _ => self.misfit("format"),
        }
        self
    }

    /// A value read as one of the enum's (`valueAliases`).
    #[must_use]
    pub fn value_alias(mut self, alias: &str, value: &str) -> Self {
        match &mut self.scalar().kind {
            Kind::String(node) => node.value_aliases.push((alias.to_owned(), value.to_owned())),
            _ => self.misfit("value_alias"),
        }
        self
    }

    fn array(&mut self, attribute: &str) -> Option<&mut crate::schema::ArrayNode> {
        if !matches!(self.property.schema.kind, Kind::Array(_)) {
            let kind = self.property.schema.type_name();
            self.problems.push(format!(
                "{}: {attribute} does not apply to a {kind}",
                self.property.name
            ));
        }
        match &mut self.property.schema.kind {
            Kind::Array(array) => Some(array),
            _ => None,
        }
    }

    /// `minItems` (0 or 1).
    #[must_use]
    pub fn min_items(mut self, count: u8) -> Self {
        if let Some(array) = self.array("min_items") {
            array.min_items = Some(count);
        }
        self
    }

    /// `maxItems`.
    #[must_use]
    pub fn max_items(mut self, count: u64) -> Self {
        if let Some(array) = self.array("max_items") {
            array.max_items = Some(count);
        }
        self
    }

    /// `single_as_list` (`orSingle`).
    #[must_use]
    pub fn single_as_list(mut self) -> Self {
        if let Some(array) = self.array("single_as_list") {
            array.single_as_list = true;
        }
        self
    }

    /// `map_as_pairs` (`orMap`).
    #[must_use]
    pub fn map_as_pairs(mut self) -> Self {
        if let Some(array) = self.array("map_as_pairs") {
            array.map_as_pairs = true;
        }
        self
    }

    /// `empty_is_absent` (an optional field only).
    #[must_use]
    pub fn empty_is_absent(mut self) -> Self {
        if self.property.required {
            self.problems
                .push(format!("{}: empty_is_absent on a required field", self.property.name));
        }
        self.property.intent.empty_is_absent = true;
        self
    }

    /// An argument name read as this one.
    #[must_use]
    pub fn alias(mut self, alias: &str) -> Self {
        self.property.intent.aliases.push(alias.to_owned());
        self
    }

    /// The argument a list of names given here means (today's `names_as`).
    #[must_use]
    pub fn names_as(mut self, name: &str) -> Self {
        self.property.intent.names_as = Some(name.to_owned());
        self
    }

    /// The value a required field left out is read as.
    #[must_use]
    pub fn default(mut self, value: Value) -> Self {
        if !self.property.required {
            self.problems
                .push(format!("{}: default on an optional field", self.property.name));
        }
        self.property.intent.default = Some(value);
        self
    }

    /// The document kinds the field applies to (an optional field only).
    #[must_use]
    pub fn kinds(mut self, kinds: &[&str]) -> Self {
        if self.property.required {
            self.problems
                .push(format!("{}: kinds on a required field", self.property.name));
        }
        self.property.intent.kinds = kinds.iter().map(|k| (*k).to_owned()).collect();
        self
    }

    /// A value that asks nothing of a kind the field does not apply to.
    #[must_use]
    pub fn kind_neutral(mut self, value: Value) -> Self {
        self.property.intent.kind_neutral.push(value);
        self
    }

    /// An enum value that applies to some kinds only.
    #[must_use]
    pub fn kind_value(mut self, value: &str, kinds: &[&str]) -> Self {
        let kinds = kinds.iter().map(|k| (*k).to_owned()).collect();
        self.property.intent.kind_values.push((value.to_owned(), kinds));
        self
    }

    /// The description for a tool serving `kind` only.
    #[must_use]
    pub fn kind_description(mut self, kind: &str, text: &str) -> Self {
        self.property
            .intent
            .kind_descriptions
            .push((kind.to_owned(), text.to_owned()));
        self
    }

    /// The field's strings may be refs.
    #[must_use]
    pub fn refs(mut self) -> Self {
        self.property.intent.holds.refs = true;
        self
    }

    /// The field's strings may name datasets.
    #[must_use]
    pub fn datasets(mut self) -> Self {
        self.property.intent.holds.datasets = true;
        self
    }

    /// The field's strings may name sources.
    #[must_use]
    pub fn sources(mut self) -> Self {
        self.property.intent.holds.sources = true;
        self
    }

    /// The field's text may hold references to one value.
    #[must_use]
    pub fn facts(mut self) -> Self {
        self.property.intent.holds.facts = true;
        self
    }

    /// The property, and the problems of attributes that do not fit it.
    #[must_use]
    pub fn build(self) -> (Property, Vec<String>) {
        (self.property, self.problems)
    }
}

/// The innermost items of an array (of arrays), or the node itself.
fn innermost(schema: &mut Schema) -> &mut Schema {
    match schema.kind {
        Kind::Array(ref mut array) => innermost(&mut array.items),
        _ => schema,
    }
}

/// The object of a validated value.
///
/// # Errors
/// [`ArgError`] when it is not an object.
pub fn object<'a>(value: &'a Value, at: &str) -> Result<&'a Map<String, Value>, ArgError> {
    value.as_object().ok_or_else(|| ArgError::new(at, "an object"))
}

/// The field `name` of a validated object, read as `T` (absent: `T`'s absent value).
///
/// # Errors
/// [`ArgError`] when it does not read as `T`.
pub fn field<T: ArgType>(map: &Map<String, Value>, name: &str, at: &str) -> Result<T, ArgError> {
    let path = crate::validate::join(at, name);
    match map.get(name) {
        Some(value) => T::from_arg(value, &path),
        None => T::absent(&path),
    }
}
