//! The reflow a changing call's checks report (lane P5m, docx-agent's `_base.checks`):
//! measured after the call's edits from the version its first edit was applied to -- where
//! the engine laid it out last or the document's last check saw it -- to the version the
//! call made; "stale: call check" past 20 pages; a stop laying out said as docx-agent's
//! deadline says it. Another writer's commit before the edit is in "before". A call never waits
//! for the engine: busy with another's relayout, the versions are laid out on a spare engine.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use crate::check::{versions, Past};
use crate::common::{engine, Never, Still, Unlimited};
use ooxml_opc::Package;
use ooxml_store::{Caller, StoreConfig};
use ooxml_tools_word::{
    reflow_after_edit, CallEditor, EditedFrom, EnginePages, LastCheck, LayoutLimit, Made, WordLayout, STALE,
};
use ooxml_word_edit::{Editor, RevisionPolicy, SetText};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

fn editor(package: Package) -> Editor {
    Editor::open(package, StoreConfig::default(), Arc::new(Still), Arc::new(Unlimited)).unwrap()
}

/// One call's edit of `editor`: the fixture's first body paragraph's text set to `text`.
fn edit(editor: &Editor, text: &str) -> Made {
    let call = CallEditor::new(editor);
    let command = SetText {
        target: ooxml_word_model::address::Address::from("p:4C32E1FB".to_owned()),
        text: text.to_owned(),
        policy: RevisionPolicy::Untracked,
    };
    call.run(&Caller::anonymous(), &command).unwrap();
    call.made().expect("the edit changed the document")
}

/// The reflow `made` reports, laid out by `layout`.
fn reflow(layout: &EnginePages, last: &LastCheck, made: &Made, limit: Option<LayoutLimit<'_>>) -> Option<Value> {
    let before = &made.before;
    let from = EditedFrom {
        version: before.seq().0,
        package: before.doc().package(),
    };
    let after = made.last.doc();
    reflow_after_edit(
        layout,
        last,
        from,
        after.package(),
        after.ids().aliases(),
        (limit, &Never),
    )
}

#[test]
fn an_edit_after_the_version_was_laid_out_reports_its_reflow() {
    let (package, _) = versions();
    let editor = editor(package.clone());
    let layout = EnginePages::of(engine());
    let laid = layout.laid(&package, &Never).unwrap();
    let made = edit(&editor, &"longer text ".repeat(400));
    let found = reflow(&layout, &LastCheck::default(), &made, None).expect("measured");
    let pages = laid.page_count();
    assert_eq!(found["changed"][0], json!(1), "{found}");
    assert_eq!(found["pages"][0], json!(pages), "{found}");
    assert!(found["pages"][1].as_u64().unwrap() > pages as u64, "{found}");
}

#[test]
fn without_a_layout_of_the_version_before_no_reflow_is_measured() {
    let (package, _) = versions();
    let editor = editor(package);
    let layout = EnginePages::of(engine());
    let made = edit(&editor, "Shorter");
    assert_eq!(reflow(&layout, &LastCheck::default(), &made, None), None);
}

#[test]
fn another_writer_s_commit_before_the_edit_is_in_the_version_before() {
    // "before" is the version the call's first edit was applied to: here the other writer's,
    // which nothing laid out -- not the version the engine laid out before it committed.
    let (package, _) = versions();
    let editor = editor(package.clone());
    let layout = EnginePages::of(engine());
    layout.laid(&package, &Never).unwrap();
    let theirs = edit(&editor, "Another writer's text");
    let made = edit(&editor, &"longer text ".repeat(400));
    let before = &made.before;
    assert_eq!(before.seq(), theirs.last.seq(), "applied to the other writer's version");
    assert_eq!(reflow(&layout, &LastCheck::default(), &made, None), None);
    // laid out after their commit, the reflow is measured from their version
    let layout = EnginePages::of(engine());
    layout.laid(before.doc().package(), &Never).unwrap();
    let found = reflow(&layout, &LastCheck::default(), &made, None).expect("measured");
    assert_eq!(found["changed"][0], json!(1), "{found}");
}

#[test]
fn an_engine_busy_laying_another_version_out_is_not_waited_for() {
    // Held by another call, the versions are laid out on a spare engine of its kind: the
    // reflow is the one measured on the engine itself.
    let (package, _) = versions();
    let editor = editor(package.clone());
    let layout = EnginePages::of(engine());
    layout.laid(&package, &Never).unwrap();
    let made = edit(&editor, &"longer text ".repeat(400));
    let busy = layout.engine().lock().unwrap();
    let spared = reflow(&layout, &LastCheck::default(), &made, None).expect("measured on a spare");
    drop(busy);
    let found = reflow(&layout, &LastCheck::default(), &made, None).expect("measured");
    assert_eq!(spared, found);
    assert_eq!(found["changed"][0], json!(1), "{found}");
}

