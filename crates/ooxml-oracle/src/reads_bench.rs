//! `word-reads-bench <file.docx> [rounds]`: the Word reads' timings on one document, as
//! a session sees them (design section 7's targets): the first `describe` of a version
//! (it reads the version), `describe` again, `word_read` of the first page of the body,
//! `word_read` of a span of 20 blocks in the middle, `find_text`; then a one-block edit
//! made two ways -- a path copy (the paragraph replaced, every other block shared) and the
//! bridge's (the main part written and parsed again, nothing shared) -- and `describe` and
//! the span's `word_read` on each new version. Best of `rounds` (default 3), wall in ms, as
//! one JSON object (the process's CPU from `/usr/bin/time`).
//!
//! Must not: be used by any library; it is a measuring instrument.

use ooxml_base::{Cancel, Limits};
use ooxml_opc::Package;
use ooxml_tools::{ArgType, ToolError};
use ooxml_tools_word::{describe, find_text, word_read, Describe, FindText, Version, WordRead};
use ooxml_word_model::docx::{Docx, Versions};
use ooxml_word_model::ids::W;
use ooxml_xml::{Children, Node, Tree};
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
/// its first): by path copy, and by the bridge's write and parse.
fn edited(package: &Package, main: &str) -> Result<(Package, Package), String> {
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
    let bridged: Tree = ooxml_xml::parse(&copied.write(), &Limits::default().xml).map_err(|e| e.to_string())?;
    Ok((
        package.with_new_tree(main, copied),
        package.with_new_tree(main, bridged),
    ))
}

/// The ids of 20 top-level blocks from the middle of the body.
fn middle_span(docx: &Docx) -> Option<String> {
    let body = docx.body()?;
    let top: Vec<&str> = body
        .ids
        .paragraphs
        .iter()
        .filter(|p| p.path.len() == 2)
        .map(|p| p.id.as_str())
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
    let bytes = std::fs::read(file).map_err(|e| e.to_string())?;
    let package = Package::open(Arc::from(bytes), &Limits::default()).map_err(|e| e.to_string())?;
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
    let v1 = Version {
        name: "a.docx",
        package: &package,
        number: 1,
        versions: &versions,
        placer: None,
    };
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
    let (copied, bridged) = edited(&package, &main)?;
    for (name, edited) in [("path_copy", &copied), ("bridge", &bridged)] {
        let reads = Reads {
            d: &d,
            span: &span,
            other: &other,
        };
        after_edit(&mut out, name, &package, edited, &reads, rounds)?;
    }
    println!("{}", Value::Object(out));
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
    package: &Package,
    edited: &Package,
    reads: &Reads<'_>,
    rounds: usize,
) -> Result<(), String> {
    let (d, span, other) = (reads.d, reads.span, reads.other);
    let mut best = (f64::MAX, f64::MAX, f64::MAX);
    for _ in 0..rounds {
        let versions = Versions::new();
        let v1 = Version {
            name: "a.docx",
            package,
            number: 1,
            versions: &versions,
            placer: None,
        };
        describe(d, &v1, &Never).map_err(|e| e.message().to_owned())?;
        let v2 = Version {
            name: "a.docx",
            package: edited,
            number: 2,
            versions: &versions,
            placer: None,
        };
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
