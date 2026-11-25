//! Owns: `word_notes` (docx-agent's `tools/structure.py`, `word_notes`): its definition,
//! its list, and its edits through the edit core's note commands.
//!
//! Must not: change the document but through a command.

use super::common::{call, find_once, need, one_of, outcome, policy, refused, Outcome};
use crate::describe::short;
use crate::review::Editing;
use ooxml_tools::{ArgEnum, ToolArgs, ToolError, ToolResult, TryCall};
use ooxml_word_edit::{DeleteNote, EditNote, InsertNote, MoveNote, NoteEdited};
use ooxml_word_model::docx::text::{paragraph_text, View};
use ooxml_word_model::ids::W;
use serde_json::{json, Value};

/// What `word_notes` does.
#[derive(ArgEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoteAction {
    /// List the notes.
    List,
    /// Insert a note.
    Insert,
    /// Rewrite a note's text.
    Edit,
    /// Delete a note and its reference.
    Delete,
    /// Move a note's reference.
    Move,
}

/// Which notes.
#[derive(ArgEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoteKind {
    /// A footnote.
    Footnote,
    /// An endnote.
    Endnote,
}

/// `word_notes`' arguments.
#[derive(ToolArgs, Debug, Clone, PartialEq, Eq)]
#[tool(
    name = "word_notes",
    description = include_str!("word_notes.md"),
    group = "word_structure",
    mutates,
    places = "mark",
    place_own("at", "note"),
    place_find
)]
pub struct WordNotes {
    /// Document id.
    pub doc: String,
    /// What to do.
    pub action: NoteAction,
    /// insert. Default footnote.
    pub kind: Option<NoteKind>,
    /// insert, move: position or range; the mark goes at its end.
    #[arg(refs)]
    pub at: Option<String>,
    /// insert, move: or text occurring once; the mark goes after it.
    pub find: Option<String>,
    /// edit, delete, move: the note (fn:2, en:1).
    #[arg(refs)]
    pub note: Option<String>,
    /// insert, edit: the text.
    pub text: Option<String>,
    /// insert: ref name for the note.
    #[arg(rename = "ref")]
    pub ref_name: Option<String>,
}

impl From<NoteEdited> for Outcome {
    fn from(out: NoteEdited) -> Self {
        Outcome {
            changed: out.id.is_some(),
            id: out.id,
            created: out.created,
            removed: out.removed,
            renamed: ooxml_word_model::ids::renames_text(&out.renamed),
            warnings: Vec::new(),
        }
    }
}

/// `word_notes` (docx-agent's `word_notes`): list the notes; insert one after text found
/// once or at a position; rewrite, delete or move one -- each one command.
#[must_use]
pub fn word_notes(args: &WordNotes, editing: &Editing<'_>) -> ToolResult<Value> {
    if let Err(error) = crate::review::stopped(editing.cancel) {
        return ToolResult::failed(error);
    }
    let list = call("word_notes", &[("doc", json!(args.doc)), ("action", json!("list"))]);
    match run(args, editing, &list) {
        Ok(result) => result,
        Err(error) => ToolResult::failed(error),
    }
}