#[test]
fn a_busy_engine_that_did_not_lay_the_version_before_out_measures_none() {
    let (package, _) = versions();
    let editor = editor(package);
    let layout = EnginePages::of(engine());
    let made = edit(&editor, "Shorter");
    let _busy = layout.engine().lock().unwrap();
    assert_eq!(reflow(&layout, &LastCheck::default(), &made, None), None);
}

#[test]
fn a_busy_engine_says_stale_past_twenty_pages_and_a_stop_on_the_spare() {
    let package = long(24);
    let long_one = editor(package.clone());
    let layout = EnginePages::of(engine());
    layout.laid(&package, &Never).unwrap();
    let made = edit(&long_one, "Shorter");
    let busy = layout.engine().lock().unwrap();
    assert_eq!(reflow(&layout, &LastCheck::default(), &made, None), Some(json!(STALE)));
    drop(busy);
    let (package, _) = versions();
    let short_one = editor(package.clone());
    layout.laid(&package, &Never).unwrap();
    let made = edit(&short_one, &"longer text ".repeat(400));
    let limit = LayoutLimit {
        cancel: &Past,
        timeout: Duration::from_millis(1500),
    };
    let _busy = layout.engine().lock().unwrap();
    assert_eq!(
        reflow(&layout, &LastCheck::default(), &made, Some(limit)),
        Some(json!("timeout: laying the document out did not finish within 1.5 s"))
    );
}

