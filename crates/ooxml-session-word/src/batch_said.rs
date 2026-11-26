//! Owns: what a Word op of a host's batch says of the batch (docx-agent's adapters around a
//! batch's ops): a `check` that laid the document out between the batch's changes
//! (`reads.py`'s `BATCH_CHECK`, worded for the batch it ran in), and, once the batch is
//! published, a table `word_format_table` kept together that the document after the batch
//! draws taller than a page, or cannot be laid out to (`objects.py`'s `_later_taller`) --
//! measured once, after the batch's last op, not after each op.
//!
//! Must not: change the document, or measure what the op left before the batch is published.

use crate::block_pages::{block_pages, BlockAt};
use crate::word::Word;
use ooxml_session::{Deferred, FormatRun};
use ooxml_tools::{Outcome, ToolResult};
use ooxml_tools_word::WordLayout;
use serde_json::Value;

/// A check that laid the document out between the changes of a batch holding it (several
/// documents' batch, or one run behind the gate after conflicts): docx-agent's words.
const CHECK_HOLDING: &str = "check laid the document out between the batch's changes, holding it: other calls \
     waited. Put check after the batch's last change (or call it alone): there it lets the document go while it lays \
     out";

/// A check that laid the document out between the changes of an optimistic batch, which
/// holds nothing (design phase 3 §4; user decision 2026-11-25, optimistic batches).
const CHECK_OPTIMISTIC: &str = "check laid the document out between the batch's changes, and lays it out again \
     each time the batch runs again. Put check after the batch's last change (or call it alone)";

/// What `run`, a `check` in a batch's transaction, says where it lays the version it reads
/// out (one laying out: `reflow` or `layout` asked) and that version is not the one `layout`
/// laid out last; `None` for any other call.
pub(crate) fn check_between(run: &FormatRun<'_>, layout: Option<&dyn WordLayout>) -> Option<&'static str> {
    let txn = run.txn?;
    if run.tool != "check" {
        return None;
    }
    let lays_out = |v: &Value| matches!(v.as_str(), Some("reflow" | "layout"));
    let wanted = match run.arguments.get("include") {
        Some(Value::Array(items)) => items.iter().any(lays_out),
        Some(one) => lays_out(one),
        None => false,
    };
    if !wanted || layout.is_some_and(|l| l.seed().laid_last(run.doc.package()).is_some()) {
        return None;
    }
    Some(if txn.gated() { CHECK_HOLDING } else { CHECK_OPTIMISTIC })
}

/// What `run`, an op of a host's batch that returned `result`, says once the batch is
/// published: for `word_format_table` keeping a table together, its pages then.
pub(crate) fn taller_later(run: &FormatRun<'_>, result: &ToolResult<Value>) -> Option<Deferred> {
    if run.tool != "word_format_table" || run.arguments.get("keep_together") != Some(&Value::Bool(true)) {
        return None;
    }
    let Outcome::Done(found) = &result.outcome else {
        return None;
    };
    let given = run.arguments.get("table")?.as_str()?;
    let table = found
        .renamed
        .iter()
        .find(|(old, _)| old == given)
        .map_or(given, |(_, new)| new.as_str())
        .to_owned();
    Some(Box::new(move |doc, _, context, cancel| {
        let Some(word) = Word::of(doc) else {
            return Vec::new();
        };
        let pages = match block_pages(word, std::slice::from_ref(&table), context, cancel) {
            Ok(Ok(at)) => match at.first() {
                Some(BlockAt::Outside) => None,
                Some(BlockAt::Pages(pages)) if !pages.is_empty() => Some(Ok(pages.clone())),
                _ => Some(Err(())),
            },
            Ok(Err(_)) | Err(_) => Some(Err(())),
        };
        let pages = pages.as_ref().map(|found| found.as_deref().map_err(|_| ()));
        ooxml_tools_word::kept_together_said(&table, pages)
            .into_iter()
            .collect()
    }))
}
