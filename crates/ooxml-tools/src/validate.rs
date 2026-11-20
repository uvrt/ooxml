//! A call's arguments checked against its schema and normalised, as today's Python
//! validator does it (`schema.py`, `validate_call`), with the same messages.
//!
//! Owns: the walk over the arguments (unknown names with "did you mean" and "belongs one
//! level up/down", required ones, types, enums, bounds, formats), the schema-level intent
//! rules applied on the way (`orSingle`, `orMap`, `orInteger`, `orNumber`, `orNull`,
//! `valueAliases`, a `null` optional read as absent, `3.0` read as `3`, and the declared
//! `empty_is_absent`), the notes they add, and for each problem the way out
//! ([`CallError::retry`]: the options, or a value that would do at the field).
//!
//! Must not: know tool names, aliases of argument names, defaults or document kinds
//! ([`crate::intent`] applies those around this walk).

use crate::difflib::{close_match, fold};
use crate::pyfmt::{number, repr_str};
use crate::schema::{ArrayNode, Bound, Format, Kind, NumberNode, ObjectNode, Schema, StringNode};
use serde_json::{Map, Value};

/// What is wrong with a call; every refusal site of the tool contract is one of these, so
/// a test can walk them all ([`Problem::ALL`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Problem {
    /// The arguments are not JSON.
    NotJson,
    /// An object argument is not an object.
    NotObject,
    /// A name the schema does not have.
    UnknownArgument,
    /// A required argument is missing.
    Required,
    /// An argument and its alias both given.
    AliasTwice,
    /// Not exactly one of a group given.
    NotExactlyOne,
    /// Not an array.
    NotArray,
    /// Fewer items than `minItems`.
    TooFewItems,
    /// More items than `maxItems`.
    TooManyItems,
    /// Not a string (nor a number where one may stand in).
    NotString,
    /// Not one of the enum's values.
    NotInEnum,
    /// Shorter than `minLength`.
    TooShort,
    /// Longer than `maxLength`.
    TooLong,
    /// Not a date or date-time of the format.
    NotFormat,
    /// Not `true` or `false`.
    NotBoolean,
    /// Not a number.
    NotNumber,
    /// Not a whole number.
    NotWhole,
    /// Below `minimum`.
    BelowMinimum,
    /// Above `maximum`.
    AboveMaximum,
}

impl Problem {
    /// Every problem.
    pub const ALL: [Problem; 19] = [
        Self::NotJson,
        Self::NotObject,
        Self::UnknownArgument,
        Self::Required,
        Self::AliasTwice,
        Self::NotExactlyOne,
        Self::NotArray,
        Self::TooFewItems,
        Self::TooManyItems,
        Self::NotString,
        Self::NotInEnum,
        Self::TooShort,
        Self::TooLong,
        Self::NotFormat,
        Self::NotBoolean,
        Self::NotNumber,
        Self::NotWhole,
        Self::BelowMinimum,
        Self::AboveMaximum,
    ];
}

/// The way out of a problem.
#[derive(Debug, Clone, PartialEq)]
pub enum Retry {
    /// The values or names to choose from.
    Options(Vec<Value>),
    /// A value at the field that would pass: `fill` when it only shows the kind wanted.
    Value {
        /// The value.
        value: Value,
        /// Whether the caller must replace it with its own.
        fill: bool,
    },
    /// The whole arguments, every required one shown by an example.
    Example,
}

/// A call that does not match its schema.
#[derive(Debug, Clone, PartialEq)]
pub struct CallError {
    /// What is wrong.
    pub problem: Problem,
    /// The message, as Python writes it.
    pub message: String,
    /// The dotted path of the argument (`box.x`, `cells[2].text`); `None` for the whole.
    pub field: Option<String>,
    /// The way out.
    pub retry: Retry,
}

impl CallError {
    pub(crate) fn new(problem: Problem, message: String, field: &str, retry: Retry) -> Box<Self> {
        let field = (!field.is_empty()).then(|| field.to_owned());
        Box::new(Self {
            problem,
            message,
            field,
            retry,
        })
    }
}

