//! A call's arguments taken as meant: the tool's declared intent rules applied in today's
//! order (`dispatch.py`, `Toolbox._arguments`), then validation, then the kind rules
//! (`Tool.ignored_for`).
//!
//! Owns: [`ToolSpec::accept`] -- arguments as a JSON string or empty, argument aliases,
//! defaults of required arguments, an array or object given as a JSON string, validation
//! ([`crate::validate`]), exactly-one groups -- and [`ToolSpec::ignored_for`], the
//! kind-neutral rule; the notes each adds; and the refusal each problem becomes, with its
//! way out ([`crate::error::WayOut`]).
//!
//! Must not: resolve refs, datasets or sources, or know which documents are open (the
//! session does). Today's tool-name aliases and action hints need the toolbox: lane P0d.

use crate::error::{Refusal, ToolError, TryCall, WayOut};
use crate::places::Places;
use crate::schema::Kind;
use crate::spec::ToolSpec;
use crate::validate::{example, is_empty, validate, CallError, Problem, Retry};
use serde_json::{Map, Value};

/// A call's arguments as meant, with the notes on what was read as what.
#[derive(Debug, Clone, PartialEq)]
pub struct Accepted {
    /// The arguments, validated and normalised.
    pub arguments: Map<String, Value>,
    /// What was read as what, in today's order (aliases, defaults, then the walk's): the
    /// result's warnings start with these.
    pub notes: Vec<String>,
}

impl ToolSpec {
    /// `arguments` taken as meant and validated.
    ///
    /// # Errors
    /// `invalid_arguments`, with the options to choose from or the call that would work.
    pub fn accept(&self, arguments: Value) -> Result<Accepted, ToolError> {
        let mut notes = Vec::new();
        let arguments = match arguments {
            Value::Null => Value::Object(Map::new()),
            Value::String(text) if text.is_empty() => Value::Object(Map::new()),
            Value::String(text) => serde_json::from_str(&text).map_err(|e| {
                let message = format!("the arguments are not JSON: {e}");
                self.refuse(
                    &CallError::new(Problem::NotJson, message, "", Retry::Example),
                    &Map::new(),
                )
            })?,
            other => other,
        };
        let arguments = self.aliased(arguments, &mut notes)?;
        let arguments = self.defaulted(arguments, &mut notes);
        let arguments = self.json_strings(arguments);
        let given = arguments.as_object().cloned().unwrap_or_default();
        let checked = validate(&self.arguments, &arguments, &mut notes).map_err(|e| self.refuse(&e, &given))?;
        let Value::Object(checked) = checked else {
            return Err(self.refuse(&self.not_object(), &given));
        };
        self.exactly_one(&checked)?;
        Ok(Accepted {
            arguments: checked,
            notes,
        })
    }

    fn not_object(&self) -> Box<CallError> {
        CallError::new(
            Problem::NotObject,
            "the arguments must be a JSON object".into(),
            "",
            Retry::Example,
        )
    }

    /// Takes out of `arguments` the arguments that do not apply to a `kind` document and ask
    /// nothing of it -- an empty value, or one of the field's kind-neutral values -- and
    /// returns a note for each. Any other value is kept, for the handler to refuse.
    pub fn ignored_for(&self, kind: Option<&str>, arguments: &mut Map<String, Value>) -> Vec<String> {
        let Some(kind) = kind else { return Vec::new() };
        let mut notes = Vec::new();
        for property in self.arguments.properties() {
            let intent = &property.intent;
            if intent.kinds.is_empty() || intent.kinds.iter().any(|k| k == kind) {
                continue;
            }
            let Some(value) = arguments.get(&property.name) else {
                continue;
            };
            if !(is_empty(value) || intent.kind_neutral.iter().any(|n| same(n, value))) {
                continue;
            }
            arguments.shift_remove(&property.name);
            notes.push(format!(
                "{} ignored: it applies to {}, not to this {kind} document",
                property.name,
                intent.kinds.join(" and ")
            ));
        }
        notes
    }