/// The fixture with its default run size doubled: a style edit that moves every line.
fn restyled(package: &Package) -> Package {
    let xml = String::from_utf8(package.read("word/styles.xml").unwrap().unwrap().to_vec()).unwrap();
    let defaults = r#"<w:sz w:val="24"/><w:szCs w:val="24"/>"#;
    assert_eq!(xml.matches(defaults).count(), 1, "the fixture's default size");
    let larger = xml.replacen(defaults, r#"<w:sz w:val="48"/><w:szCs w:val="48"/>"#, 1);
    package.with_bytes("word/styles.xml", Arc::from(larger.into_bytes()))
}

#[test]
fn a_style_edit_reports_its_reflow_on_the_engine_and_on_a_spare_alike() {
    // The version before is painted in its own styles after the edit's are laid out: the
    // engine's own measure is the spare's, which lays both out fresh.
    let (package, _) = versions();
    let after = restyled(&package);
    let measure = |layout: &EnginePages| {
        let from = EditedFrom {
            version: 0,
            package: &package,
        };
        let aliases = Default::default();
        reflow_after_edit(layout, &LastCheck::default(), from, &after, &aliases, (None, &Never))
    };
    let layout = EnginePages::of(engine());
    let laid = layout.laid(&package, &Never).unwrap();
    let own = measure(&layout).expect("measured");
    assert_eq!(own["pages"][0], json!(laid.page_count()), "{own}");
    assert!(own["pages"][1].as_u64().unwrap() > laid.page_count() as u64, "{own}");
    assert_eq!(own["changed"][0], json!(1), "{own}");
    let spare = EnginePages::of(engine());
    spare.laid(&package, &Never).unwrap();
    let _busy = spare.engine().lock().unwrap();
    assert_eq!(measure(&spare), Some(own));
}

/// The fixture with `count` page breaks before its body.
fn long(count: usize) -> Package {
    let (package, _) = versions();
    let xml = String::from_utf8(package.read("word/document.xml").unwrap().unwrap().to_vec()).unwrap();
    let at = xml.find("<w:body>").unwrap() + "<w:body>".len();
    let breaks = r#"<w:p><w:r><w:br w:type="page"/></w:r></w:p>"#.repeat(count);
    let longer = format!("{}{breaks}{}", &xml[..at], &xml[at..]);
    package.with_bytes("word/document.xml", Arc::from(longer.into_bytes()))
}

#[test]
fn a_version_before_of_more_than_twenty_pages_is_stale() {
    let package = long(24);
    let editor = editor(package.clone());
    let layout = EnginePages::of(engine());
    assert!(layout.laid(&package, &Never).unwrap().page_count() > 20);
    let made = edit(&editor, "Shorter");
    assert_eq!(reflow(&layout, &LastCheck::default(), &made, None), Some(json!(STALE)));
}

#[test]
fn laying_the_version_made_out_past_the_limit_is_said() {
    let (package, _) = versions();
    let editor = editor(package.clone());
    let layout = EnginePages::of(engine());
    layout.laid(&package, &Never).unwrap();
    let made = edit(&editor, &"longer text ".repeat(400));
    let limit = LayoutLimit {
        cancel: &Past,
        timeout: Duration::from_millis(1500),
    };
    assert_eq!(
        reflow(&layout, &LastCheck::default(), &made, Some(limit)),
        Some(json!("timeout: laying the document out did not finish within 1.5 s"))
    );
}

/// The `q` quantile of `times` (sorted), milliseconds.
fn quantile(times: &[Duration], q: f64) -> f64 {
    let at = ((times.len() as f64 - 1.0) * q).round() as usize;
    times[at].as_secs_f64() * 1000.0
}

/// One writer's call: an edit, and (`measured`) its reflow as its checks report it (where
/// the engine laid out the version before: the call before laid its own version out); its
/// time, and whether a reflow was measured.
fn timed_call(editor: &Editor, layout: &EnginePages, text: &str, measured: bool) -> (Duration, bool) {
    let started = Instant::now();
    let made = edit(editor, text);
    let found = measured && reflow(layout, &LastCheck::default(), &made, None).is_some();
    (started.elapsed(), found)
}

#[test]
#[ignore = "a timing: cargo test --release -- --ignored edit_reflow"]
fn edits_with_their_reflow_timed_alone_and_with_twenty_four_writers() {
    let (package, _) = versions();
    for measured in [false, true] {
        let editor = editor(package.clone());
        let layout = EnginePages::of(engine());
        layout.laid(&package, &Never).unwrap();
        let calls: Vec<(Duration, bool)> = (0..20)
            .map(|i| {
                timed_call(
                    &editor,
                    &layout,
                    &format!("alone {i} {}", "text ".repeat(i * 20)),
                    measured,
                )
            })
            .collect();
        let found = calls.iter().filter(|c| c.1).count();
        let mut alone: Vec<Duration> = calls.into_iter().map(|c| c.0).collect();
        alone.sort();
        let times = Mutex::new(Vec::new());
        let started = Instant::now();
        std::thread::scope(|scope| {
            for writer in 0..24 {
                let (editor, layout, times) = (&editor, &layout, &times);
                scope.spawn(move || {
                    for i in 0..5 {
                        let text = format!("writer {writer} call {i} {}", "text ".repeat((writer + i) * 10));
                        let took = timed_call(editor, layout, &text, measured);
                        times.lock().unwrap().push(took);
                    }
                });
            }
        });
        let wall = started.elapsed();
        let mixed = times.into_inner().unwrap();
        let mixed_found = mixed.iter().filter(|c| c.1).count();
        let mut mixed: Vec<Duration> = mixed.into_iter().map(|c| c.0).collect();
        mixed.sort();
        println!(
            "reflow {measured}: alone p50 {:.1} ms p99 {:.1} ms ({found}/20 measured); 24 writers x 5 calls \
             p50 {:.1} ms p99 {:.1} ms ({mixed_found}/120 measured), wall {:.0} ms",
            quantile(&alone, 0.5),
            quantile(&alone, 0.99),
            quantile(&mixed, 0.5),
            quantile(&mixed, 0.99),
            wall.as_secs_f64() * 1000.0
        );
    }
}

#[test]
#[ignore = "a timing: cargo test --release -- --ignored edit_reflow"]
fn a_reflow_on_a_spare_engine_timed_against_the_engine_s_own() {
    // A version before of about 19 pages, one edit after it (texts alike: the edit's text diff is quadratic in unlike text): the measure on the engine itself
    // (it laid the version before out) and on a spare, the engine held by another call.
    let (package, _) = versions();
    let writer = editor(package);
    edit(&writer, &"longer text ".repeat(1900));
    let made = edit(&writer, &"longer text ".repeat(1890));
    let before = made.before.doc().package().clone();
    for (name, busy) in [("own engine", false), ("spare engine", true)] {
        let mut times = Vec::new();
        for _ in 0..5 {
            let layout = EnginePages::of(engine());
            layout.laid(&before, &Never).unwrap();
            let held = busy.then(|| layout.engine().lock().unwrap());
            let started = Instant::now();
            let found = reflow(&layout, &LastCheck::default(), &made, None).expect("measured");
            times.push(started.elapsed());
            drop(held);
            assert!(found.get("changed").is_some(), "{found}");
        }
        times.sort();
        println!(
            "{name}: reflow from a ~19-page version p50 {:.1} ms max {:.1} ms",
            quantile(&times, 0.5),
            quantile(&times, 1.0)
        );
    }
}