type Outer<'a> = Option<(&'a ObjectNode, &'a str)>;

/// `arguments` checked against the object `root` and normalised; `notes` gets a line for
/// each value read as another.
///
/// # Errors
/// The first problem found, as [`CallError`].
pub fn validate(root: &ObjectNode, arguments: &Value, notes: &mut Vec<String>) -> Result<Value, Box<CallError>> {
    if !arguments.is_object() {
        return Err(CallError::new(
            Problem::NotObject,
            "the arguments must be a JSON object".into(),
            "",
            Retry::Example,
        ));
    }
    object(root, arguments, "", notes, None)
}

fn label(where_: &str) -> &str {
    if where_.is_empty() {
        "the arguments"
    } else {
        where_
    }
}

pub(crate) fn join(where_: &str, name: &str) -> String {
    if where_.is_empty() {
        name.to_owned()
    } else {
        format!("{where_}.{name}")
    }
}

fn check(
    schema: &Schema,
    value: &Value,
    where_: &str,
    notes: &mut Vec<String>,
    outer: Outer,
) -> Result<Value, Box<CallError>> {
    if value.is_null() && schema.or_null {
        return Ok(Value::Null);
    }
    match &schema.kind {
        Kind::Object(node) => object(node, value, where_, notes, outer),
        Kind::Free => match value {
            Value::Object(_) => Ok(value.clone()),
            _ => Err(type_error(Problem::NotObject, "must be an object", schema, where_)),
        },
        Kind::Array(node) => array(node, schema, value, where_, notes, outer),
        Kind::String(node) => string(node, schema, value, where_, notes),
        Kind::Boolean => match value {
            Value::Bool(_) => Ok(value.clone()),
            _ => Err(type_error(Problem::NotBoolean, "must be true or false", schema, where_)),
        },
        Kind::Integer(node) => numeric(node, true, schema, value, where_),
        Kind::Number(node) => numeric(node, false, schema, value, where_),
    }
}

fn type_error(problem: Problem, what: &str, schema: &Schema, where_: &str) -> Box<CallError> {
    let retry = Retry::Value {
        value: example(schema, where_),
        fill: true,
    };
    CallError::new(problem, format!("{} {what}", label(where_)), where_, retry)
}

fn object(
    node: &ObjectNode,
    value: &Value,
    where_: &str,
    notes: &mut Vec<String>,
    outer: Outer,
) -> Result<Value, Box<CallError>> {
    let Value::Object(given) = value else {
        let schema = Schema::new(Kind::Object(node.clone()));
        return Err(type_error(Problem::NotObject, "must be an object", &schema, where_));
    };
    if let Some(unknown) = given.keys().find(|key| node.property(key).is_none()) {
        let name = join(where_, unknown);
        let names: Vec<&str> = node.properties().iter().map(|p| p.name.as_str()).collect();
        let refused = node
            .properties()
            .iter()
            .find(|p| p.intent.refused_aliases.iter().any(|a| a == unknown))
            .map(|p| format!("; did you mean {}?", repr_str(&p.name)));
        let hint = refused
            .or_else(|| misplaced(unknown, node, where_, outer))
            .unwrap_or_else(|| did_you_mean(unknown, &names));
        let mut sorted: Vec<&str> = names.clone();
        sorted.sort_unstable();
        let options = sorted.into_iter().map(Value::from).collect();
        return Err(CallError::new(
            Problem::UnknownArgument,
            format!("unknown argument {}{hint}", repr_str(&name)),
            &name,
            Retry::Options(options),
        ));
    }
    let mut result = Map::new();
    for property in node.properties() {
        let path = join(where_, &property.name);
        let found = given.get(&property.name).filter(|v| !v.is_null());
        let found = match found {
            Some(v) if property.intent.empty_is_absent && !property.required && is_empty(v) => {
                notes.push(format!("{path} ignored: it is empty"));
                None
            }
            other => other,
        };
        let Some(found) = found else {
            if property.required {
                let retry = Retry::Value {
                    value: example(&property.schema, &path),
                    fill: true,
                };
                return Err(CallError::new(
                    Problem::Required,
                    format!("{} is required", repr_str(&path)),
                    &path,
                    retry,
                ));
            }
            continue;
        };
        let checked = check(&property.schema, found, &path, notes, Some((node, where_)))?;
        result.insert(property.name.clone(), checked);
    }
    Ok(Value::Object(result))
}

