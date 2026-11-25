//! The style edits timed on a long document (lane P4d's hand-back): run with `STYLE_BENCH=<path
//! to a .docx> cargo test --release -p ooxml-word-edit --test integration -- --ignored
//! --nocapture style_bench::`; each edit's best of three runs, each on the document as opened.
// Tests may unwrap, panic and index (docs/architecture/README.md, section 2.5).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use crate::common::{best, editor};
use ooxml_store::Caller;
use ooxml_word_edit::{
    FormatEdit, FormatPlan, LevelFormat, ListAction, ListEdit, Property, RevisionPolicy, Setting, StyleAction,
    StyleEdit, StyleFormatting, Tracking, Value,
};
use std::sync::Arc;
use std::time::{Duration, Instant};

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
        targets: targets
            .iter()
            .map(|t| ooxml_word_model::address::Address::parse(t))
            .collect(),
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
    style_sheet_edits(&bytes);
}

/// `word_format`'s edits: one paragraph, tracked, a span; a style; keeps.
fn format_edits(bytes: &Arc<[u8]>, targets: &[String; 2], tracked: &RevisionPolicy) {
    let [middle, span] = targets;
    let report = |name: &str, took: Duration| println!("{name}: {:.1} ms", took.as_secs_f64() * 1000.0);
    let bold = |targets: Vec<String>, policy: &RevisionPolicy| FormatEdit {
        targets: targets
            .iter()
            .map(|t| ooxml_word_model::address::Address::parse(t))
            .collect(),
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
    let heading = |targets: Vec<String>| FormatEdit {
        targets: targets
            .iter()
            .map(|t| ooxml_word_model::address::Address::parse(t))
            .collect(),
        plan: FormatPlan {
            paragraph_style: Some("Heading 2".into()),
            ..FormatPlan::default()
        },
        cleared: false,
        policy: RevisionPolicy::Untracked,
    };
    report(
        "format, one paragraph to a heading style",
        best(bytes, &heading(vec![middle.clone()])),
    );
    let styled = FormatEdit {
        targets: (vec![span.clone()])
            .iter()
            .map(|t| ooxml_word_model::address::Address::parse(t))
            .collect(),
        plan: FormatPlan {
            paragraph_style: Some("Heading 2".into()),
            ..FormatPlan::default()
        },
        cleared: false,
        policy: RevisionPolicy::Untracked,
    };
    report("format, 100 paragraphs to a heading style", best(bytes, &styled));
    let kept = FormatEdit {
        targets: (vec![span.clone()])
            .iter()
            .map(|t| ooxml_word_model::address::Address::parse(t))
            .collect(),
        plan: FormatPlan {
            keep_together: Some(true),
            ..FormatPlan::default()
        },
        cleared: false,
        policy: RevisionPolicy::Untracked,
    };
    report("format, 100 paragraphs kept together", best(bytes, &kept));
}

/// `word_styles`' edits: a style's formatting, a style added, the most used custom
/// paragraph style removed (every use pointed at Normal), the unused purged, the default
/// table style, the document's language.
fn style_sheet_edits(bytes: &Arc<[u8]>) {
    let report = |name: &str, took: Duration| println!("{name}: {:.1} ms", took.as_secs_f64() * 1000.0);
    let docx = editor(bytes).store().snapshot().doc().docx().unwrap();
    let uses = ooxml_word_edit::content_uses(&docx);
    let most = docx
        .styles()
        .iter()
        .filter(|s| ooxml_word_model::docx::style_kind(s) == "paragraph")
        .filter(|s| s.attribute(Some(ooxml_word_model::ids::W), "default").is_none())
        .map(|s| ooxml_word_model::docx::style_id(s).to_owned())
        .max_by_key(|id| uses.get(id).copied().unwrap_or(0))
        .unwrap();
    println!("styles: {} uses of {most}", uses.get(&most).copied().unwrap_or(0));
    let style = |action: StyleAction| StyleEdit { action };
    let bold = StyleFormatting {
        run: vec![Setting {
            property: Property::Bold,
            value: Some(Value::Flag(true)),
        }],
        ..StyleFormatting::default()
    };
    let cases = [
        (
            "styles modify Normal",
            StyleAction::Modify {
                name: "Normal".into(),
                kind: None,
                numbering: None,
                tabs: None,
                regions: Vec::new(),
                formatting: bold.clone(),
            },
        ),
        (
            "styles add",
            StyleAction::Add {
                name: "Bench Body".into(),
                kind: "paragraph".into(),
                based_on: Some("Normal".into()),
                next_style: None,
                tabs: None,
                regions: Vec::new(),
                formatting: bold,
            },
        ),
        (
            "styles remove, uses repointed",
            StyleAction::Remove {
                name: most,
                kind: None,
                replacement: Some("Normal".into()),
            },
        ),
        ("styles purge_unused", StyleAction::PurgeUnused),
        (
            "styles set_default",
            StyleAction::SetDefault {
                name: "Table Grid".into(),
            },
        ),
        (
            "styles set_language",
            StyleAction::SetLanguage {
                language: "fr-BE".into(),
                style: None,
            },
        ),
    ];
    for (name, action) in cases {
        report(name, best(bytes, &style(action)));
    }
}
