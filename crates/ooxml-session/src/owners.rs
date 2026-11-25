//! Owns: who made each step of a document ([`Owners`], as docx-agent's session keeps a
//! document's owners): the tool and the caller's context -- its keys in the order the host
//! gave them, as a reply names the caller (`agent=b`) -- by the state number the step made
//! ([`crate::Opened::number`]), noted as a call, a bridged step or a step group's close is
//! numbered; and each caller's own plain undos, newest last, which its own redo brings back
//! ([`crate::history`]).
//!
//! Must not: number a step (the store does), word a reply (`undo` does), or keep an own
//! undo across a new step: a new step drops plain redo, so it drops them too.

use crate::CallContext;
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

/// Who made a step: the tool, and the caller (`None`: the session's own, no caller).
#[derive(Debug, Clone, PartialEq)]
pub struct StepOwner {
    /// The tool that made it (`library` for a host's step of no tool of its own).
    pub(crate) tool: Arc<str>,
    /// The caller's key, as the store keeps it with the step (`None`: no caller).
    pub(crate) caller: Option<Arc<str>>,
    /// The caller's context, its keys in the order given (empty: no caller).
    pub(crate) context: Arc<Map<String, Value>>,
}

impl StepOwner {
    /// `tool` made by `context`'s caller.
    pub(crate) fn of(tool: &str, context: &CallContext) -> Self {
        let caller = context.caller.name().map(Arc::from);
        let context = context.caller_context.clone().unwrap_or_else(|| {
            // no context given: the caller key's own fields (sorted), or the key as its name
            let fields = caller.as_deref().map(|key| match serde_json::from_str::<Value>(key) {
                Ok(Value::Object(fields)) => fields,
                _ => Map::from_iter([("caller".to_owned(), Value::from(key))]),
            });
            Arc::new(fields.unwrap_or_default())
        });
        Self {
            tool: Arc::from(tool),
            caller,
            context,
        }
    }

    /// Whether `caller` (a caller's key) made it.
    pub(crate) fn is(&self, caller: Option<&str>) -> bool {
        self.caller.as_deref() == caller
    }

    /// Who made it, in words: `word_insert_text by agent=b`.
    pub(crate) fn describe(&self) -> String {
        if self.context.is_empty() {
            return format!("{} (a call with no caller context)", self.tool);
        }
        let fields: Vec<String> = self
            .context
            .iter()
            .map(|(key, value)| format!("{key}={}", ooxml_tools::pyfmt::str_of(value)))
            .collect();
        format!("{} by {}", self.tool, fields.join(", "))
    }

    /// Who made it, as a reply's details say it: `{"tool": ..., "context": {...}}`.
    pub(crate) fn to_json(&self) -> Map<String, Value> {
        let mut out = Map::new();
        out.insert("tool".into(), Value::from(&*self.tool));
        out.insert("context".into(), Value::Object((*self.context).clone()));
        out
    }
}

/// A caller's own plain undo, for its redo: the caller and the step it took back (the state
/// number the step made, and the store's version of it).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OwnUndo {
    pub(crate) caller: Arc<str>,
    pub(crate) number: u64,
    pub(crate) seq: u64,
}

#[derive(Debug, Default)]
struct Kept {
    by_number: BTreeMap<u64, StepOwner>,
    own_undos: Vec<OwnUndo>,
}

/// Who made each of a document's steps, and its callers' own undos.
#[derive(Debug, Default)]
pub struct Owners(Mutex<Kept>);

/// Owners kept before the record is thinned to the steps the history still holds.
const KEEP: usize = 256;