/// A value that asks for nothing: `false`, `""`, an empty array or object (not `0`, which
/// is a measure).
#[must_use]
pub fn is_empty(value: &Value) -> bool {
    match value {
        Value::Bool(b) => !b,
        Value::String(s) => s.is_empty(),
        Value::Array(items) => items.is_empty(),
        Value::Object(map) => map.is_empty(),
        Value::Null | Value::Number(_) => false,
    }
}

fn did_you_mean(value: &str, options: &[&str]) -> String {
    match close_match(value, options) {
        Some(found) if found != value => format!("; did you mean {}?", repr_str(&found)),
        _ => String::new(),
    }
}

/// Where an unknown `key` of the object at `where_` belongs when it is a property one
/// level up or one level down.
fn misplaced(key: &str, node: &ObjectNode, where_: &str, outer: Outer) -> Option<String> {
    let folded = fold(key);
    let named = |object: &ObjectNode| -> Option<String> {
        // Python's dict of folded names: the last name of a folded key wins.
        object
            .properties()
            .iter()
            .rev()
            .find(|p| fold(&p.name) == folded)
            .map(|p| p.name.clone())
    };
    if let Some((up, up_where)) = outer {
        if let Some(meant) = named(up) {
            let inner = if up_where.is_empty() {
                where_.to_owned()
            } else {
                where_
                    .get(up_where.len()..)
                    .unwrap_or("")
                    .trim_start_matches('.')
                    .to_owned()
            };
            let place = if up_where.is_empty() {
                "at the top level".to_owned()
            } else {
                format!("on {up_where}")
            };
            return Some(format!("; {meant} belongs one level up, {place}, not in {inner}"));
        }
    }
    let mut below = Vec::new();
    for property in node.properties() {
        let (child, many) = match &property.schema.kind {
            Kind::Array(array) => (&array.items, true),
            _ => (&property.schema, false),
        };
        if let Kind::Object(object) = &child.kind {
            if let Some(meant) = named(object) {
                below.push(format!(
                    "{}{}.{meant}",
                    join(where_, &property.name),
                    if many { "[]" } else { "" }
                ));
            }
        }
    }
    match below.as_slice() {
        [] => None,
        [one] => Some(format!("; {key} belongs one level down: {one}")),
        many => Some(format!(
            "; {key} belongs one level down, in one of: {}",
            many.join(", ")
        )),
    }
}

/// An `orMap` array's two item properties, in order.
fn pair_names(items: &Schema) -> Option<(&str, &str)> {
    let object = items.as_object()?;
    match object.properties() {
        [a, b] if a.required && b.required && a.schema.type_name() == "string" && b.schema.type_name() == "string" => {
            Some((a.name.as_str(), b.name.as_str()))
        }
        _ => None,
    }
}