    /// The refusal of a problem, with its way out: the options, or the call with a value
    /// at the field that would do.
    fn refuse(&self, error: &CallError, given: &Map<String, Value>) -> ToolError {
        let field = error.field.clone().unwrap_or_default();
        let call = |arguments: Map<String, Value>, fill: Vec<String>| TryCall {
            tool: self.name.clone(),
            arguments,
            fill,
        };
        let way_out = match &error.retry {
            Retry::Options(options) => WayOut::options(options.clone()).unwrap_or_else(|| {
                // No option to choose: the call without the argument.
                let mut arguments = given.clone();
                set_path(&mut arguments, &field, None);
                WayOut::call(call(arguments, Vec::new()))
            }),
            Retry::Value { value, fill } => {
                let mut arguments = given.clone();
                set_path(&mut arguments, &field, Some(value.clone()));
                WayOut::call(call(arguments, if *fill { vec![field.clone()] } else { Vec::new() }))
            }
            Retry::Example => {
                let arguments = match example(&self.schema(), "") {
                    Value::Object(map) => map,
                    _ => Map::new(),
                };
                let fill = arguments.keys().cloned().collect();
                WayOut::call(call(arguments, fill))
            }
        };
        ToolError::refusal(Refusal::InvalidArguments, error.message.clone(), way_out).at(field)
    }

    fn aliased(&self, mut arguments: Value, notes: &mut Vec<String>) -> Result<Value, ToolError> {
        for alias in self.aliases() {
            let parts: Vec<&str> = if alias.path.is_empty() {
                Vec::new()
            } else {
                alias.path.split('.').collect()
            };
            walk(&mut arguments, &parts, "", &mut |node, where_| {
                renamed(node, &alias.alias, &alias.name, where_, notes)
            })?;
        }
        for property in self.arguments.properties() {
            let Some(name) = &property.intent.names_as else {
                continue;
            };
            if names_meant(&arguments, &property.name, name) {
                renamed(&mut arguments, &property.name, name, "", notes)?;
            }
        }
        if let Some(places) = &self.places {
            for name in &places.names {
                let Some(map) = arguments
                    .as_object()
                    .filter(|m| m.get(name).is_some_and(|v| !v.is_null()))
                else {
                    continue;
                };
                if let Some(meant) = places.meant(&self.name, name, map)? {
                    renamed(&mut arguments, name, &meant, "", notes)?;
                }
            }
            if let Some(field) = &places.items {
                self.items_placed(&mut arguments, places, field, notes)?;
            }
        }
        Ok(arguments)
    }

    fn defaulted(&self, mut arguments: Value, notes: &mut Vec<String>) -> Value {
        let Value::Object(map) = &mut arguments else {
            return arguments;
        };
        let missing: Vec<(&str, &Value)> = self
            .defaults()
            .into_iter()
            .filter(|(name, _)| map.get(*name).is_none_or(Value::is_null))
            .collect();
        for (name, value) in &missing {
            notes.push(format!("{name} not given: read as {}", crate::pyfmt::repr(value)));
        }
        for (name, value) in missing {
            map.insert(name.to_owned(), value.clone());
        }
        arguments
    }

    /// Each item of the array argument `field` with its places read as [`Places::meant`]
    /// reads a call's (`items[0].after` read as `at`).
    fn items_placed(
        &self,
        arguments: &mut Value,
        places: &Places,
        field: &str,
        notes: &mut Vec<String>,
    ) -> Result<(), ToolError> {
        let Some(Value::Array(items)) = arguments.as_object_mut().and_then(|m| m.get_mut(field)) else {
            return Ok(());
        };
        for (index, item) in items.iter_mut().enumerate() {
            for name in &places.names {
                let Some(map) = item.as_object().filter(|m| m.get(name).is_some_and(|v| !v.is_null())) else {
                    continue;
                };
                if let Some(meant) = places.meant(&self.name, name, map)? {
                    renamed(item, name, &meant, &format!("{field}[{index}]"), notes)?;
                }
            }
        }
        Ok(())
    }