impl Owners {
    // A poisoned record is still consistent: every change to it is one insert or remove.
    fn kept(&self) -> MutexGuard<'_, Kept> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// A new step numbered `number`, made by `owner` (`None`: by no tool, a change from
    /// outside the tools); `held`: the numbers the history still holds, which the record
    /// is thinned to once it grows. Every own undo goes: the new step dropped plain redo.
    pub(crate) fn stepped(&self, number: u64, owner: Option<StepOwner>, held: impl FnOnce() -> Vec<u64>) {
        let mut kept = self.kept();
        kept.own_undos.clear();
        match owner {
            Some(owner) => kept.by_number.insert(number, owner),
            None => kept.by_number.remove(&number),
        };
        if kept.by_number.len() > KEEP {
            let held = held();
            kept.by_number.retain(|n, _| *n == number || held.contains(n));
        }
    }

    /// Who made the step numbered `number`, if a tool did.
    pub(crate) fn of(&self, number: u64) -> Option<StepOwner> {
        self.kept().by_number.get(&number).cloned()
    }

    /// `undo`: `caller` took back its own step, the latest of the document's.
    pub(crate) fn undone(&self, undo: OwnUndo) {
        self.kept().own_undos.push(undo);
    }

    /// `caller`'s latest own undo (its redo's next), if any.
    pub(crate) fn latest_undo(&self, caller: &str) -> Option<OwnUndo> {
        let kept = self.kept();
        kept.own_undos.iter().rev().find(|u| &*u.caller == caller).cloned()
    }

    /// `undo` brought back, or gone with the redo it waited on.
    pub(crate) fn forget(&self, undo: &OwnUndo) {
        let mut kept = self.kept();
        if let Some(index) = kept.own_undos.iter().rposition(|u| u == undo) {
            kept.own_undos.remove(index);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ooxml_store::Caller;

    fn context(caller: Option<&str>, given: Option<Value>) -> CallContext {
        CallContext {
            caller: caller.map_or_else(Caller::anonymous, Caller::named),
            caller_context: given.and_then(|v| match v {
                Value::Object(fields) => Some(Arc::new(fields)),
                _ => None,
            }),
            ..CallContext::default()
        }
    }

    #[test]
    fn a_caller_is_described_in_the_order_its_context_was_given() {
        let given = serde_json::json!({"turn": 3, "agent": "b"});
        let owner = StepOwner::of(
            "word_insert_text",
            &context(Some(r#"{"agent":"b","turn":3}"#), Some(given)),
        );
        assert_eq!(owner.describe(), "word_insert_text by turn=3, agent=b");
        assert!(owner.is(Some(r#"{"agent":"b","turn":3}"#)));
        let sorted = StepOwner::of("word_delete", &context(Some(r#"{"agent":"b","turn":3}"#), None));
        assert_eq!(sorted.describe(), "word_delete by agent=b, turn=3");
        let plain = StepOwner::of("word_delete", &context(Some("writer-2"), None));
        assert_eq!(plain.describe(), "word_delete by caller=writer-2");
        let own = StepOwner::of("word_delete", &context(None, None));
        assert_eq!(own.describe(), "word_delete (a call with no caller context)");
        assert_eq!(
            Value::Object(own.to_json()),
            serde_json::json!({"tool": "word_delete", "context": {}})
        );
    }

    #[test]
    fn a_new_step_drops_the_own_undos_and_the_record_keeps_what_the_history_holds() {
        let owners = Owners::default();
        let owner = || Some(StepOwner::of("t", &context(Some("a"), None)));
        owners.stepped(1, owner(), Vec::new);
        owners.undone(OwnUndo {
            caller: Arc::from("a"),
            number: 1,
            seq: 1,
        });
        assert!(owners.latest_undo("a").is_some());
        assert!(owners.latest_undo("b").is_none());
        owners.stepped(2, owner(), Vec::new);
        assert!(owners.latest_undo("a").is_none());
        for number in 3..=(KEEP as u64 + 2) {
            owners.stepped(number, owner(), || vec![2]);
        }
        assert!(owners.of(1).is_none());
        assert!(owners.of(2).is_some());
        owners.stepped(2, None, Vec::new);
        assert!(owners.of(2).is_none());
    }
}
