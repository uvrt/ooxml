//! `word-reads-bench <file.docx> [rounds]`: the Word reads' timings on one document, as
//! a session sees them (design section 7's targets): the first `describe` of a version
//! (it reads the version), `describe` again, `word_read` of the first page of the body,
//! `word_read` of a span of 20 blocks in the middle, `find_text`; then a one-block edit
//! made as a path copy (the paragraph replaced, every other block shared) and `describe`
//! and the span's `word_read` on the new version; and the path docx-agent takes: through a
//! session, the span's first paragraph edited by `word_insert_text`, then `describe` and the
//! span reads on the version it made. Best of `rounds` (default 3), wall in ms, as one JSON
//! object (the process's CPU from `/usr/bin/time`).
//!
//! Must not: be used by any library; it is a measuring instrument.

use ooxml_base::{Cancel, Limits};
use ooxml_opc::Package;
use ooxml_tools::{ArgType, ToolError};
use ooxml_tools_word::{describe, find_text, word_read, Describe, FindText, Version, WordRead};
use ooxml_word_model::docx::{Docx, Versions};
use ooxml_word_model::ids::W;
use ooxml_xml::{Children, Node};
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

/// The package with its main part's middle body paragraph given one more run (a copy of
/// its first), by path copy.
fn edited(package: &Package, main: &str) -> Result<Package, String> {
    let tree = package.tree(main).map_err(|e| e.to_string())?.ok_or("no main part")?;
    let root = tree.root();
    let (b, body) = root
        .children()
        .iter()
        .enumerate()
        .find_map(|(k, n)| {
            n.as_element()
                .filter(|e| e.name().is(Some(W), "body"))
                .map(|e| (k, e.clone()))
        })
        .ok_or("no body")?;
    let kids: Vec<Node> = body.children().iter().cloned().collect();
    let mid = (kids.len() / 2..kids.len())
        .find(|k| {
            kids.get(*k)
                .and_then(Node::as_element)
                .is_some_and(|e| e.name().is(Some(W), "p") && e.child_elements().any(|r| r.name().is(Some(W), "r")))
        })
        .ok_or("no paragraph with a run")?;
    let p = kids.get(mid).and_then(Node::as_element).ok_or("no paragraph")?;
    let run = p
        .child_elements()
        .find(|r| r.name().is(Some(W), "r"))
        .cloned()
        .ok_or("no run")?;
    let mut runs: Vec<Node> = p.children().iter().cloned().collect();
    runs.push(Node::Element(run));
    let new_p = Arc::new(p.with_children(Children::from_vec(runs)));
    let mut new_kids = kids;
    if let Some(slot) = new_kids.get_mut(mid) {
        *slot = Node::Element(new_p);
    }
    let new_body = Arc::new(body.with_children(Children::from_vec(new_kids)));
    let mut top: Vec<Node> = root.children().iter().cloned().collect();
    if let Some(slot) = top.get_mut(b) {
        *slot = Node::Element(new_body);
    }
    let copied = tree.with_root(Arc::new(root.with_children(Children::from_vec(top))));
    Ok(package.with_new_tree(main, copied))
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
    let package = ooxml_word_model::WordPackage::open(package).0;
    let main = ooxml_word_model::parts::main_part(&package)
        .map_err(|e| e.to_string())?
        .ok_or("no main part")?;
    let mut out = Map::new();
    let d: Describe = arg(json!({"doc": "d1"}))?;
    let page: WordRead = arg(json!({"doc": "d1"}))?;
    let find: FindText = arg(json!({"doc": "d1", "text": "the"}))?;
    let start = Instant::now();
    let opened = Docx::open(&package).map_err(|e| e.to_string())?;
    out.insert("open_ms".into(), json!(start.elapsed().as_secs_f64() * 1000.0));
    let span = middle_span(&opened).ok_or("no span")?;
    out.insert("span".into(), json!(span));
    let other = span.split("..").next().unwrap_or("").to_owned();
    let span: WordRead = arg(json!({"doc": "d1", "range": span}))?;
    let other: WordRead = arg(json!({"doc": "d1", "range": other}))?;
    // Version 1: read cold once, then warm.
    let versions = Versions::new();
    let v1 = Version::new("a.docx", &package, 1, &versions);
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
    let copied = ooxml_word_model::WordPackage::edited(edited(&package, &main)?);
    let reads = Reads {
        d: &d,
        span: &span,
        other: &other,
    };
    after_edit(&mut out, "path_copy", &package, &copied, &reads, rounds)?;
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

/// The calls timed on a new version.
struct Reads<'a> {
    d: &'a Describe,
    span: &'a WordRead,
    other: &'a WordRead,
}

/// `describe` and two span reads on the version `edited` after version 1 was read, best of
/// `rounds`, under `name`.
fn after_edit(
    out: &mut Map<String, Value>,
    name: &str,
    package: &ooxml_word_model::WordPackage,
    edited: &ooxml_word_model::WordPackage,
    reads: &Reads<'_>,
    rounds: usize,
) -> Result<(), String> {
    let (d, span, other) = (reads.d, reads.span, reads.other);
    let mut best = (f64::MAX, f64::MAX, f64::MAX);
    for _ in 0..rounds {
        let versions = Versions::new();
        let v1 = Version::new("a.docx", package, 1, &versions);
        describe(d, &v1, &Never).map_err(|e| e.message().to_owned())?;
        let v2 = Version::new("a.docx", edited, 2, &versions);
        let start = Instant::now();
        describe(d, &v2, &Never).map_err(|e| e.message().to_owned())?;
        let first = start.elapsed().as_secs_f64() * 1000.0;
        let start = Instant::now();
        word_read(span, &v2, &Never).map_err(|e| e.message().to_owned())?;
        let read = start.elapsed().as_secs_f64() * 1000.0;
        // Another span of the same version: its lookups read already.
        let start = Instant::now();
        word_read(other, &v2, &Never).map_err(|e| e.message().to_owned())?;
        let again = start.elapsed().as_secs_f64() * 1000.0;
        best = (best.0.min(first), best.1.min(read), best.2.min(again));
    }
    out.insert(format!("{name}_describe_ms"), json!(best.0));
    out.insert(format!("{name}_word_read_span_first_ms"), json!(best.1));
    out.insert(format!("{name}_word_read_other_span_ms"), json!(best.2));
    Ok(())
}
