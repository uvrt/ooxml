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
use ooxml_session::batch::{answer, combine, envelope, Left, BATCH_TIMEOUT, MARGIN, MAX_OPS, READ_BATCH};
use ooxml_session::DocId;
use ooxml_tools::roles::RoleSet;
use ooxml_tools::{compact, Failure, Outcome, Refusal, ToolError, ToolResult, TryCall, WayOut};
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
        if ops.len() > MAX_OPS {
            let message = format!(
                "{} ops; at most {MAX_OPS} per read_batch: send the rest in another",
                ops.len()
            );
            return ToolResult::failed(ToolError::failure(Failure::Limit, message).at("ops"));
        }
        let own = self.host.settings.limits.result_chars;
        let cap = self.host.settings.read_batch_chars.max(own);
        let clock = &self.session.context().clock;
        let started = clock.monotonic();
        let mut answers: Vec<(Map<String, Value>, usize, bool)> = Vec::new();
        // each answered op's images: their descriptors and bytes
        let mut shown: Vec<(usize, Vec<Value>, Vec<ooxml_tools::File>)> = Vec::new();
        let mut pictured: Vec<(usize, usize)> = Vec::new();
        let mut left: Vec<(usize, Left)> = Vec::new();
        let mut used = MARGIN;
        let mut pinned = Vec::new();
        // (the batch names the first document an op names, whatever came of the op)
        let first_doc = ops
            .iter()
            .find_map(|op| op.pointer("/arguments/doc").and_then(Value::as_str).map(str::to_owned));
        for (index, op) in ops.iter().enumerate() {
            if clock.monotonic().saturating_sub(started) > BATCH_TIMEOUT {
                left.push((index, Left::Deadline));
                continue;
            }
            let (mut result, tool, paged, _docs) = self.read_op(op, roles, caller, &mut pinned);
            let one = answer(index, &tool, &result, op, own, paged);
            let images = match &result.outcome {
                Outcome::Done(done) => done.images.clone(),
                Outcome::Failed(_) => Vec::new(),
            };
            let counted = image_chars(&images);
            if counted > 0 {
                pictured.push((index, counted));
            }
            let size = compact(&Value::Object(one.clone())).chars().count() + counted;
            if used + size > cap {
                left.push((index, Left::Budget));
                // (its images never sent: given back to the session's budget)
                self.stores().images.release(images.len());
                continue;
            }
            used += size;
            answers.push((one, index, result.is_error()));
            shown.push((index, images, std::mem::take(&mut result.app.files)));
        }
        let read = |answers: &[(Map<String, Value>, usize, bool)], left: &[(usize, Left)]| {
            let mut done = combine(&ops, answers, left, cap);
            said_images(&mut done, answers, &shown, &pictured);
            done
        };
        let mut combined = read(&answers, &left);
        let total = |done: &ooxml_tools::Done<Value>| {
            compact(&Value::Object(envelope(done))).chars().count() + image_chars(&done.images)
        };
        while !answers.is_empty() && total(&combined) > cap {
            if let Some((_, index, _)) = answers.pop() {
                let images = shown.iter().find(|(k, ..)| *k == index).map_or(0, |(_, i, _)| i.len());
                self.stores().images.release(images);
                left.push((index, Left::Budget));
                left.sort_by_key(|(k, _)| *k);
            }
            combined = read(&answers, &left);
        }
        let files: Vec<ooxml_tools::File> = shown
            .into_iter()
            .filter(|(k, ..)| answers.iter().any(|(_, i, _)| i == k))
            .flat_map(|(_, _, files)| files)
            .collect();
        self.batch_result(combined, files, cap, first_doc)
    }

    /// The batch's reply: its answers' images' bytes for the application, its budget, the
    /// first document its ops name at its version now.
    fn batch_result(
        &self,
        combined: ooxml_tools::Done<Value>,
        files: Vec<ooxml_tools::File>,
        cap: usize,
        first_doc: Option<String>,
    ) -> ToolResult<Value> {
        let mut result = ToolResult::done(combined);
        result.app.files = files;
        result.app.budget = Some(cap);
        if let Some(doc) = first_doc {
            result.version = DocId::parse(&doc)
                .and_then(|id| self.session.snapshot(&id).ok())
                .map(|head| head.number());
            result.doc = Some(doc);
        }
        result
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

/// Characters an image counts for in a `read_batch`'s reply, per token it costs (the rate
/// `Limits.max_read_batch_chars` is set at: 96,000 characters, about 24,000 tokens).
const CHARS_PER_IMAGE_TOKEN: usize = 4;

/// What images (their descriptors) count toward a `read_batch`'s budget (round 35, R20):
/// each its tokens -- the larger of Claude's and OpenAI's count -- times
/// [`CHARS_PER_IMAGE_TOKEN`].
fn image_chars(images: &[Value]) -> usize {
    images
        .iter()
        .map(|image| {
            let tokens = |who: &str| {
                image
                    .pointer(&format!("/tokens/{who}"))
                    .and_then(Value::as_u64)
                    .unwrap_or(0)
            };
            let most = tokens("anthropic").max(tokens("openai"));
            CHARS_PER_IMAGE_TOKEN * usize::try_from(most).unwrap_or(usize::MAX)
        })
        .sum()
}

/// The batch's reply with the images of its answers, in op order, its summary saying they
/// follow, and each op left out over the budget saying what its images count.
fn said_images(
    done: &mut ooxml_tools::Done<Value>,
    answers: &[(Map<String, Value>, usize, bool)],
    shown: &[(usize, Vec<Value>, Vec<ooxml_tools::File>)],
    pictured: &[(usize, usize)],
) {
    for (_, index, _) in answers {
        if let Some((_, images, _)) = shown.iter().find(|(k, ..)| k == index) {
            done.images.extend(images.iter().cloned());
        }
    }
    if let Some(Value::Object(data)) = &mut done.data {
        if let Some(Value::Array(left)) = data.get_mut("not_returned") {
            for entry in left.iter_mut() {
                let op = entry
                    .get("op")
                    .and_then(Value::as_u64)
                    .and_then(|k| usize::try_from(k).ok());
                let counted = op.and_then(|k| pictured.iter().find(|(i, _)| *i == k)).map(|(_, n)| *n);
                let budget = entry
                    .get("why")
                    .and_then(Value::as_str)
                    .is_some_and(|w| w.starts_with("over the reply"));
                if let (Some(counted), true, Some(why)) = (counted, budget, entry.get("why").and_then(Value::as_str)) {
                    let why = format!("{why} (its images count {})", thousands(counted));
                    entry["why"] = Value::from(why);
                }
            }
        }
    }
    if !done.images.is_empty() {
        done.summary.push_str(&format!(
            ". {} image(s) follow, in op order, each after its op's caption",
            done.images.len()
        ));
    }
}

/// `n` with thousands separated by commas (Python's `{n:,}`).
fn thousands(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (k, c) in digits.chars().enumerate() {
        if k > 0 && (digits.len() - k).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}
