//! Owns: a case that panics, as the differential harnesses read it: `{"panic": <what it
//! said>}` in place of its result, so a harness counts it (tools/gates.py fails on any) and
//! goes on with the next case instead of losing the run.
//!
//! Must not: be used by any library; it is a test instrument.

use serde_json::{json, Value};
use std::panic::{catch_unwind, AssertUnwindSafe};

/// What a panic said, where it carries text.
fn said(panic: &(dyn std::any::Any + Send)) -> String {
    panic
        .downcast_ref::<&str>()
        .map(|s| (*s).to_owned())
        .or_else(|| panic.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "a panic without a message".to_owned())
}

/// `case()`'s result, or `{"panic": ...}` where it panicked.
pub(crate) fn caught(case: impl FnOnce() -> Value) -> Value {
    catch_unwind(AssertUnwindSafe(case)).unwrap_or_else(|panic| json!({"panic": said(&*panic)}))
}

/// A session reply's panic (the session answered it as an `internal` failure, and says what
/// it said in its application part), as the harnesses read one.
pub(crate) fn of_reply<T>(reply: &ooxml_tools::ToolResult<T>) -> Option<Value> {
    reply.app.panic.as_ref().map(|what| json!({"panic": what}))
}
