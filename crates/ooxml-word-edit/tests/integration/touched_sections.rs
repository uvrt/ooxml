//! `touched`'s heading sections for a change of many scattered blocks (lane L4a: a field
//! update rewrites thousands): the sections are those each changed block's change alone
//! touches, together -- however many blocks change between two headings or before the
//! first.
// Tests may unwrap, panic and index (docs/architecture/README.md, section 2.5).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use crate::common::{editor, fixture};
use ooxml_store::Caller;
use ooxml_word_edit::touched::{touched, Section};
use ooxml_word_edit::{RevisionPolicy, SetText};
use std::collections::BTreeSet;

fn sections(found: Vec<ooxml_word_edit::touched::Touched>) -> BTreeSet<(Option<String>, i64, [String; 2])> {
    found
        .into_iter()
        .filter_map(|t| t.section)
        .map(|Section { heading, level, blocks }| (heading, level, blocks))
        .collect()
}

#[test]
fn many_changed_blocks_touch_the_sections_each_touches_alone() {
    let editor = editor(&fixture("samplelib/sample-long.docx"));
    let first = editor.store().snapshot().doc().clone();
    let docx = first.docx().unwrap();
    let ids: Vec<String> = docx
        .body()
        .unwrap()
        .ids
        .paragraphs()
        .iter()
        .filter(|p| p.path.len() == 2)
        .map(|p| p.id.clone())
        .collect();
    assert!(ids.len() > 40, "{} paragraphs", ids.len());
    let mut alone = BTreeSet::new();
    // Runs of neighbours (several blocks under one heading) and scattered ones.
    let chosen: Vec<&String> = ids
        .iter()
        .enumerate()
        .filter(|(k, _)| k % 7 < 3)
        .map(|(_, id)| id)
        .collect();
    for id in &chosen {
        let before = editor.store().snapshot().doc().clone();
        let command = SetText {
            target: ooxml_word_model::address::Address::from((*id).clone()),
            text: format!("Changed {id}"),
            policy: RevisionPolicy::Untracked,
        };
        editor.run(&Caller::anonymous(), &command).unwrap();
        let after = editor.store().snapshot().doc().clone();
        alone.extend(sections(touched(&before, &after).unwrap()));
    }
    let last = editor.store().snapshot().doc().clone();
    let together = sections(touched(&first, &last).unwrap());
    assert!(together.len() > 1, "{together:?}");
    assert_eq!(together, alone);
}
