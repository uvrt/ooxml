//! Owns: `read_batch` (today's, ooxml-edit's `Dispatcher._run_reads`): reading calls run one
//! after another, each answered as the same call alone would be -- within its own reply
//! budget -- and the answers together within the batch's; what does not fit, and every op
//! the batch's deadline stops, listed with the call that asks for it (tool rule 4); an op
//! that fails answered with its error while the others run.
//!
//! Must not: run a changing tool (the session serves reading tools only so far). Every op
//! reads a document at the version the batch's first op on it read, as today's batch holds
//! its documents for all its ops; nothing is held after the batch.

use crate::session::Pinned;
use crate::{CallContext, Session};
use ooxml_tools::{compact, Done, Failure, Outcome, ToolArgs, ToolError, ToolResult, MAX_RESULT_CHARS};
use serde_json::{json, Map, Value};
use std::time::Duration;

/// One reading call of a batch.
#[derive(ToolArgs, Debug, Clone, PartialEq)]
pub struct ReadOp {
    // No description: today's definition gives the op's tool none.
    pub(crate) tool: String,
    /// That tool's arguments.
    pub(crate) arguments: Map<String, Value>,
}

/// `read_batch`'s arguments.
#[derive(ToolArgs, Debug, Clone, PartialEq)]
#[tool(
    name = "read_batch",
    description = include_str!("read_batch.md"),
    documents(),
    strict = false,
    batchable = false
)]
pub struct ReadBatch {
    /// The reading calls, at most 200.
    #[arg(min_items = 1)]
    pub ops: Vec<ReadOp>,
}

/// The tool's name.
pub const READ_BATCH: &str = "read_batch";

/// Ops a batch may hold, as `Limits.max_batch_ops`.
const MAX_OPS: usize = 200;

/// The batch's reply budget, as `Limits.max_read_batch_chars`.
pub const MAX_READ_BATCH_CHARS: usize = 96_000;

/// What the reply needs besides its ops' answers, in characters.
const MARGIN: usize = 2_000;

/// How long a batch may take, as `Limits.batch_timeout`.
pub const BATCH_TIMEOUT: Duration = Duration::from_secs(120);

/// An op's answer: its result as the model reads a lone call's -- within `limit` -- without
/// the empty fields a reading call never fills, led by its index and tool; `next`, the call
/// for its next page, where it has one and its tool takes a cursor.
fn answer(index: usize, tool: &str, result: &ToolResult<Value>, op: &Value, paged: bool) -> Map<String, Value> {
    let envelope: Value = serde_json::from_str(&result.to_text(Some(MAX_RESULT_CHARS))).unwrap_or(Value::Null);
    let mut out = Map::new();
    out.insert("op".into(), Value::from(index));
    out.insert("tool".into(), Value::from(tool));
    if let Value::Object(envelope) = envelope {
        for (key, value) in envelope {
            let empty = match &value {
                Value::Null => true,
                Value::Array(a) => a.is_empty(),
                Value::Object(o) => o.is_empty(),
                Value::String(s) => s.is_empty(),
                _ => false,
            };
            if key != "ok" && !empty {
                out.insert(key, value);
            }
        }
    }
    let cursor = match &result.outcome {
        Outcome::Done(done) => done.next_cursor.clone(),
        Outcome::Failed(_) => None,
    };
    if let (Some(cursor), true, Some(Value::Object(arguments))) = (cursor, paged, op.get("arguments")) {
        let mut arguments = arguments.clone();
        arguments.insert("cursor".into(), Value::from(cursor));
        out.insert("next".into(), json!({"tool": tool, "arguments": arguments}));
    }
    out
}

/// Why an op was left out: over the budget, or past the deadline.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Left {
    Budget,
    Deadline,
}

