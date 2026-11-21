//! `word-reads <manifest.json>`: the Rust Word reading tools' replies (`describe`,
//! `word_read`) on documents, in the form `python/word_reads_diff.py` writes docx-agent's:
//! the summary, data, next cursor and total, or the refusal.
//!
//! The manifest is a JSON list of `{"file": path, "case": name, "tool": name, "arguments": {...}}`,
//! or `{"file", "case", "fact": name}` for one of `check`'s facts read from the document
//! alone ([`crate::facts`]; `"opened"`: the file as the session opened it, where the fact
//! compares with it). A `check` case may name the calls made before it in the same
//! session (`"before": [{"tool", "arguments"}]`: checks and edits): a sequence of versions,
//! each check measured from the one before (`since_last_check`).
//!
//! Must not: be used by any library; it is a test instrument.

use ooxml_base::context::{FaceData, FaceRequest, Timestamp};
use ooxml_base::{BaseError, CacheBudget, Cancel, Clock, Context, FontSource, Limits, Locale, PartStore};
use ooxml_opc::Package;
use ooxml_session::{CallContext, DocKind, Session, SessionConfig};
use ooxml_tools::{ArgType, Done, ToolError};
use ooxml_tools_word::Version;
use ooxml_word_model::docx::Versions;
use serde_json::{json, Value};
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

/// Never cancelled.
struct Never;

impl Cancel for Never {
    fn is_cancelled(&self) -> bool {
        false
    }
}

/// Run over a manifest: one JSON line per case on stdout.
///
/// # Errors
/// A message when the manifest cannot be read.
pub fn run(args: &[String]) -> Result<(), String> {
    let manifest = args.first().ok_or("usage: word-reads <manifest.json>")?;
    let text = std::fs::read_to_string(manifest).map_err(|e| e.to_string())?;
    let items: Vec<Value> = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    let mut opened: Option<(String, Package, Versions)> = None;
    for item in items {
        let file = item
            .get("file")
            .and_then(Value::as_str)
            .ok_or("an item without a file")?;
        let case = item.get("case").cloned().unwrap_or(Value::Null);
        let tool = item.get("tool").and_then(Value::as_str).unwrap_or("describe");
        let arguments = item.get("arguments").cloned().unwrap_or_else(|| json!({}));
        if opened.as_ref().is_none_or(|(f, _, _)| f != file) {
            let bytes = std::fs::read(file).map_err(|e| format!("{file}: {e}"))?;
            let package = Package::open(Arc::from(bytes), &Limits::default()).map_err(|e| format!("{file}: {e}"))?;
            opened = Some((file.to_owned(), package, Versions::new()));
        }
        let Some((_, package, versions)) = opened.as_ref() else {
            continue;
        };
        let name = Path::new(file).file_name().and_then(|n| n.to_str()).unwrap_or(file);
        let version = Version::new(name, package, 1, versions);
        let reply = if let Some(fact) = item.get("fact").and_then(Value::as_str) {
            crate::facts::reply(package, fact, item.get("opened").and_then(Value::as_str))
        } else if item.get("laid").is_some() {
            let fonts = item.get("fonts").unwrap_or(&Value::Null);
            crate::laid_facts::reply(package, fonts, item.get("min_size"), item.get("range"))
        } else if matches!(tool, "read_batch" | "check") {
            let before = item
                .get("before")
                .and_then(Value::as_array)
                .map_or(&[][..], Vec::as_slice);
            let limit = item.get("layout_timeout").and_then(Value::as_f64);
            through_session(tool, &arguments, (file, name), (item.get("fonts"), limit), before)
        } else {
            call(tool, &arguments, &version)
        };
        println!("{}", json!({"file": file, "case": case, "reply": reply}));
    }
    Ok(())
}

fn call(tool: &str, arguments: &Value, version: &Version<'_>) -> Value {
    let result = match tool {
        "describe" => ooxml_tools_word::Describe::from_arg(arguments, "")
            .map_err(|e| ToolError::internal(e.to_string(), "arguments"))
            .and_then(|a| ooxml_tools_word::describe(&a, version, &Never)),
        "word_read" => ooxml_tools_word::WordRead::from_arg(arguments, "")
            .map_err(|e| ToolError::internal(e.to_string(), "arguments"))
            .and_then(|a| ooxml_tools_word::word_read(&a, version, &Never)),
        "word_inspect" => ooxml_tools_word::WordInspect::from_arg(arguments, "")
            .map_err(|e| ToolError::internal(e.to_string(), "arguments"))
            .and_then(|a| ooxml_tools_word::word_inspect(&a, version, &Never)),
        "find_text" => ooxml_tools_word::FindText::from_arg(arguments, "")
            .map_err(|e| ToolError::internal(e.to_string(), "arguments"))
            .and_then(|a| ooxml_tools_word::find_text(&a, version, &Never)),
        other => Err(ToolError::internal(format!("no tool {other}"), "oracle")),
    };
    match result {
        Ok(done) => reply(&done),
        Err(error) => json!({"error": error.to_json()}),
    }
}