fn array(
    node: &ArrayNode,
    schema: &Schema,
    value: &Value,
    where_: &str,
    notes: &mut Vec<String>,
    outer: Outer,
) -> Result<Value, Box<CallError>> {
    let mut value = value.clone();
    if node.map_as_pairs {
        if let (Value::Object(map), Some((first, second))) = (&value, pair_names(&node.items)) {
            let pairs: Vec<Value> = map
                .iter()
                .map(|(a, b)| {
                    let mut pair = Map::new();
                    pair.insert(first.to_owned(), Value::from(a.as_str()));
                    pair.insert(second.to_owned(), b.clone());
                    Value::Object(pair)
                })
                .collect();
            notes.push(format!(
                "{} given as an object: read as {} pair(s) {{{first}, {second}}}",
                label(where_),
                pairs.len()
            ));
            value = Value::Array(pairs);
        }
    }
    if node.single_as_list && !value.is_array() {
        // One item in the array's place: a one-item array, when it is an item.
        let mut ignored = Vec::new();
        if check(&node.items, &value, &format!("{where_}[0]"), &mut ignored, outer).is_ok() {
            value = Value::Array(vec![value]);
        }
    }
    let Value::Array(items) = &value else {
        return Err(type_error(Problem::NotArray, "must be an array", schema, where_));
    };
    if let Some(least) = node.min_items {
        if items.len() < usize::from(least) {
            let retry = Retry::Value {
                value: example(schema, where_),
                fill: true,
            };
            let message = format!("{} needs at least {least} item(s)", label(where_));
            return Err(CallError::new(Problem::TooFewItems, message, where_, retry));
        }
    }
    if let Some(most) = node.max_items {
        if items.len() as u64 > most {
            let kept: Vec<Value> = items
                .iter()
                .take(usize::try_from(most).unwrap_or(usize::MAX))
                .cloned()
                .collect();
            let retry = Retry::Value {
                value: Value::Array(kept),
                fill: true,
            };
            let message = format!("{} has {} items; at most {most}", label(where_), items.len());
            return Err(CallError::new(Problem::TooManyItems, message, where_, retry));
        }
    }
    let mut out = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        out.push(check(&node.items, item, &format!("{where_}[{index}]"), notes, outer)?);
    }
    Ok(Value::Array(out))
}

fn string(
    node: &StringNode,
    schema: &Schema,
    value: &Value,
    where_: &str,
    notes: &mut Vec<String>,
) -> Result<Value, Box<CallError>> {
    if let Value::Number(_) = value {
        if let Some(least) = node.or_integer {
            let whole = NumberNode {
                minimum: Some(Bound::Int(least)),
                maximum: None,
            };
            return numeric(&whole, true, &Schema::new(Kind::Integer(whole.clone())), value, where_);
        }
        if node.or_number {
            let any = NumberNode::default();
            return numeric(&any, false, &Schema::new(Kind::Number(any.clone())), value, where_);
        }
    }
    let Value::String(text) = value else {
        let what = if node.or_number {
            "must be a number or a string"
        } else if node.or_integer.is_some() {
            "must be a whole number or a string"
        } else {
            "must be a string"
        };
        return Err(type_error(Problem::NotString, what, schema, where_));
    };
    let mut text = text.clone();
    if !node.value_aliases.is_empty() && !node.values.contains(&text) {
        let lower = text.to_lowercase();
        // Python's dict of lower-cased aliases: the last of a folded alias wins.
        if let Some((_, meant)) = node
            .value_aliases
            .iter()
            .rev()
            .find(|(alias, _)| alias.to_lowercase() == lower)
        {
            notes.push(format!(
                "{} {} read as {}",
                label(where_),
                repr_str(&text),
                repr_str(meant)
            ));
            text.clone_from(meant);
        }
    }
    if !node.values.is_empty() && !node.values.contains(&text) {
        let options: Vec<&str> = node.values.iter().map(String::as_str).collect();
        let message = format!(
            "{} is {}; not one of the allowed values{}",
            label(where_),
            repr_str(&text),
            did_you_mean(&text, &options)
        );
        let retry = Retry::Options(node.values.iter().map(|v| Value::from(v.as_str())).collect());
        return Err(CallError::new(Problem::NotInEnum, message, where_, retry));
    }
    lengths_and_format(node, schema, &text, where_)?;
    Ok(Value::String(text))
}

