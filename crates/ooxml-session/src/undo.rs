//! Owns: the `undo` tool (docx-agent's, `ooxml_edit.tools.shared`'s `undo` for a Word
//! document): the session's own undo or redo of a document's latest steps, or with a named
//! caller that caller's own ([`crate::history`]), answered in today's words -- what was
//! undone or redone, and why not: nothing (of the caller's) to take, the caller's latest
//! change not the latest (the later steps and who made each, [`crate::owners`]), its undone
//! change not the next to redo. Nothing to take is a refusal with its way out (tool rule 2):
//! the document's steps and who made each as its valid options, and the undo or redo the
//! other way when that one would work. The session runs it itself, as `read_batch`: it moves
//! the document's history, which no tool handler is handed.
//!
//! Must not: take a scope (a Word body is one scope every change shares: the definition has
//! none), or run inside a batch.

use crate::history::{Direction, NumberedStep, Numbers};
use crate::owners::Owners;
use crate::{CallContext, Session, SessionError, Undone};
use ooxml_tools::{Accepted, Done, Failure, Refusal, ToolArgs, ToolError, ToolResult, TryCall, WayOut};
use serde_json::{json, Map, Value};

/// `undo`'s arguments.
#[derive(ToolArgs, Debug, Clone, PartialEq)]
#[tool(
    name = "undo",
    description = include_str!("undo.md"),
    role = "edit",
    mutates = false,
    batchable = false
)]
pub struct Undo {
    /// Document id.
    pub doc: String,
    /// Steps, 1-50. Default 1.
    #[arg(minimum = 1, maximum = 50)]
    pub steps: Option<i64>,
    /// Redo instead. Default false.
    pub redo: Option<bool>,
}

/// The tool's name.
pub const UNDO: &str = "undo";

/// Steps listed of each side of the head in a refusal's way out (tool rule 4).
const LISTED: usize = 10;

impl Session {
    /// `undo`: its arguments taken as meant and validated, then the document's history
    /// moved and the reply worded.
    pub(crate) fn call_undo(&self, arguments: Value, context: &CallContext) -> ToolResult<Value> {
        let spec = self.registry().undo_spec();
        let accepted = spec.accept(arguments).and_then(|Accepted { arguments, notes }| {
            let (named, opened) = self.document_of(spec, &arguments, &context.names, None)?;
            Ok((arguments, notes, named, opened))
        });
        let (arguments, notes, named, opened) = match accepted {
            Ok(found) => found,
            Err(error) => return Self::undo_reply(ToolResult::failed(error)),
        };
        let steps = arguments.get("steps").and_then(Value::as_u64).unwrap_or(1);
        let direction = if arguments.get("redo").and_then(Value::as_bool).unwrap_or(false) {
            Direction::Redo
        } else {
            Direction::Undo
        };
        let asked = Asked {
            doc: &named,
            caller: context.caller.name(),
            steps,
            direction,
        };
        let id = opened.id().clone();
        // (at most 50: the definition's bound)
        let moved = self.history(&id, usize::try_from(steps).unwrap_or(usize::MAX), context, direction);
        let mut result = match moved {
            Ok(undone) if undone.count > 0 => {
                let mut result = ToolResult::done(asked.done(undone));
                result.warnings.extend(asked.fewer(undone));
                result.version = Some(undone.version);
                result
            }
            Ok(_) => ToolResult::failed(match (self.numbers(&id), self.document(&id)) {
                (Ok(numbers), Ok(document)) => asked.nothing(&numbers, document.owners(), document.head().number()),
                (Err(gone), _) | (_, Err(gone)) => gone.to_tool_error(),
            }),
            Err(error) => ToolResult::failed(match self.document(&id) {
                Ok(document) => asked.refused(&error, document.owners()),
                Err(gone) => gone.to_tool_error(),
            }),
        };
        if result.version.is_none() {
            // (refused: the state the document is at)
            result.version = self.document(&id).ok().map(|d| d.head().number());
        }
        result.doc = Some(named.clone());
        let mut warnings = notes;
        warnings.append(&mut result.warnings);
        result.warnings = warnings;
        Self::undo_reply(result)
    }

