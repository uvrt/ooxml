//! `word-reads-bench <file.docx> [rounds]`: the Word reads' timings on one document, as
//! a session sees them (design section 7's targets): the first `describe` of a version
//! (it reads the version), `describe` again, `word_read` of the first page of the body,
//! `word_read` of a span of 20 blocks in the middle, `find_text` (the document opened as its
//! editor opens it, read through its one chain of readings); then the path docx-agent takes:
//! through a session, the span's first paragraph edited by `word_insert_text`, then `describe` and the
//! span reads on the version it made. Best of `rounds` (default 3), wall in ms, as one JSON
//! object (the process's CPU from `/usr/bin/time`).
//!
//! Must not: be used by any library; it is a measuring instrument.

use ooxml_base::{Cancel, Limits};
use ooxml_opc::Package;
use ooxml_tools::{ArgType, ToolError};
use ooxml_tools_word::{describe, find_text, word_read, Describe, FindText, Version, WordRead};
use ooxml_word_edit::WordDoc;
use ooxml_word_model::docx::Docx;
use serde_json::{json, Map, Value};
use std::sync::Arc;
use std::time::Instant;

struct Never;

impl Cancel for Never {
    fn is_cancelled(&self) -> bool {
        false
    }
}

fn timed(out: &mut Map<String, Value>, key: &str, rounds: usize, mut f: impl FnMut() -> Result<(), ToolError>) {
    let mut best = f64::MAX;
    for _ in 0..rounds {
        let start = Instant::now();
        if let Err(e) = f() {
            out.insert(key.into(), json!({"error": e.to_json()}));
            return;
        }
        best = best.min(start.elapsed().as_secs_f64() * 1000.0);
    }
    out.insert(key.into(), json!((best * 10.0).round() / 10.0));
}

fn arg<T: ArgType>(value: Value) -> Result<T, String> {
    T::from_arg(&value, "").map_err(|e| e.to_string())
}

/// The ids of 20 top-level blocks from the middle of the body.
fn middle_span(docx: &Docx) -> Option<String> {
    let body = docx.body()?;
    let top: Vec<String> = body
        .ids
        .paragraphs()
        .iter()
        .filter(|p| p.path.len() == 2)
        .map(|p| p.id)
        .collect();
    let a = top.get(top.len() / 2)?;
    let b = top.get((top.len() / 2 + 19).min(top.len().saturating_sub(1)))?;
    Some(format!("{a}..{b}"))
}

/// Run the benchmark.
///
/// # Errors
/// A message when the file cannot be read or edited.
pub fn run(args: &[String]) -> Result<(), String> {
    let file = args.first().ok_or("usage: word-reads-bench <file.docx> [rounds]")?;
    let rounds: usize = args.get(1).and_then(|r| r.parse().ok()).unwrap_or(3);
    let bytes: Arc<[u8]> = Arc::from(std::fs::read(file).map_err(|e| e.to_string())?);
    let package = Package::open(Arc::clone(&bytes), &Limits::default()).map_err(|e| e.to_string())?;
    let mut out = Map::new();
    let d: Describe = arg(json!({"doc": "d1"}))?;
    let page: WordRead = arg(json!({"doc": "d1"}))?;
    let find: FindText = arg(json!({"doc": "d1", "text": "the"}))?;
    let start = Instant::now();
    let doc = WordDoc::open(package, Arc::new(crate::export::Unlimited)).map_err(|e| e.to_string())?;
    let opened = doc.docx().map_err(|e| e.to_string())?;
    out.insert("open_ms".into(), json!(start.elapsed().as_secs_f64() * 1000.0));
    let span = middle_span(&opened).ok_or("no span")?;
    out.insert("span".into(), json!(span));
    let other = span.split("..").next().unwrap_or("").to_owned();
    let span: WordRead = arg(json!({"doc": "d1", "range": span}))?;
    let other: WordRead = arg(json!({"doc": "d1", "range": other}))?;
    // Version 1: read once at open, then warm.
    let v1 = Version::new("a.docx", doc.package(), 1).with_word(Some(&doc));
    let start = Instant::now();
    describe(&d, &v1, &Never).map_err(|e| e.message().to_owned())?;
    out.insert("describe_cold_ms".into(), json!(start.elapsed().as_secs_f64() * 1000.0));
    timed(&mut out, "describe_ms", rounds, || describe(&d, &v1, &Never).map(drop));
    let start = Instant::now();
    word_read(&page, &v1, &Never).map_err(|e| e.message().to_owned())?;
    out.insert(
        "word_read_page_first_ms".into(),
        json!(start.elapsed().as_secs_f64() * 1000.0),
    );
    let start = Instant::now();
    word_read(&span, &v1, &Never).map_err(|e| e.message().to_owned())?;
    out.insert(
        "word_read_span_first_ms".into(),
        json!(start.elapsed().as_secs_f64() * 1000.0),
    );
    timed(&mut out, "word_read_page_ms", rounds, || {
        word_read(&page, &v1, &Never).map(drop)
    });
    timed(&mut out, "word_read_span_ms", rounds, || {
        word_read(&span, &v1, &Never).map(drop)
    });
    timed(&mut out, "find_text_ms", rounds, || {
        find_text(&find, &v1, &Never).map(drop)
    });
    through_session(&mut out, &bytes, &reads_of(&span, &other), rounds)?;
    println!("{}", Value::Object(out));
    Ok(())
}

fn reads_of(span: &WordRead, other: &WordRead) -> (Value, Value) {
    let range = |r: &WordRead| json!({"doc": "d1", "range": r.range});
    (range(span), range(other))
}

/// The session path docx-agent takes: the document opened from its bytes and read, then the
/// span's first paragraph edited by `word_insert_text` and the version it made read. Best of
/// `rounds`.
fn through_session(
    out: &mut Map<String, Value>,
    bytes: &[u8],
    spans: &(Value, Value),
    rounds: usize,
) -> Result<(), String> {
    use ooxml_session::{CallContext, DocKind, Session, SessionConfig};
    let first = spans.1.get("range").cloned().unwrap_or(Value::Null);
    let insert = json!({"doc": "d1", "target": first, "text": " more"});
    let mut best = [f64::MAX; 4];
    for _ in 0..rounds {
        let session = Session::new(
            SessionConfig::default(),
            crate::reads::session_context(),
            ooxml_session_word::formats(None),
        );
        session
            .open(Arc::from(bytes), DocKind::Docx, "a.docx")
            .map_err(|e| e.to_string())?;
        let context = CallContext::default();
        let call = |name: &str, arguments: Value| -> Result<f64, String> {
            let start = Instant::now();
            let result = session.call(name, arguments, &context);
            if result.is_error() {
                return Err(format!("{name}: {:?}", result.to_json().get("error")));
            }
            Ok(start.elapsed().as_secs_f64() * 1000.0)
        };
        call("describe", json!({"doc": "d1"}))?;
        call("word_read", spans.0.clone())?;
        let edit = call("word_insert_text", insert.clone())?;
        let span = call("word_read", spans.0.clone())?;
        let describe = call("describe", json!({"doc": "d1"}))?;
        let other = call("word_read", spans.1.clone())?;
        for (slot, value) in best.iter_mut().zip([edit, span, describe, other]) {
            *slot = slot.min(value);
        }
    }
    for (key, value) in ["edit", "word_read_span_first", "describe", "word_read_other_span"]
        .iter()
        .zip(best)
    {
        out.insert(format!("session_{key}_ms"), json!((value * 10.0).round() / 10.0));
    }
    Ok(())
}