    /// An array or object argument given as a string holding its JSON, parsed; inside such
    /// a string an unescaped `\v` is read as U+000B.
    fn json_strings(&self, mut arguments: Value) -> Value {
        let Value::Object(map) = &mut arguments else {
            return arguments;
        };
        for (name, value) in map.iter_mut() {
            let Some(property) = self.arguments.property(name) else {
                continue;
            };
            let array = match property.schema.kind {
                Kind::Array(_) => true,
                Kind::Object(_) | Kind::Free => false,
                _ => continue,
            };
            let Value::String(text) = value else { continue };
            let text = text.trim();
            if !text.starts_with(if array { '[' } else { '{' }) {
                continue;
            }
            let parsed = serde_json::from_str::<Value>(text).or_else(|_| serde_json::from_str(&vertical_tabs(text)));
            if let Ok(parsed) = parsed {
                if parsed.is_array() == array && (array || parsed.is_object()) {
                    *value = parsed;
                }
            }
        }
        arguments
    }

    fn exactly_one(&self, arguments: &Map<String, Value>) -> Result<(), ToolError> {
        for group in &self.exactly_one {
            let given: Vec<&String> = group.iter().filter(|name| arguments.contains_key(*name)).collect();
            if given.len() == 1 {
                continue;
            }
            let joined = |names: Vec<&String>| names.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ");
            let mut message = format!("give exactly one of {}", joined(group.iter().collect()));
            if !given.is_empty() {
                message = format!("{message}; got {}", joined(given.clone()));
            }
            let field = given
                .get(1)
                .or(group.first().as_ref())
                .map_or(String::new(), |s| (*s).clone());
            let options = WayOut::names(group.iter().cloned()).unwrap_or_else(|| {
                WayOut::call(TryCall {
                    tool: self.name.clone(),
                    arguments: arguments.clone(),
                    fill: Vec::new(),
                })
            });
            return Err(ToolError::refusal(Refusal::InvalidArguments, message, options).at(field));
        }
        Ok(())
    }
}

/// Equal and of one type (`0` is not `false`; `1` is not `1.0`).
fn same(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => x.is_f64() == y.is_f64() && x == y,
        _ => a == b,
    }
}

type Visit<'a> = dyn FnMut(&mut Value, &str) -> Result<(), ToolError> + 'a;

/// `visit(node, where)` at the argument path `parts` (`["items[]", "frame"]`), each value
/// named by where it is, as today's `_walk`.
fn walk(node: &mut Value, parts: &[&str], where_: &str, visit: &mut Visit) -> Result<(), ToolError> {
    let Some((part, rest)) = parts.split_first() else {
        if let Value::Array(items) = node {
            for (index, item) in items.iter_mut().enumerate() {
                visit(item, &format!("{where_}[{index}]"))?;
            }
            return Ok(());
        }
        return visit(node, where_);
    };
    let many = part.ends_with("[]");
    let name = part.trim_end_matches("[]");
    let Some(child) = node.as_object_mut().and_then(|map| map.get_mut(name)) else {
        return Ok(());
    };
    let path = crate::validate::join(where_, name);
    match child {
        Value::Array(items) if many => {
            for (index, item) in items.iter_mut().enumerate() {
                let at = format!("{path}[{index}]");
                if rest.is_empty() {
                    visit(item, &at)?;
                } else {
                    walk(item, rest, &at, visit)?;
                }
            }
            Ok(())
        }
        other => walk(other, rest, &path, visit),
    }
}