    fn undo_reply(mut result: ToolResult<Value>) -> ToolResult<Value> {
        result.app.tool = UNDO.to_owned();
        result
    }
}

/// What an `undo` call asked for, as its reply words it.
struct Asked<'a> {
    /// The document, as the call names it.
    doc: &'a str,
    /// The caller's key (`None`: the session's own undo).
    caller: Option<&'a str>,
    steps: u64,
    direction: Direction,
}

impl Asked<'_> {
    fn verb(&self) -> &'static str {
        match self.direction {
            Direction::Undo => "undo",
            Direction::Redo => "redo",
        }
    }

    /// `undone` steps taken: what was done, and how many of those asked for.
    fn done(&self, undone: Undone) -> Done<Value> {
        let verb = match self.direction {
            Direction::Undo => "Undid",
            Direction::Redo => "Redid",
        };
        let own = if self.caller.is_some() { " of your own" } else { "" };
        let mut done = Done::new(format!("{verb} {}{own} step(s) of {}", undone.count, self.doc));
        done.changed.push(self.doc.to_owned());
        let mut data = Map::new();
        data.insert("steps".into(), Value::from(undone.count));
        data.insert("version".into(), Value::from(undone.version));
        if self.caller.is_some() {
            data.insert("own".into(), Value::Bool(true));
        }
        done.data = Some(Value::Object(data));
        done
    }

    /// A caller's own undo or redo that took fewer steps than asked: why.
    fn fewer(&self, undone: Undone) -> Option<String> {
        let count = u64::try_from(undone.count).unwrap_or(u64::MAX);
        (self.caller.is_some() && count < self.steps).then(|| {
            format!(
                "{count} of {} step(s): no more of yours, or the next depends on a later change by another caller",
                self.steps
            )
        })
    }

    /// Nothing (of the caller's) to take, in today's words; its way out: the document's
    /// steps and who made each ([`Asked::steps`]), and the undo or redo the other way when
    /// that one would work for the caller. `head`: the state the document is at.
    fn nothing(&self, numbers: &Numbers, owners: &Owners, head: u64) -> ToolError {
        let (yours, why) = if self.caller.is_some() {
            (" of yours", ": undo here takes back your own changes only")
        } else {
            ("", "")
        };
        let message = format!("nothing{yours} to {} in {}{why}", self.verb(), self.doc);
        let (steps, left_out) = self.steps(numbers, owners, head);
        // (never empty: the oldest state kept, or the steps around the head, are always listed)
        let mut way_out = WayOut::options(steps).unwrap_or_else(|| WayOut::one(json!({"version": head})));
        if let Some(call) = self.other_way(numbers, owners) {
            way_out = way_out.and_call(call);
        }
        let error = ToolError::refusal(Refusal::Refused, message, way_out);
        match left_out {
            Some(left_out) => error.with_detail("left_out", left_out),
            None => error,
        }
    }

    /// The document's steps, newest first, each with who made it (and `yours` for the
    /// caller's own): those plain redo brings back (`undone`), then those undo takes back,
    /// then the oldest state the history keeps (`oldest`). At most [`LISTED`] of each side,
    /// the nearest the head; how many of each were left out, when any were (tool rule 4).
    fn steps(&self, numbers: &Numbers, owners: &Owners, head: u64) -> (Vec<Value>, Option<Value>) {
        let entry = |step: &NumberedStep, undone: bool| {
            let mut out = Map::new();
            out.insert("version".into(), Value::from(step.after));
            if let Some(owner) = owners.of(step.after) {
                out.extend(owner.to_json());
            }
            if self.caller.is_some() && step.caller.as_deref() == self.caller {
                out.insert("yours".into(), Value::Bool(true));
            }
            if undone {
                out.insert("undone".into(), Value::Bool(true));
            }
            Value::Object(out)
        };
        // (each side's next is its last)
        let redo: Vec<&NumberedStep> = numbers.redo.iter().rev().take(LISTED).collect();
        let undo: Vec<&NumberedStep> = numbers.steps.iter().rev().take(LISTED).collect();
        let mut listed: Vec<Value> = redo.iter().rev().map(|s| entry(s, true)).collect();
        listed.extend(undo.iter().map(|s| entry(s, false)));
        let (redo_out, undo_out) = (numbers.redo.len() - redo.len(), numbers.steps.len() - undo.len());
        if undo_out == 0 {
            let oldest = numbers.steps.first().map_or(head, |s| s.before);
            listed.push(json!({"version": oldest, "oldest": true}));
        }
        let left_out = (redo_out + undo_out > 0).then(|| json!({"undo": undo_out, "redo": redo_out}));
        (listed, left_out)
    }

    /// The undo or redo the other way, when it would take or bring back a step: one of the
    /// caller's, or any for the session's own.
    fn other_way(&self, numbers: &Numbers, owners: &Owners) -> Option<TryCall> {
        let works = match (self.direction, self.caller) {
            (Direction::Redo, None) => !numbers.steps.is_empty(),
            (Direction::Undo, None) => !numbers.redo.is_empty(),
            (Direction::Redo, Some(caller)) => numbers
                .steps
                .last()
                .is_some_and(|s| s.caller.as_deref() == Some(caller)),
            (Direction::Undo, Some(caller)) => owners
                .latest_undo(caller)
                .is_some_and(|undo| numbers.redo.last().is_some_and(|next| next.after == undo.number)),
        };
        works.then(|| {
            let mut arguments = Map::new();
            arguments.insert("doc".into(), Value::from(self.doc));
            if self.direction == Direction::Undo {
                arguments.insert("redo".into(), Value::Bool(true));
            }
            TryCall {
                tool: UNDO.to_owned(),
                arguments,
                fill: Vec::new(),
            }
        })
    }

    /// Why the history did not move, in today's words.
    fn refused(&self, error: &SessionError, owners: &Owners) -> ToolError {
        match error {
            SessionError::Entangled { step, later } => self.entangled(*step, later, owners),
            SessionError::RedoNotNext { step } => ToolError::failure(
                Failure::Entangled,
                format!(
                    "your undone change to {} (version {step}) is not the next to redo: another caller undid a \
                     change since. Make your change again instead",
                    self.doc
                ),
            ),
            other => other.to_tool_error(),
        }
    }

    /// The caller's latest change (state `step`) is not the latest: `later` steps came after.
    fn entangled(&self, step: u64, later: &[u64], owners: &Owners) -> ToolError {
        let tool = owners
            .of(step)
            .map_or_else(|| "library".to_owned(), |o| o.tool.to_string());
        let last = later.last().copied().unwrap_or(step);
        let who = match owners.of(last) {
            None => "a change made outside the tools".to_owned(),
            Some(owner) if owner.is(self.caller) => format!("your own {}", owner.tool),
            Some(owner) => owner.describe(),
        };
        let more = if later.len() > 1 {
            format!(", one of {} later changes", later.len())
        } else {
            String::new()
        };
        let doc = self.doc;
        let message = format!(
            "your latest change to {doc} ({tool}, version {step}) is not the latest: {who} changed {doc} after it \
             (version {last}{more}), and undoing yours would take back what came after. Change it back with an \
             edit, or ask whoever made the later change to undo it first"
        );
        let later: Vec<Value> = later
            .iter()
            .map(|number| {
                let mut out = Map::new();
                out.insert("version".into(), Value::from(*number));
                if let Some(owner) = owners.of(*number) {
                    out.extend(owner.to_json());
                }
                Value::Object(out)
            })
            .collect();
        ToolError::failure(Failure::Entangled, message)
            .with_detail("version", json!(step))
            .with_detail("later", Value::Array(later))
    }
}