/// `tool` (`read_batch`, `check`) through a session holding the document as `d1`, which
/// owns it (a check's validation compares with the problems it opened with), handed a page
/// engine laying out with `fonts` (docx-agent's, as the case gives them) where given, within
/// `layout_timeout` seconds where given, after the calls `before` (each `{"tool",
/// "arguments"}`, through the same session and engine). A reply that says part of it is not
/// given yet (`unported`) is an error naming what; a call before it that fails is an error
/// naming that call.
fn through_session(
    tool: &str,
    arguments: &Value,
    (file, name): (&str, &str),
    (fonts, layout_timeout): (Option<&Value>, Option<f64>),
    before: &[Value],
) -> Value {
    let Ok(bytes) = std::fs::read(file) else {
        return json!({"error": {"code": "internal", "message": "unreadable"}});
    };
    let session = Session::new(SessionConfig::default(), session_context());
    if let Err(e) = session.open(Arc::from(bytes), DocKind::Docx, name) {
        return json!({"error": {"code": "internal", "message": e.to_string()}});
    }
    let context = CallContext {
        layout: fonts.map(|fonts| {
            Arc::new(crate::structure_tools::engine_pages(fonts)) as Arc<dyn ooxml_tools_word::WordLayout>
        }),
        layout_timeout: layout_timeout.and_then(|s| std::time::Duration::try_from_secs_f64(s).ok()),
        ..CallContext::default()
    };
    for (k, step) in before.iter().enumerate() {
        let name = step.get("tool").and_then(Value::as_str).unwrap_or_default();
        let args = step.get("arguments").cloned().unwrap_or_else(|| json!({}));
        let done = session.call(name, args, &context);
        // A call left to the host leaves the case to it, as the case's own call would.
        if let Some(why) = done.unported().first() {
            return json!({"error": {"code": "unported", "message": why}});
        }
        if let Some(error) = done.to_json().get("error") {
            return json!({"error": {"code": "before", "message": format!("call {k} ({name}): {error}")}});
        }
    }
    let result = session.call(tool, arguments.clone(), &context);
    if let Some(why) = result.unported().first() {
        return json!({"error": {"code": "unported", "message": why}});
    }
    let envelope = result.to_json();
    match envelope.get("error") {
        Some(error) => json!({"error": error}),
        None => json!({"summary": envelope.get("summary"), "data": envelope.get("data"),
                       "next_cursor": envelope.get("next_cursor"), "total": envelope.get("total")}),
    }
}

/// A session's ports for the oracle: no fonts, no stored blobs, the system's clock, an
/// unbounded cache budget.
pub(crate) fn session_context() -> Context {
    struct NoFonts;
    impl FontSource for NoFonts {
        fn face(&self, _: &FaceRequest) -> Result<Option<FaceData>, BaseError> {
            Ok(None)
        }
    }
    struct NoStore;
    impl PartStore for NoStore {
        fn load(&self, _: &str) -> Result<Option<Arc<[u8]>>, BaseError> {
            Ok(None)
        }
        fn store(&self, _: &str, _: Arc<[u8]>) -> Result<(), BaseError> {
            Ok(())
        }
    }
    struct Clocks(Instant);
    impl Clock for Clocks {
        fn now(&self) -> Timestamp {
            Timestamp { seconds: 0, nanos: 0 }
        }
        fn monotonic(&self) -> std::time::Duration {
            self.0.elapsed()
        }
    }
    struct Unbounded;
    impl CacheBudget for Unbounded {
        fn reserve(&self, _: usize) -> bool {
            true
        }
        fn release(&self, _: usize) {}
        fn used(&self) -> usize {
            0
        }
    }
    Context {
        fonts: Arc::new(NoFonts),
        parts: Arc::new(NoStore),
        clock: Arc::new(Clocks(Instant::now())),
        budget: Arc::new(Unbounded),
        cancel: Arc::new(Never),
        limits: Limits::default(),
        locale: Locale::default(),
    }
}

fn reply(done: &Done<Value>) -> Value {
    json!({"summary": done.summary, "data": done.data, "next_cursor": done.next_cursor, "total": done.total})
}