/// A string's `minLength`, `maxLength` and `format`.
fn lengths_and_format(node: &StringNode, schema: &Schema, text: &str, where_: &str) -> Result<(), Box<CallError>> {
    let length = text.chars().count() as u64;
    if let Some(least) = node.min_length.filter(|least| length < *least) {
        let message = format!("{} must be at least {least} characters", label(where_));
        let retry = Retry::Value {
            value: example(schema, where_),
            fill: true,
        };
        return Err(CallError::new(Problem::TooShort, message, where_, retry));
    }
    if let Some(most) = node.max_length.filter(|most| length > *most) {
        let message = format!("{} is {length} characters; at most {most}", label(where_));
        let cut: String = text.chars().take(usize::try_from(most).unwrap_or(usize::MAX)).collect();
        let retry = Retry::Value {
            value: Value::String(cut),
            fill: true,
        };
        return Err(CallError::new(Problem::TooLong, message, where_, retry));
    }
    if let Some(format) = node.format {
        let (ok, what) = match format {
            Format::Date => (crate::dates::is_date(text), "must be a date, YYYY-MM-DD"),
            Format::DateTime => (
                crate::dates::is_date_time(text),
                "must be a date-time, YYYY-MM-DDThh:mm:ssZ",
            ),
        };
        if !ok {
            return Err(type_error(Problem::NotFormat, what, schema, where_));
        }
    }
    Ok(())
}

fn numeric(
    node: &NumberNode,
    whole: bool,
    schema: &Schema,
    value: &Value,
    where_: &str,
) -> Result<Value, Box<CallError>> {
    let Value::Number(n) = value else {
        return Err(type_error(Problem::NotNumber, "must be a number", schema, where_));
    };
    let mut n = n.clone();
    if whole && n.is_f64() {
        let float = n.as_f64().unwrap_or(f64::NAN);
        if float.fract() != 0.0 || !float.is_finite() {
            let retry = Retry::Value {
                value: Value::from(float.round()),
                fill: true,
            };
            let message = format!("{} must be a whole number", label(where_));
            return Err(CallError::new(Problem::NotWhole, message, where_, retry));
        }
        #[allow(clippy::cast_possible_truncation)] // whole and finite
        let int = float as i64;
        n = int.into();
    }
    let float = n.as_f64().unwrap_or(0.0);
    if let Some(least) = node.minimum.filter(|least| float < least.as_f64()) {
        let message = format!("{} is {}; at least {}", label(where_), number(&n), bound(least));
        let retry = Retry::Value {
            value: least.to_json(),
            fill: false,
        };
        return Err(CallError::new(Problem::BelowMinimum, message, where_, retry));
    }
    if let Some(most) = node.maximum.filter(|most| float > most.as_f64()) {
        let message = format!("{} is {}; at most {}", label(where_), number(&n), bound(most));
        let retry = Retry::Value {
            value: most.to_json(),
            fill: false,
        };
        return Err(CallError::new(Problem::AboveMaximum, message, where_, retry));
    }
    Ok(Value::Number(n))
}

fn bound(value: Bound) -> String {
    match value {
        Bound::Int(n) => n.to_string(),
        Bound::Float(f) => crate::pyfmt::repr_float(f),
    }
}

/// A value of the kind `schema` wants, to show in a call that would work: an enum's first
/// value, a number within the bounds, `false`, an array of one example item, an object of
/// its required properties, and for a free string `<path>`.
#[must_use]
pub fn example(schema: &Schema, where_: &str) -> Value {
    match &schema.kind {
        Kind::String(node) => match (node.values.first(), node.format) {
            (Some(first), _) => Value::from(first.as_str()),
            (None, Some(Format::Date)) => Value::from("2026-10-07"),
            (None, Some(Format::DateTime)) => Value::from("2026-10-07T09:00:00Z"),
            (None, None) => Value::from(format!("<{}>", if where_.is_empty() { "text" } else { where_ })),
        },
        Kind::Integer(node) | Kind::Number(node) => match (node.minimum, node.maximum) {
            (Some(least), _) => least.to_json(),
            (None, Some(most)) if most.as_f64() < 0.0 => most.to_json(),
            _ => Value::from(0),
        },
        Kind::Boolean => Value::Bool(false),
        Kind::Array(node) => Value::Array(vec![example(&node.items, &format!("{where_}[0]"))]),
        Kind::Object(node) => Value::Object(
            node.properties()
                .iter()
                .filter(|p| p.required)
                .map(|p| (p.name.clone(), example(&p.schema, &join(where_, &p.name))))
                .collect(),
        ),
        Kind::Free => Value::Object(Map::new()),
    }
}
