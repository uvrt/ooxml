//! The style edits timed on a long document (lane P4d's hand-back): run with
//! `STYLE_BENCH=<path to a .docx> cargo test --release --test style_bench -- --ignored
//! --nocapture`; each edit's best of three runs, each on the document as opened.
// Tests may unwrap, panic and index (docs/architecture/README.md, section 2.5).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use ooxml_base::Limits;
use ooxml_derive::CacheBudget;
use ooxml_opc::Package;
use ooxml_store::{Caller, Clock, Command, StoreConfig};
use ooxml_word_edit::{
    EditError, Editor, FormatEdit, FormatPlan, LevelFormat, ListAction, ListEdit, Property, RevisionPolicy, Setting,
    Tracking, Value, WordDoc,
};
use std::sync::Arc;
use std::time::{Duration, Instant};

struct Monotonic(Instant);

impl Clock for Monotonic {
    fn monotonic(&self) -> Duration {
        self.0.elapsed()
    }
}

struct Unlimited;

impl CacheBudget for Unlimited {
    fn reserve(&self, _bytes: usize) -> bool {
        true
    }
    fn release(&self, _bytes: usize) {}
    fn used(&self) -> usize {
        0
    }
}

fn editor(bytes: &Arc<[u8]>) -> Editor {
    let package = Package::open(Arc::clone(bytes), &Limits::default()).unwrap();
    Editor::open(
        package,
        StoreConfig::default(),
        Arc::new(Monotonic(Instant::now())),
        Arc::new(Unlimited),
    )
    .unwrap()
}

/// The best of three runs of `command`, each on a fresh editor (opening not counted).
fn best<C: Command<WordDoc, Error = EditError>>(bytes: &Arc<[u8]>, command: &C) -> Duration {
    (0..3)
        .map(|_| {
            let editor = editor(bytes);
            let started = Instant::now();
            editor.run(&Caller::anonymous(), command).unwrap();
            started.elapsed()
        })
        .min()
        .unwrap()
}

#[test]
#[ignore = "a timing on a long document: STYLE_BENCH=<docx> cargo test --release -- --ignored"]
fn style_edits_on_a_long_document() {
    let Ok(path) = std::env::var("STYLE_BENCH") else {
        return;
    };
    let bytes: Arc<[u8]> = Arc::from(std::fs::read(path).unwrap());
    let opened = editor(&bytes);
    let docx = opened.store().snapshot().doc().docx().unwrap();
    let ids: Vec<String> = docx
        .body()
        .unwrap()
        .ids
        .paragraphs()
        .iter()
        .filter(|p| p.path.len() == 2)
        .map(|p| p.id.clone())
        .collect();
    println!("{} body paragraphs", ids.len());
    let report = |name: &str, took: Duration| println!("{name}: {:.1} ms", took.as_secs_f64() * 1000.0);
    let tracked = RevisionPolicy::Tracked(Tracking {
        author: "Bench".into(),
        date: "2026-11-18T10:00:00Z".into(),
    });
    let list = |targets: Vec<String>, action: ListAction, policy: &RevisionPolicy| ListEdit {
        targets,
        action,
        policy: policy.clone(),
    };
    let add = || ListAction::Add {
        kind: "number".into(),
        level: 0,
    };
    let (early, middle) = (ids[10].clone(), ids[ids.len() / 2].clone());
    report(
        "lists add, one paragraph",
        best(&bytes, &list(vec![middle.clone()], add(), &RevisionPolicy::Untracked)),
    );
    report(
        "lists add, tracked",
        best(&bytes, &list(vec![middle.clone()], add(), &tracked)),
    );
    let span = format!("{early}..{}", ids[110]);
    report(
        "lists add, 100 paragraphs",
        best(&bytes, &list(vec![span.clone()], add(), &RevisionPolicy::Untracked)),
    );
    let format = ListAction::Format(LevelFormat {
        format: Some("lowerRoman".into()),
        ..LevelFormat::default()
    });
    let on_list = |second: ListAction| -> Duration {
        (0..3)
            .map(|_| {
                let editor = editor(&bytes);
                editor
                    .run(
                        &Caller::anonymous(),
                        &list(vec![span.clone()], add(), &RevisionPolicy::Untracked),
                    )
                    .unwrap();
                let started = Instant::now();
                editor
                    .run(
                        &Caller::anonymous(),
                        &list(vec![ids[60].clone()], second.clone(), &RevisionPolicy::Untracked),
                    )
                    .unwrap();
                started.elapsed()
            })
            .min()
            .unwrap()
    };
    report("lists restart, 50 items after", on_list(ListAction::Restart(3)));
    report("lists format", on_list(format));
    format_edits(&bytes, &[middle, span], &tracked);
}

/// `word_format`'s edits: one paragraph, tracked, a span; a style; keeps.
fn format_edits(bytes: &Arc<[u8]>, targets: &[String; 2], tracked: &RevisionPolicy) {
    let [middle, span] = targets;
    let report = |name: &str, took: Duration| println!("{name}: {:.1} ms", took.as_secs_f64() * 1000.0);
    let bold = |targets: Vec<String>, policy: &RevisionPolicy| FormatEdit {
        targets,
        plan: FormatPlan {
            run: vec![Setting {
                property: Property::Bold,
                value: Some(Value::Flag(true)),
            }],
            paragraph: vec![Setting {
                property: Property::SpaceAfter,
                value: Some(Value::Number(6.0)),
            }],
            ..FormatPlan::default()
        },
        cleared: false,
        policy: policy.clone(),
    };
    report(
        "format, one paragraph",
        best(bytes, &bold(vec![middle.clone()], &RevisionPolicy::Untracked)),
    );
    report("format, tracked", best(bytes, &bold(vec![middle.clone()], tracked)));
    report(
        "format, 100 paragraphs",
        best(bytes, &bold(vec![span.clone()], &RevisionPolicy::Untracked)),
    );
    let styled = FormatEdit {
        targets: vec![span.clone()],
        plan: FormatPlan {
            paragraph_style: Some("Heading 2".into()),
            ..FormatPlan::default()
        },
        cleared: false,
        policy: RevisionPolicy::Untracked,
    };
    report("format, 100 paragraphs to a heading style", best(bytes, &styled));
    let kept = FormatEdit {
        targets: vec![span.clone()],
        plan: FormatPlan {
            keep_together: Some(true),
            ..FormatPlan::default()
        },
        cleared: false,
        policy: RevisionPolicy::Untracked,
    };
    report("format, 100 paragraphs kept together", best(bytes, &kept));
}