/// One object of the arguments (at `where_`) with `alias` read as `name`; both given is a
/// refusal (the call says one thing twice).
/// Whether `alias`, a property that takes no names, holds a list of names -- strings that
/// are not numbers -- while `name` is not given: then it can only mean `name` (today's
/// `names_as`).
fn names_meant(arguments: &Value, alias: &str, name: &str) -> bool {
    let Value::Object(map) = arguments else { return false };
    if map.get(name).is_some_and(|v| !v.is_null()) {
        return false;
    }
    let Some(Value::Array(items)) = map.get(alias) else {
        return false;
    };
    !items.is_empty()
        && items
            .iter()
            .all(|item| item.as_str().is_some_and(|text| !crate::pyfmt::is_float(text)))
}

fn renamed(node: &mut Value, alias: &str, name: &str, where_: &str, notes: &mut Vec<String>) -> Result<(), ToolError> {
    let Value::Object(map) = node else { return Ok(()) };
    if map.get(alias).is_none_or(Value::is_null) {
        return Ok(());
    }
    let field = crate::validate::join(where_, alias);
    if map.get(name).is_some_and(|v| !v.is_null()) {
        let way_out = WayOut::one(Value::from(name));
        let message = format!("{alias} and {name} say the same here; give {name} only");
        return Err(ToolError::refusal(Refusal::InvalidArguments, message, way_out).at(field));
    }
    if let Some(value) = map.shift_remove(alias) {
        map.insert(name.to_owned(), value);
    }
    notes.push(format!("{field} read as {name}"));
    Ok(())
}

/// `text` with every unescaped `\v` (an odd run of backslashes before `v`) as `\u000b`.
fn vertical_tabs(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut run = 0usize;
    for c in text.chars() {
        if c == 'v' && run % 2 == 1 {
            out.push_str("u000b");
            run = 0;
            continue;
        }
        run = if c == '\\' { run + 1 } else { 0 };
        out.push(c);
    }
    out
}

/// Sets (or, with `None`, removes) the value at a dotted path with indexes
/// (`items[2].text`); a path that does not exist is left as it is, except its last name.
pub(crate) fn set_path(map: &mut Map<String, Value>, path: &str, value: Option<Value>) {
    let mut steps: Vec<Step> = Vec::new();
    for part in path.split('.').filter(|p| !p.is_empty()) {
        let (name, indexes) = part.split_once('[').map_or((part, ""), |(n, rest)| (n, rest));
        steps.push(Step::Name(name.to_owned()));
        for index in indexes.split('[').filter(|i| !i.is_empty()) {
            if let Ok(index) = index.trim_end_matches(']').parse() {
                steps.push(Step::Index(index));
            }
        }
    }
    let mut root = Value::Object(std::mem::take(map));
    put(&mut root, &steps, value);
    if let Value::Object(back) = root {
        *map = back;
    }
}

enum Step {
    Name(String),
    Index(usize),
}

fn put(node: &mut Value, steps: &[Step], value: Option<Value>) {
    let Some((step, rest)) = steps.split_first() else {
        return;
    };
    match (step, node) {
        (Step::Name(name), Value::Object(map)) if rest.is_empty() => match value {
            Some(value) => {
                map.insert(name.clone(), value);
            }
            None => {
                map.shift_remove(name);
            }
        },
        (Step::Name(name), Value::Object(map)) => {
            if let Some(child) = map.get_mut(name) {
                put(child, rest, value);
            }
        }
        (Step::Index(index), Value::Array(items)) if rest.is_empty() => match value {
            Some(value) => {
                if let Some(slot) = items.get_mut(*index) {
                    *slot = value;
                }
            }
            None => {
                if *index < items.len() {
                    items.remove(*index);
                }
            }
        },
        (Step::Index(index), Value::Array(items)) => {
            if let Some(child) = items.get_mut(*index) {
                put(child, rest, value);
            }
        }
        // Not the shape the path names: replace the node as a whole when it is the last step.
        (_, node) if rest.is_empty() => {
            if let Some(value) = value {
                *node = value;
            }
        }
        _ => {}
    }
}
