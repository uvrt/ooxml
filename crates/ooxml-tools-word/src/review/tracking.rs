//! Owns: `word_set_tracking` (docx-agent's `word_set_tracking`): the session's tracking
//! mode for the document -- handed back to the session, which keeps it and tracks later
//! edits by its author -- and, when asked, Word's own Track Changes switch, set through
//! the edit core's [`SetWordTracking`].
//!
//! Must not: keep the mode itself (the session does).

use super::Editing;
use ooxml_tools::{Done, ToolArgs, ToolError, ToolResult, TryCall};
use ooxml_word_edit::{word_tracks_changes, SetWordTracking};
use serde_json::{json, Map, Value};

/// `word_set_tracking`'s arguments.
#[derive(ToolArgs, Debug, Clone, PartialEq, Eq)]
#[tool(
    name = "word_set_tracking",
    description = include_str!("word_set_tracking.md"),
    group = "word_review",
    mutates,
    batchable = false
)]
pub struct WordSetTracking {
    /// Document id.
    pub doc: String,
    /// Track later edits.
    pub on: bool,
    /// Revisions' author; needed to turn it on.
    pub author: Option<String>,
    /// Also set Word's Track Changes to on.
    pub word_switch: Option<bool>,
}

/// The tracking mode a call leaves: edits by `author` are tracked changes, or none are.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Tracked {
    /// Tracking is off.
    Off,
    /// Later edits are tracked changes by this author.
    On(String),
}

/// `word_set_tracking`: the reply, and the mode the session keeps from now on (`None`: the
/// call failed and the mode stays as it was).
#[must_use]
pub fn word_set_tracking(args: &WordSetTracking, editing: &Editing<'_>) -> (ToolResult<Value>, Option<Tracked>) {
    match set(args, editing) {
        Ok((done, mode)) => (ToolResult::done(done), Some(mode)),
        Err(error) => (ToolResult::failed(error), None),
    }
}

fn set(args: &WordSetTracking, editing: &Editing<'_>) -> Result<(Done<Value>, Tracked), ToolError> {
    super::stopped(editing.cancel)?;
    let mode = match (args.on, &args.author) {
        (true, None) => return Err(super::needed("author", " to turn tracking on", try_call(args))),
        (true, Some(author)) => Tracked::On(author.clone()),
        (false, _) => Tracked::Off,
    };
    let mut head = editing.editor.current().doc().clone();
    if let Some(switch) = args.word_switch {
        let done = editing
            .editor
            .run(editing.caller, &SetWordTracking { on: switch })
            .map_err(|e| super::edit_refusal(e, "", try_call(args)))?;
        head = done.version.doc().clone();
    }
    let word_switch = word_tracks_changes(&head).map_err(|e| ToolError::internal(e.to_string(), "word:model"))?;
    let mut summary = match &mode {
        Tracked::On(author) => format!("Tracking on, as {author}"),
        Tracked::Off => "Tracking off".to_owned(),
    };
    if let Some(switch) = args.word_switch {
        summary.push_str(&format!("; Word's switch {}", if switch { "on" } else { "off" }));
    }
    let author = match &mode {
        Tracked::On(author) => json!(author),
        Tracked::Off => Value::Null,
    };
    let mut data = Map::new();
    data.insert("tracking".into(), json!(args.on));
    data.insert("author".into(), author);
    data.insert("word_switch".into(), json!(word_switch));
    let mut done = Done::new(summary);
    done.data = Some(Value::Object(data));
    Ok((done, mode))
}

/// `word_set_tracking` called with `args`, an author given when tracking is turned on.
fn try_call(args: &WordSetTracking) -> TryCall {
    let mut arguments = Map::new();
    arguments.insert("doc".into(), Value::from(args.doc.as_str()));
    arguments.insert("on".into(), Value::from(args.on));
    if args.on {
        let author = args.author.clone().unwrap_or_else(|| "Reviewer".to_owned());
        arguments.insert("author".into(), Value::from(author));
    }
    if let Some(switch) = args.word_switch {
        arguments.insert("word_switch".into(), Value::from(switch));
    }
    TryCall {
        tool: "word_set_tracking".to_owned(),
        arguments,
        fill: if args.on && args.author.is_none() {
            vec!["author".to_owned()]
        } else {
            Vec::new()
        },
    }
}

/// Registers `word_set_tracking` (the crate's [`crate::register`]).
pub(crate) fn register(r: &mut impl ooxml_tools::Registrar<crate::Word>) {
    r.tracking::<WordSetTracking>(word_set_tracking);
}