fn run(args: &WordNotes, editing: &Editing<'_>, list: &TryCall) -> Result<ToolResult<Value>, ToolError> {
    let editor = editing.editor;
    let summary = |done: &NoteEdited, what: &str| format!("{what} {}", done.id.clone().unwrap_or_default());
    let reply =
        |result: Result<ooxml_word_edit::Edited<NoteEdited>, ooxml_word_edit::EditError>, what: String| match result {
            Ok(done) => outcome(&[done.out.into()], what, None),
            Err(error) => refused(error, list.clone()),
        };
    if args.action == NoteAction::List {
        return Ok(listed(editing));
    }
    if args.action == NoteAction::Insert {
        let at = position(args, editing, list)?;
        let command = InsertNote {
            kind: match args.kind {
                Some(NoteKind::Endnote) => ooxml_word_edit::NoteKind::Endnote,
                _ => ooxml_word_edit::NoteKind::Footnote,
            },
            at: ooxml_word_model::address::Address::from(at),
            text: args.text.clone().unwrap_or_default(),
            policy: policy(editing),
        };
        return Ok(match editor.run(editing.caller, &command) {
            Ok(done) => {
                let text = summary(&done.out, "Inserted");
                let id = done.out.id.clone();
                let mut result = outcome(&[done.out.into()], text, None);
                if let (Some(name), Some(id), ooxml_tools::Outcome::Done(made)) =
                    (&args.ref_name, id, &mut result.outcome)
                {
                    made.refs.push((name.clone(), id));
                }
                result
            }
            Err(error) => refused(error, list.clone()),
        });
    }
    let note = need(args.note.as_deref(), "note", list)?.to_owned();
    Ok(match args.action {
        NoteAction::Edit => {
            let text = need(args.text.as_deref(), "text", list)?.to_owned();
            let command = EditNote {
                note: note.clone(),
                text,
                policy: policy(editing),
            };
            reply(editor.run(editing.caller, &command), format!("Edited {note}"))
        }
        NoteAction::Delete => {
            let command = DeleteNote {
                note: note.clone(),
                policy: policy(editing),
            };
            reply(editor.run(editing.caller, &command), format!("Deleted {note}"))
        }
        _ => {
            let to = position(args, editing, list)?;
            let command = MoveNote {
                note: note.clone(),
                to: ooxml_word_model::address::Address::from(to),
                policy: policy(editing),
            };
            reply(editor.run(editing.caller, &command), format!("Moved {note}"))
        }
    })
}

/// Where a mark goes (docx-agent's `_position`): `at` as given, or the end of the text
/// `find` names once.
fn position(args: &WordNotes, editing: &Editing<'_>, list: &TryCall) -> Result<String, ToolError> {
    let key = one_of(&[("at", args.at.is_some()), ("find", args.find.is_some())], list)?;
    if key == "find" {
        let range = find_once(editing, args.find.as_deref().unwrap_or(""), list)?;
        return Ok(end_of(&range));
    }
    Ok(args.at.clone().unwrap_or_default())
}

/// A range id in one paragraph as the position at its end (`p:A@3:9` is `p:A@9`); a range
/// across paragraphs as it is.
pub(crate) fn end_of(range: &str) -> String {
    if range.contains("..") {
        return range.to_owned();
    }
    match range.rsplit_once('@') {
        Some((base, span)) => format!("{base}@{}", span.rsplit(':').next().unwrap_or(span)),
        None => range.to_owned(),
    }
}

/// Every note (a hundred), with its kind and text, as docx-agent lists them.
fn listed(editing: &Editing<'_>) -> ToolResult<Value> {
    let head = editing.editor.current();
    let docx = match head.doc().docx() {
        Ok(docx) => docx,
        Err(error) => return ToolResult::failed(ToolError::internal(error.to_string(), "word:model")),
    };
    let mut notes = Vec::new();
    for story in &docx.parts {
        let root = story.root();
        let (prefix, kind) = if root.name().is(Some(W), "footnotes") {
            ("fn", "footnote")
        } else if root.name().is(Some(W), "endnotes") {
            ("en", "endnote")
        } else {
            continue;
        };
        for (k, node) in root.children().iter().enumerate() {
            let ooxml_xml::Node::Element(note) = node else { continue };
            let normal = matches!(note.attribute(Some(W), "type"), None | Some("normal"));
            if !(normal && (note.name().is(Some(W), "footnote") || note.name().is(Some(W), "endnote"))) {
                continue;
            }
            let path = [u32::try_from(k).unwrap_or(u32::MAX)];
            let text: Vec<String> = story
                .ids
                .paragraphs_within(&path)
                .iter()
                .map(|p| paragraph_text(&p.element, View::Current))
                .collect();
            let text = text.join("\n").replacen('\u{fffc}', "", 1);
            let text = text.trim_matches(|c: char| c.is_whitespace());
            let id = format!("{prefix}:{}", note.attribute(Some(W), "id").unwrap_or(""));
            notes.push(json!({"id": id, "kind": kind, "text": short(text, 200)}));
        }
    }
    notes.truncate(100);
    let mut done = ooxml_tools::Done::new("Notes");
    done.data = Some(Value::Array(notes));
    ToolResult::done(done)
}

/// Registers `word_notes` (the crate's [`crate::register`]).
pub(crate) fn register(r: &mut impl crate::Registrar) {
    r.edit::<WordNotes>(word_notes, crate::Hosting::PLAIN);
}