impl Session {
    /// `read_batch` with `arguments` (already validated against its definition).
    pub(crate) fn read_batch(&self, arguments: &Value, context: &CallContext) -> Result<Done<Value>, ToolError> {
        let ops: Vec<Value> = arguments
            .get("ops")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        if ops.len() > MAX_OPS {
            return Err(ToolError::failure(
                Failure::Limit,
                format!(
                    "{} ops; at most {MAX_OPS} per read_batch: send the rest in another",
                    ops.len()
                ),
            )
            .at("ops"));
        }
        let cap = MAX_READ_BATCH_CHARS.max(MAX_RESULT_CHARS);
        let clock = &*self.context.clock;
        let started = clock.monotonic();
        let mut answers: Vec<(Map<String, Value>, usize, bool)> = Vec::new();
        let mut left: Vec<(usize, Left)> = Vec::new();
        let mut used = MARGIN;
        let mut pinned = Pinned::new();
        for (index, op) in ops.iter().enumerate() {
            if clock.monotonic().saturating_sub(started) > BATCH_TIMEOUT {
                left.push((index, Left::Deadline));
                continue;
            }
            let given = op.get("tool").and_then(Value::as_str).unwrap_or("");
            let arguments = op.get("arguments").cloned().unwrap_or_else(|| json!({}));
            let result = self.call_one(given, arguments, context, READ_BATCH, Some(&mut pinned));
            let paged = self
                .registry()
                .named(given)
                .is_some_and(|t| t.spec().arguments.property("cursor").is_some());
            let one = answer(index, given, &result, op, paged);
            let size = compact(&Value::Object(one.clone())).chars().count();
            if used + size > cap {
                left.push((index, Left::Budget));
                continue;
            }
            used += size;
            answers.push((one, index, result.is_error()));
        }
        let mut combined = combine(&ops, &answers, &left, cap);
        while !answers.is_empty() && compact(&Value::Object(envelope(&combined))).chars().count() > cap {
            if let Some((_, index, _)) = answers.pop() {
                left.push((index, Left::Budget));
                left.sort_by_key(|(k, _)| *k);
            }
            combined = combine(&ops, &answers, &left, cap);
        }
        Ok(combined)
    }
}

/// What the envelope of `done` holds, for its size.
fn envelope(done: &Done<Value>) -> Map<String, Value> {
    ToolResult::done(done.clone()).to_json()
}

/// The batch's result: the answers in op order, then what was left out.
fn combine(
    ops: &[Value],
    answers: &[(Map<String, Value>, usize, bool)],
    left: &[(usize, Left)],
    cap: usize,
) -> Done<Value> {
    let failed: Vec<&Map<String, Value>> = answers.iter().filter(|(_, _, f)| *f).map(|(a, _, _)| a).collect();
    let mut said = vec![format!("Read {} of {} op(s)", answers.len() - failed.len(), ops.len())];
    if !failed.is_empty() {
        let shown: Vec<String> = failed
            .iter()
            .take(3)
            .map(|a| {
                let tool = a
                    .get("tool")
                    .and_then(Value::as_str)
                    .filter(|t| !t.is_empty())
                    .unwrap_or("?");
                let message = a
                    .get("error")
                    .and_then(|e| e.get("message"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                format!(
                    "ops[{}] ({tool}): {message}",
                    a.get("op").cloned().unwrap_or(Value::Null)
                )
            })
            .collect();
        let more = if failed.len() > 3 {
            format!("; and {} more", failed.len() - 3)
        } else {
            String::new()
        };
        said.push(format!("{} failed: {}{more}", failed.len(), shown.join("; ")));
    }
    let mut data = Map::new();
    data.insert(
        "ops".into(),
        Value::Array(answers.iter().map(|(a, _, _)| Value::Object(a.clone())).collect()),
    );
    if !left.is_empty() {
        let thousands = thousands(cap);
        let timeout = BATCH_TIMEOUT.as_secs();
        let why = |w: Left| match w {
            Left::Budget => format!("over the reply's {thousands}-character budget"),
            Left::Deadline => format!("not run: past the {timeout} s deadline"),
        };
        data.insert(
            "not_returned".into(),
            Value::Array(left.iter().map(|(k, w)| json!({"op": k, "why": why(*w)})).collect()),
        );
        let resend: Vec<Value> = left.iter().filter_map(|(k, _)| ops.get(*k).cloned()).collect();
        data.insert(
            "resend".into(),
            json!({"tool": READ_BATCH, "arguments": {"ops": resend}}),
        );
        let budget = left.iter().filter(|(_, w)| *w == Left::Budget).count();
        let late = left.len() - budget;
        let mut parts = Vec::new();
        if budget > 0 {
            parts.push(format!("{budget} did not fit the {thousands}-character reply"));
        }
        if late > 0 {
            parts.push(format!("{late} not run before the {timeout} s deadline"));
        }
        said.push(format!(
            "Not returned: {}; send data.resend for them",
            parts.join(" and ")
        ));
    }
    let mut done = Done::new(said.join(". "));
    done.data = Some(Value::Object(data));
    done
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
