//! Owns: `set_properties` (docx-agent's handler of the shared tool): the title, author,
//! language and subject core properties set through the edit core's
//! [`SetCoreProperties`], and the four read back.
//!
//! Must not: set the dates or the revision (the tool takes none).

use crate::review::{edit_refusal, stopped, Editing};
use ooxml_tools::{Done, Refusal, ToolArgs, ToolError, ToolResult, TryCall, WayOut};
use ooxml_word_edit::{core_properties, CoreProperty, SetCoreProperties};
use serde_json::{Map, Value};

/// `set_properties`'s arguments, shared by every format.
#[derive(ToolArgs, Debug, Clone, PartialEq, Eq)]
#[tool(
    name = "set_properties",
    description = include_str!("set_properties.md"),
    group = "shared_misc",
    mutates
)]
pub struct SetProperties {
    /// Document id.
    pub doc: String,
    /// Title.
    pub title: Option<String>,
    /// Author.
    pub author: Option<String>,
    /// Editing language, e.g. en-US.
    pub language: Option<String>,
    /// Subject.
    pub subject: Option<String>,
}

/// The tool's properties, by the names it gives them, in its order.
const NAMES: [(&str, CoreProperty); 4] = [
    ("title", CoreProperty::Title),
    ("author", CoreProperty::Author),
    ("language", CoreProperty::Language),
    ("subject", CoreProperty::Subject),
];

impl SetProperties {
    fn given(&self) -> Vec<(&'static str, CoreProperty, String)> {
        let values = [&self.title, &self.author, &self.language, &self.subject];
        NAMES
            .into_iter()
            .zip(values)
            .filter_map(|((name, property), value)| value.as_ref().map(|v| (name, property, v.clone())))
            .collect()
    }

    fn try_call(&self) -> TryCall {
        let mut arguments = Map::new();
        arguments.insert("doc".into(), Value::from(self.doc.as_str()));
        for (name, _, value) in self.given() {
            arguments.insert(name.into(), Value::from(value));
        }
        TryCall {
            tool: "set_properties".to_owned(),
            arguments,
            fill: Vec::new(),
        }
    }
}

/// `set_properties`: the properties given set as one step (none when each already is),
/// the four read back (`null` for one the document does not set).
#[must_use]
pub fn set_properties(args: &SetProperties, editing: &Editing<'_>) -> ToolResult<Value> {
    match set(args, editing) {
        Ok(done) => ToolResult::done(done),
        Err(error) => ToolResult::failed(error),
    }
}

fn set(args: &SetProperties, editing: &Editing<'_>) -> Result<Done<Value>, ToolError> {
    stopped(editing.cancel)?;
    let given = args.given();
    if given.is_empty() {
        return Err(ToolError::refusal(
            Refusal::InvalidArguments,
            "give a property to set",
            WayOut::names(NAMES.iter().map(|(name, _)| *name)).unwrap_or_else(|| WayOut::call(args.try_call())),
        ));
    }
    let command = SetCoreProperties {
        values: given.iter().map(|(_, p, v)| (*p, v.clone())).collect(),
    };
    let done = editing
        .editor
        .run(editing.caller, &command)
        .map_err(|e| edit_refusal(e, "", args.try_call()))?;
    let now = core_properties(done.version.doc()).map_err(|e| ToolError::internal(e.to_string(), "word:model"))?;
    let mut data = Map::new();
    for (name, property) in NAMES {
        let value = now.iter().find(|(p, _)| *p == property).map(|(_, v)| v.as_str());
        data.insert(name.into(), value.map_or(Value::Null, Value::from));
    }
    let names: Vec<&str> = given.iter().map(|(name, _, _)| *name).collect();
    let mut reply = Done::new(format!("Set {}", names.join(", ")));
    if done.out.changed {
        // docx-agent's `EditResult("properties")`
        reply.changed.push("properties".to_owned());
    }
    reply.data = Some(Value::Object(data));
    Ok(reply)
}

/// Registers `set_properties` (the group's [`super::register`]).
pub(crate) fn register(r: &mut impl crate::Registrar) {
    r.edit::<SetProperties>(set_properties, crate::Hosting::SERVED);
}
