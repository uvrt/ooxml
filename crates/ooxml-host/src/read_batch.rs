//! Owns: `read_batch` in the host (design phase 3, section 2: moved from the session's
//! dispatch): reading calls run one after another, each through the host's own pipeline as
//! the same call alone would run (its name read, its roles, the one acceptance, its refs),
//! each answered within its own reply budget and the answers together within the batch's;
//! an op that would change a document refused, pointing at `batch`; every op reads a
//! document at the version the batch's first op on it read. What did not fit, and every op
//! the deadline stopped, is listed with the call that asks for it (`ooxml-session`'s
//! [`ooxml_session::batch`] words the answers and the reply).
//!
//! Must not: run a changing call, or hold a document between the batch's ops.

use crate::catalogue::Catalogue;
use crate::pipeline::Going;
use crate::session::HostSession;
use ooxml_session::batch::{answer, combine, envelope, passed, Left, MARGIN, MAX_READ_BATCH_CHARS, READ_BATCH};
use ooxml_session::DocId;
use ooxml_tools::roles::RoleSet;
use ooxml_tools::{compact, Failure, Refusal, ToolError, ToolResult, TryCall, WayOut};
use serde_json::{json, Map, Value};

impl HostSession {
    /// `read_batch` with `arguments` taken as meant.
    pub(crate) fn read_batch(
        &self,
        arguments: &Map<String, Value>,
        roles: RoleSet,
        caller: Option<&Map<String, Value>>,
    ) -> ToolResult<Value> {
        let ops: Vec<Value> = arguments
            .get("ops")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let (most, timeout) = (self.host.settings.max_batch_ops, self.host.settings.batch_timeout);
        // (the document an op names first, which a reply that ran none of them names)
        let named = ops.iter().find_map(|op| {
            let doc = op.get("arguments")?.get("doc")?.as_str()?;
            DocId::parse(doc)
                .filter(|id| self.session.kind_of(id).is_ok())
                .map(|_| doc.to_owned())
        });
        if ops.len() > most {
            let message = format!(
                "{} ops; at most {most} per read_batch: send the rest in another",
                ops.len()
            );
            let mut result = ToolResult::failed(ToolError::failure(Failure::Limit, message).at("ops"));
            self.name_doc(&mut result, named);
            return result;
        }
        let own = self.host.settings.limits.result_chars;
        let cap = MAX_READ_BATCH_CHARS.max(own);
        let clock = &self.session.context().clock;
        let started = clock.monotonic();
        let mut answers: Vec<(Map<String, Value>, usize, bool)> = Vec::new();
        let mut left: Vec<(usize, Left)> = Vec::new();
        let mut used = MARGIN;
        let mut pinned = Vec::new();
        let mut first_doc: Option<String> = None;
        for (index, op) in ops.iter().enumerate() {
            if passed(clock.monotonic().saturating_sub(started), timeout) {
                left.push((index, Left::Deadline));
                continue;
            }
            let (result, tool, paged, docs) = self.read_op(op, roles, caller, &mut pinned);
            if first_doc.is_none() {
                first_doc = docs.first().cloned();
            }
            let one = answer(index, &tool, &result, op, own, paged);
            let size = compact(&Value::Object(one.clone())).chars().count();
            if used + size > cap {
                left.push((index, Left::Budget));
                continue;
            }
            used += size;
            answers.push((one, index, result.is_error()));
        }
        let mut combined = combine(&ops, &answers, &left, cap, timeout);
        while !answers.is_empty() && compact(&Value::Object(envelope(&combined))).chars().count() > cap {
            if let Some((_, index, _)) = answers.pop() {
                left.push((index, Left::Budget));
                left.sort_by_key(|(k, _)| *k);
            }
            combined = combine(&ops, &answers, &left, cap, timeout);
        }
        let mut result = ToolResult::done(combined);
        result.app.budget = Some(cap);
        self.name_doc(&mut result, first_doc.or(named));
        result
    }

