//! Owns: a panic inside a call ([`Session::call`], [`Session::edit`], a batch's op) answered
//! at the session's edge as the call's `internal` failure -- "the call failed inside the
//! library", whether it changed the document, the call again as its way out (retry, else
//! report `details.trace`) -- its message kept for the application only
//! ([`ooxml_tools::AppOnly::panic`]), and the `panics{tool}` counter
//! ([`crate::SessionStats::panics`]).
//!
//! Must not: let a panic reach the host (every host, the oracles' binaries too, gets the
//! reply), or leave one uncounted.

use crate::Session;
use ooxml_tools::{ToolError, ToolResult, TryCall, WayOut};
use serde_json::{Map, Value};
use std::any::Any;
use std::sync::PoisonError;

/// The most characters of a panic's message the reply's trace keeps.
const TRACE_CHARS: usize = 200;

/// What a panic says: its message, where it carries one as text.
fn said(panic: &(dyn Any + Send)) -> String {
    panic
        .downcast_ref::<&str>()
        .map(|s| (*s).to_owned())
        .or_else(|| panic.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "a panic without a message".to_owned())
}

/// The call again, as its way out: its arguments as given (an object, or its JSON text).
fn again(name: &str, arguments: Value) -> TryCall {
    let arguments = match arguments {
        Value::Object(map) => map,
        Value::String(text) => match serde_json::from_str(&text) {
            Ok(Value::Object(map)) => map,
            _ => Map::new(),
        },
        _ => Map::new(),
    };
    TryCall {
        tool: name.to_owned(),
        arguments,
        fill: Vec::new(),
    }
}

impl Session {
    /// The reply to a call of `name` with `arguments` that panicked with `panic`, counted in
    /// `panics{name}`; `changed`: whether the call's edits had changed the document before it
    /// did (they stay, as the call's step).
    pub(crate) fn panicked(
        &self,
        name: &str,
        arguments: Value,
        panic: &(dyn Any + Send),
        changed: bool,
    ) -> ToolResult<Value> {
        let what = said(panic);
        let seen = {
            let mut counts = self.counts.panics.lock().unwrap_or_else(PoisonError::into_inner);
            *counts.entry(name.to_owned()).or_default() += 1;
            counts.values().sum::<u64>()
        };
        let kept: String = what.chars().take(TRACE_CHARS).collect();
        let message = if changed {
            "the call failed inside the library; what it changed before it failed stays, as its step"
        } else {
            "the call failed inside the library; nothing changed"
        };
        let error = ToolError::internal(
            format!("{message}. Retry the call (try_call); if it fails again, report details.trace"),
            &format!("panic:{name}:{seen}: {kept}"),
        )
        .with_way_out(WayOut::call(again(name, arguments)));
        let mut result = ToolResult::failed(error);
        result.app.panic = Some(what);
        result
    }
}
