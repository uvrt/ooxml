//! Validation kept in place against the full check: on every docx-agent fixture, the
//! problems the tally gives -- as opened, then after each of a few tracked and untracked
//! `set_text`s in every story -- equal those the memo-free check finds (design section 7:
//! each fact ships a from-scratch oracle it is tested against).
// Tests may unwrap, panic and index (docs/architecture/README.md, section 2.5).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use crate::common::{Monotonic, Unlimited};
use ooxml_base::Limits;
use ooxml_opc::Package;
use ooxml_store::{Caller, StoreConfig};
use ooxml_word_edit::validate::{full_problems, problems};
use ooxml_word_edit::{Editor, RevisionPolicy, SetText, Tracking};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

fn fixtures(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            fixtures(&path, out);
        } else if path.extension().is_some_and(|e| e == "docx") {
            out.push(path);
        }
    }
}

#[test]
fn kept_problems_equal_the_full_check_on_every_fixture() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../crates/ooxml-oracle/fixtures");
    let mut files = Vec::new();
    fixtures(&root, &mut files);
    files.sort();
    assert!(files.len() > 10);
    for file in files {
        let bytes: Arc<[u8]> = Arc::from(std::fs::read(&file).unwrap());
        let package = Package::open(bytes, &Limits::default()).unwrap();
        let Ok(editor) = Editor::open(
            package,
            StoreConfig::default(),
            Arc::new(Monotonic(Instant::now())),
            Arc::new(Unlimited),
        ) else {
            continue;
        };
        let opened = editor.store().snapshot();
        assert_eq!(
            problems(opened.doc()).unwrap(),
            full_problems(opened.doc()).unwrap(),
            "{}: as opened",
            file.display()
        );
        let docx = opened.doc().docx().unwrap();
        // a paragraph of each story, its last one, edited in turn
        let targets: Vec<String> = docx
            .parts
            .iter()
            .filter_map(|p| p.ids.paragraphs().last().map(|q| q.id))
            .collect();
        for (k, target) in targets.iter().enumerate() {
            let policy = if k % 2 == 0 {
                RevisionPolicy::Tracked(Tracking {
                    author: "Writer A".to_owned(),
                    date: "2026-11-17T10:00:00Z".to_owned(),
                })
            } else {
                RevisionPolicy::Untracked
            };
            let command = SetText {
                target: ooxml_word_model::address::Address::from(target.clone()),
                text: format!("Edited {k}: a sentence"),
                policy,
            };
            let Ok(done) = editor.run(&Caller::anonymous(), &command) else {
                continue;
            };
            let doc = done.version.doc();
            assert_eq!(
                problems(doc).unwrap(),
                full_problems(doc).unwrap(),
                "{}: after editing {target}",
                file.display()
            );
        }
    }
}
