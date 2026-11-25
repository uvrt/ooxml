//! `word-edits-bench <document.docx> [writers] [edits]`: what a `set_text` costs on a long
//! document -- opening (the reading and the baseline problems), then one writer's edits
//! (prepare and commit, `touched`, the validation delta, each timed), then `writers`
//! threads editing at once (each edit's whole time, the store's conflicts and exclusive
//! runs) -- as JSON.
//!
//! Must not: be used by any library; it is a test instrument.

use crate::edits::{Monotonic, Unlimited};
use ooxml_base::Limits;
use ooxml_opc::Package;
use ooxml_store::{Caller, StoreConfig};
use ooxml_word_edit::{Editor, RevisionPolicy, SetText, Tracking};
use serde_json::json;
use std::sync::Arc;
use std::time::Instant;

fn ms(d: std::time::Duration) -> f64 {
    (d.as_secs_f64() * 1e6).round() / 1e3
}

fn quantile(sorted: &[f64], q: f64) -> f64 {
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss
    )]
    let at = ((sorted.len().saturating_sub(1)) as f64 * q).round() as usize;
    sorted.get(at).copied().unwrap_or(0.0)
}

fn summary(mut values: Vec<f64>) -> serde_json::Value {
    values.sort_by(f64::total_cmp);
    json!({"n": values.len(), "p50": quantile(&values, 0.5), "p90": quantile(&values, 0.9),
           "p99": quantile(&values, 0.99), "max": values.last().copied().unwrap_or(0.0)})
}

fn command(k: usize, targets: &[String], writer: usize) -> SetText {
    let target = targets
        .get((k * 7919 + writer * 104_729) % targets.len().max(1))
        .cloned()
        .unwrap_or_default();
    let policy = if k.is_multiple_of(2) {
        RevisionPolicy::Untracked
    } else {
        RevisionPolicy::Tracked(Tracking {
            author: format!("Writer {writer}"),
            date: "2026-11-17T10:00:00Z".to_owned(),
        })
    };
    SetText {
        target: ooxml_word_model::address::Address::from(target),
        text: format!("Edit {k} by writer {writer}: the quarterly figures, restated."),
        policy,
    }
}

/// Whether a paragraph holds a field (a complex or a simple one).
fn has_field(element: &ooxml_xml::Element) -> bool {
    element
        .child_elements()
        .any(|c| matches!(c.name().local(), "fldChar" | "fldSimple" | "instrText") || has_field(c))
}

/// `writers` threads each making `edits` edits at once: each edit's whole time, the
/// store's conflicts, exclusive runs and rebases.
fn many(editor: &Editor, targets: &[String], writers: usize, edits: usize) -> serde_json::Value {
    let before = editor.store().stats();
    let latencies = std::sync::Mutex::new(Vec::new());
    let wall = Instant::now();
    std::thread::scope(|scope| {
        for w in 0..writers {
            let (editor, targets, latencies) = (&editor, &targets, &latencies);
            scope.spawn(move || {
                let caller = Caller::named(&format!("writer {w}"));
                for k in 0..edits {
                    let at = Instant::now();
                    if editor.run(&caller, &command(k + 100, targets, w)).is_ok() {
                        if let Ok(mut l) = latencies.lock() {
                            l.push(ms(at.elapsed()));
                        }
                    }
                }
            });
        }
    });
    let wall = wall.elapsed();
    let after = editor.store().stats();
    let latencies = latencies.into_inner().unwrap_or_default();
    json!({"threads": writers, "edits": writers * edits, "wall_ms": ms(wall),
           "latency": summary(latencies),
           "conflicts": after.conflicts - before.conflicts,
           "exclusive": after.exclusive - before.exclusive,
           "rebased": after.rebased - before.rebased,
           "held_max_ms": ms(after.held_max)})
}

/// Run the benchmark.
///
/// # Errors
/// A message when the document cannot be read or an edit fails.
pub fn run(args: &[String]) -> Result<(), String> {
    let file = args
        .first()
        .ok_or("usage: word-edits-bench <document.docx> [writers] [edits]")?;
    let writers: usize = args.get(1).and_then(|a| a.parse().ok()).unwrap_or(24);
    let edits: usize = args.get(2).and_then(|a| a.parse().ok()).unwrap_or(10);
    let bytes: Arc<[u8]> = Arc::from(std::fs::read(file).map_err(|e| e.to_string())?);
    let started = Instant::now();
    let package = Package::open(Arc::clone(&bytes), &Limits::default()).map_err(|e| e.to_string())?;
    let editor = Editor::open(
        package,
        StoreConfig::default(),
        Arc::new(Monotonic::new()),
        Arc::new(Unlimited),
    )
    .map_err(|e| e.to_string())?;
    let opened = started.elapsed();
    let docx = editor.store().snapshot().doc().docx().map_err(|e| e.to_string())?;
    // paragraphs of plain text: a field's result cannot be rewritten whole (refused)
    let targets: Vec<String> = docx
        .body()
        .map(|b| {
            b.ids
                .paragraphs()
                .iter()
                .filter(|p| p.path.len() == 2 && !has_field(&p.element))
                .map(|p| p.id.clone())
                .collect()
        })
        .unwrap_or_default();
    // the first edit stamps whatever lacks an id
    let first = Instant::now();
    editor
        .run(&Caller::anonymous(), &command(0, &targets, 99))
        .map_err(|e| e.to_string())?;
    let first = first.elapsed();
    let (mut commit, mut touched, mut validation, mut whole) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for k in 1..=20 {
        let at = Instant::now();
        let done = editor
            .run(&Caller::anonymous(), &command(k, &targets, 0))
            .map_err(|e| e.to_string())?;
        whole.push(ms(at.elapsed()));
        commit.push(ms(done.timing.commit.total));
        touched.push(ms(done.timing.touched));
        validation.push(ms(done.timing.validation));
    }
    let writers_json = many(&editor, &targets, writers, edits);
    let out = json!({
        "file": file,
        "paragraphs": targets.len(),
        "open_ms": ms(opened),
        "first_edit_ms": ms(first),
        "one_writer": {"whole": summary(whole), "prepare_commit": summary(commit),
                       "touched": summary(touched), "validation": summary(validation)},
        "writers": writers_json,
    });
    println!("{out}");
    Ok(())
}