    /// `result` naming `doc` at its head, where there is one.
    fn name_doc(&self, result: &mut ToolResult<Value>, doc: Option<String>) {
        if let Some(doc) = doc {
            result.version = DocId::parse(&doc)
                .and_then(|id| self.session.snapshot(&id).ok())
                .map(|head| head.number());
            result.doc = Some(doc);
        }
    }

    /// One op: its result, its tool's name as read, whether its tool takes a cursor, and the
    /// documents it named.
    fn read_op(
        &self,
        op: &Value,
        roles: RoleSet,
        caller: Option<&Map<String, Value>>,
        pinned: &mut Vec<ooxml_session::Opened>,
    ) -> (ToolResult<Value>, String, bool, Vec<String>) {
        let catalogue = &self.host.catalogue;
        let given = op.get("tool").and_then(Value::as_str).unwrap_or("");
        let arguments = op.get("arguments").cloned().unwrap_or_else(|| json!({}));
        let retry = TryCall {
            tool: given.to_owned(),
            arguments: arguments.as_object().cloned().unwrap_or_default(),
            fill: Vec::new(),
        };
        let mut going = Going::new(given, arguments.clone());
        let name = catalogue.tool_named(given, &mut going.notes, true);
        let failed = |error: ToolError, notes: Vec<String>| {
            let mut result = ToolResult::failed(error);
            result.warnings = notes;
            result
        };
        let Some(entry) = catalogue.get(&name) else {
            let error = catalogue.no_tool(given, roles, true, READ_BATCH, &retry);
            return (failed(error, going.notes), given.to_owned(), false, Vec::new());
        };
        let paged = entry.spec.arguments.property("cursor").is_some();
        let refused = |message: String, options: Vec<String>| {
            let way_out = WayOut::names(options).unwrap_or_else(|| WayOut::call(retry.clone()));
            ToolError::refusal(Refusal::InvalidArguments, message, way_out).at("tool")
        };
        if !entry.runs_in(READ_BATCH) {
            let error = refused(
                format!("{name} cannot run inside a read_batch; call it on its own"),
                Vec::new(),
            );
            return (failed(error, going.notes), name, paged, Vec::new());
        }
        let checked = Catalogue::check_role(entry, None, roles, Some("tool"), &retry)
            .and_then(|()| {
                entry.spec.accept(arguments).map_err(|error| {
                    let field = error
                        .field()
                        .map_or_else(|| "arguments".to_owned(), |f| format!("arguments.{f}"));
                    error.at(field)
                })
            })
            .and_then(|accepted| {
                Catalogue::check_role(entry, Some(&accepted.arguments), roles, None, &retry).map(|()| accepted)
            });
        let accepted = match checked {
            Ok(accepted) => accepted,
            Err(error) => return (failed(error, going.notes), name, paged, Vec::new()),
        };
        going.notes.extend(accepted.notes);
        if entry.changes(&accepted.arguments) {
            let message = format!(
                "{name} changes the document, and read_batch only reads: send this op in batch (with the other \
                 changes), or call it on its own"
            );
            let error = ToolError::refusal(
                Refusal::Refused,
                message,
                WayOut::names(["batch".to_owned(), name.clone()]).unwrap_or_else(|| WayOut::call(retry.clone())),
            )
            .at("tool");
            return (failed(error, going.notes), name, paged, Vec::new());
        }
        going.pinned = Some(std::mem::take(pinned));
        let mut result = self.run_format(entry, accepted.arguments, caller, &mut going, &retry);
        *pinned = going.pinned.take().unwrap_or_default();
        if result.is_error() {
            // (an op that failed read no version of its document)
            result.doc = None;
            result.version = None;
        }
        let mut warnings = std::mem::take(&mut going.notes);
        warnings.append(&mut result.warnings);
        result.warnings = warnings;
        result.app.tool.clone_from(&name);
        (result, name, paged, going.docs)
    }
}
