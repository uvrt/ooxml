//! Owns: what the structure tools share -- an edit's outcome as docx-agent's `outcome`
//! writes the reply (ids made, changed, removed and renamed, warnings), an edit's refusal
//! as the tool's (an edit not ported yet said in `unported`, for the host to answer its own
//! way), the arguments docx-agent's `need` and `one_of` ask for, text found once
//! (`find_range`), and the edit policy of the host's tracking mode.
//!
//! Must not: change the document but through a command.

use crate::review::{edit_refusal, Editing};
use ooxml_tools::{Done, Refusal, ToolError, ToolResult, TryCall, WayOut};
use ooxml_word_edit::{EditError, RevisionPolicy, Tracking};
use serde_json::{Map, Value};

/// What docx-agent's `EditResult` says of one edit.
#[derive(Debug, Clone, Default)]
pub(crate) struct Outcome {
    pub(crate) id: Option<String>,
    pub(crate) changed: bool,
    pub(crate) created: Vec<String>,
    pub(crate) removed: Vec<String>,
    pub(crate) renamed: Vec<(String, String)>,
    pub(crate) warnings: Vec<String>,
}

impl From<ooxml_word_edit::FieldEdited> for Outcome {
    fn from(out: ooxml_word_edit::FieldEdited) -> Self {
        Outcome {
            id: out.id,
            changed: out.changed,
            created: out.created,
            removed: Vec::new(),
            renamed: out.renamed,
            // docx-agent's `outcome` notes each of an edit's `unknown` after its warnings
            warnings: out
                .warnings
                .into_iter()
                .chain(out.unknown.iter().map(|u| format!("{u}: the layout cannot say")))
                .collect(),
        }
    }
}

impl From<ooxml_word_edit::LinkEdited> for Outcome {
    fn from(out: ooxml_word_edit::LinkEdited) -> Self {
        Outcome {
            id: out.id,
            changed: out.changed,
            created: out.created,
            removed: out.removed,
            renamed: out.renamed,
            warnings: out.warnings,
        }
    }
}

/// The reply of `edits`, as docx-agent's `outcome` writes it: each edit's made and
/// removed ids, its id as changed (when it changed and was not made), its renames (a later
/// one of the same id winning in place) and warnings, in order.
pub(crate) fn outcome(edits: &[Outcome], summary: String, data: Option<Value>) -> ToolResult<Value> {
    let mut done = Done::new(summary);
    done.data = data;
    let mut warnings: Vec<String> = Vec::new();
    for edit in edits {
        for id in &edit.created {
            if !done.created.contains(id) {
                done.created.push(id.clone());
            }
        }
        for id in &edit.removed {
            if !done.removed.contains(id) {
                done.removed.push(id.clone());
            }
        }
        if let Some(id) = edit.id.as_ref().filter(|_| edit.changed) {
            if !done.created.contains(id) && !done.changed.contains(id) {
                done.changed.push(id.clone());
            }
        }
        for (old, new) in &edit.renamed {
            match done.renamed.iter_mut().find(|(o, _)| o == old) {
                Some(found) => found.1.clone_from(new),
                None => done.renamed.push((old.clone(), new.clone())),
            }
        }
        for warning in &edit.warnings {
            if !warnings.contains(warning) {
                warnings.push(warning.clone());
            }
        }
    }
    let mut result = ToolResult::done(done);
    result.warnings = warnings;
    result
}

/// An edit's refusal as the tool's reply: one not ported yet is said in `unported`.
pub(crate) fn refused(error: EditError, call: TryCall) -> ToolResult<Value> {
    if let EditError::Unsupported(what) = error {
        let mut done = Done::new(String::new());
        done.unported.push(what);
        return ToolResult::done(done);
    }
    ToolResult::failed(edit_refusal(error, "", call))
}

/// `value`, or `invalid_arguments` saying `name` is required (docx-agent's `need`).
pub(crate) fn need<'v, T: ?Sized>(value: Option<&'v T>, name: &str, call: &TryCall) -> Result<&'v T, ToolError> {
    value.ok_or_else(|| crate::review::needed(name, "", call.clone()))
}

/// The one of `given` (name, whether given) given; `invalid_arguments` unless exactly
/// one is (docx-agent's `one_of`).
pub(crate) fn one_of(given: &[(&str, bool)], call: &TryCall) -> Result<String, ToolError> {
    let names: Vec<&str> = given.iter().map(|(n, _)| *n).collect();
    let present: Vec<&str> = given.iter().filter(|(_, g)| *g).map(|(n, _)| *n).collect();
    if let [one] = present.as_slice() {
        return Ok((*one).to_owned());
    }
    let mut message = format!("give exactly one of {}", names.join(", "));
    if !present.is_empty() {
        message.push_str(&format!("; got {}", present.join(", ")));
    }
    let field = present.get(1).or(names.first()).copied().unwrap_or("");
    let way_out = WayOut::names(names.iter().copied())
        .unwrap_or_else(|| WayOut::call(call.clone()))
        .and_call(call.clone());
    Err(ToolError::refusal(Refusal::InvalidArguments, message, way_out).at(field))
}

/// The range id of the one place `find` occurs in the document now (docx-agent's
/// `find_range` without `within`).
pub(crate) fn find_once(editing: &Editing<'_>, find: &str, call: &TryCall) -> Result<String, ToolError> {
    find_once_within(editing, find, None, call)
}

/// [`find_once`] in the paragraphs `within` holds, when given (docx-agent's `find_range`).
pub(crate) fn find_once_within(
    editing: &Editing<'_>,
    find: &str,
    within: Option<&str>,
    call: &TryCall,
) -> Result<String, ToolError> {
    let head = editing.editor.store().snapshot();
    ooxml_word_edit::anchor(head.doc(), find, within).map_err(|e| edit_refusal(e, "", call.clone()))
}

/// The policy an edit runs by: tracked as the host's author, or untracked.
pub(crate) fn policy(editing: &Editing<'_>) -> RevisionPolicy {
    match editing.tracking {
        Some(tracking) => RevisionPolicy::Tracked(Tracking::clone(tracking)),
        None => RevisionPolicy::Untracked,
    }
}

/// A call of `tool` with `arguments`.
pub(crate) fn call(tool: &str, arguments: &[(&str, Value)]) -> TryCall {
    let arguments: Map<String, Value> = arguments.iter().map(|(k, v)| ((*k).to_owned(), v.clone())).collect();
    TryCall {
        tool: tool.to_owned(),
        arguments,
        fill: Vec::new(),
    }
}
