//! The change set's differential (phase 3 identity design, section 5.3): editor-driven
//! random lineages -- text set, paragraphs put, blocks deleted and moved, cells written,
//! tracked and untracked, undone and redone -- over documents stamped and not. After each
//! step, the version's change set (the store's, worked out against the version before):
//! - equals the change set between the two versions read afresh (indexes built from scratch);
//! - names exactly the blocks a from-scratch comparison of every block's key finds gone,
//!   come and changed;
//! - gives `touched` and the ids delta that main's frozen reference gives (`delta_reference`).
// Tests may unwrap, panic and index (docs/architecture/README.md, section 2.5).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use crate::common::{editor, fixture, policy, Rng, WORDS};
use crate::delta_reference;
use ooxml_store::{Caller, Snapshot};
use ooxml_word_edit::touched::touched_of;
use ooxml_word_edit::{
    ids_delta_of, CellWrite, DeleteBlock, Editor, InsertParagraph, MoveBlocks, Moving, SetCells, SetText, WordDoc,
};
use ooxml_word_model::address::Address;
use ooxml_word_model::docx::Docx;
use ooxml_word_model::ids::{BlockKey, Entry, StoryDelta, StoryIx, StoryTable, VersionDelta};
use ooxml_xml::Element;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;

const FIXTURES: &[&str] = &[
    "samplelib/sample-simple.docx",
    "samplelib/sample-resume.docx",
    "generated/ids-and-markup.docx",
    "generated/controls.docx",
    "generated/tables-placed.docx",
    "generated/table-keeps.docx",
    "generated/lists-and-styles.docx",
    "generated/review-follow.docx",
    "wordto/sample-with-table.docx",
    "samplelib/sample-long.docx",
];

/// Every block of every story of `docx`, by key: its element.
fn every_block(docx: &Docx, stories: &mut StoryTable) -> HashMap<BlockKey, Arc<Element>> {
    let mut out = HashMap::new();
    for part in &docx.parts {
        let story = stories.intern(&part.story).unwrap();
        let ids = &part.ids;
        for p in ids.paragraphs().iter() {
            out.insert(ids.key_of(story, Entry::Paragraph(p.index)).unwrap(), p.element);
        }
        for (n, table) in ids.tables().iter().enumerate() {
            out.insert(ids.key_of(story, Entry::Table(n)).unwrap(), Arc::clone(&table.element));
            for (r, (path, _)) in table.rows.iter().enumerate() {
                let row = ooxml_word_model::docx::at_path(part.root(), path).unwrap();
                out.insert(ids.row_key_of(story, n, r).unwrap(), Arc::clone(row));
            }
        }
    }
    out
}

/// The keys a from-scratch comparison finds: gone, come, and held by both with other content.
fn from_scratch(old: &Docx, new: &Docx, stories: &mut StoryTable) -> [BTreeSet<String>; 3] {
    let (a, b) = (every_block(old, stories), every_block(new, stories));
    let name = |k: &BlockKey| format!("{k:?}");
    let gone = a.keys().filter(|k| !b.contains_key(k)).map(name).collect();
    let came = b.keys().filter(|k| !a.contains_key(k)).map(name).collect();
    let differ = b
        .iter()
        .filter(|(k, e)| a.get(k).is_some_and(|o| !Arc::ptr_eq(o, e) && **o != ***e))
        .map(|(k, _)| name(k))
        .collect();
    [gone, came, differ]
}

/// The same sets, as `delta` gives them (a renamed block is gone under its old key and come
/// under its new one).
fn of_delta(delta: &VersionDelta, stories: &[StoryIx]) -> [BTreeSet<String>; 3] {
    let name = |k: &BlockKey| format!("{k:?}");
    let mut out: [BTreeSet<String>; 3] = Default::default();
    for change in &delta.stories {
        let StoryDelta::Blocks(d) = &change.delta else {
            panic!("a whole story in a lineage: {}", change.part);
        };
        assert!(stories.contains(&change.story));
        out[0].extend(d.gone.iter().map(|(k, _)| name(k)));
        out[0].extend(d.renamed.iter().map(|(o, _)| name(o)));
        out[1].extend(d.came.iter().map(|(k, _)| name(k)));
        out[1].extend(d.renamed.iter().map(|(_, n)| name(n)));
        let renamed: std::collections::HashSet<BlockKey> = d.renamed.iter().map(|(_, n)| *n).collect();
        out[2].extend(
            d.changed
                .iter()
                .chain(&d.stamp_only)
                .filter(|k| !renamed.contains(k))
                .map(name),
        );
    }
    out
}

/// A random id of the head's body: a paragraph (any depth) or a table.
fn some_block(doc: &WordDoc, rng: &mut Rng, tables: bool) -> Option<String> {
    let docx = doc.docx().unwrap();
    let ids = &docx.body()?.ids;
    if tables {
        let n = ids.tables().len();
        return (n > 0).then(|| ids.tables().head(rng.below(n)).unwrap().id);
    }
    let n = ids.paragraphs().len();
    (n > 0).then(|| ids.paragraphs().get(rng.below(n)).unwrap().id)
}

/// One random step on `editor`; `false` when it changed nothing.
fn step(editor: &Editor, rng: &mut Rng, writer: u64) -> bool {
    let head = editor.store().snapshot();
    let doc = head.doc();
    let caller = Caller::anonymous();
    let text = || format!("{} {}", WORDS[(writer as usize) % WORDS.len()], writer);
    let policy = policy(rng, writer);
    let (pick, tables) = (rng.below(8), rng.below(3) == 0);
    let ran = match pick {
        0 | 1 => some_block(doc, rng, false).map(|id| {
            let command = SetText {
                target: Address::from(id),
                text: text(),
                policy,
            };
            editor.run(&caller, &command).map(|e| e.changed)
        }),
        2 => some_block(doc, rng, tables).map(|anchor| {
            let command = InsertParagraph {
                anchor,
                after: rng.below(2) == 0,
                text: text(),
                style: None,
                policy,
            };
            editor.run(&caller, &command).map(|e| e.changed)
        }),
        3 => some_block(doc, rng, tables).map(|id| {
            let command = DeleteBlock {
                target: Address::from(id),
                policy,
            };
            editor.run(&caller, &command).map(|e| e.changed)
        }),
        4 => {
            let moving = some_block(doc, rng, tables);
            let to = some_block(doc, rng, false);
            moving.zip(to).map(|(moving, to)| {
                let to = Some(Address::from(to));
                let command = MoveBlocks {
                    blocks: Moving::Blocks(Address::from(moving)),
                    after: to.clone().filter(|_| writer % 2 == 0),
                    before: to.filter(|_| writer % 2 == 1),
                    policy,
                };
                editor.run(&caller, &command).map(|e| e.changed)
            })
        }
        5 => some_block(doc, rng, true).map(|table| {
            let command = SetCells {
                table: Address::from(table),
                writes: vec![CellWrite::at(rng.below(2), rng.below(2), text())],
                stamping: rng.below(2) == 0,
                policy,
                accepted: None,
            };
            editor.run(&caller, &command).map(|e| e.changed)
        }),
        6 => return editor.store().undo().is_ok(),
        _ => return editor.store().redo().is_ok(),
    };
    matches!(ran, Some(Ok(true)))
}

/// What differs between the derived results and the reference's for one step, if anything.
fn compare_step(before: &Snapshot<WordDoc>, after: &Snapshot<WordDoc>, stories: &mut StoryTable) -> Vec<String> {
    let (old, new) = (before.doc(), after.doc());
    for part in old.docx().unwrap().parts.iter().chain(&new.docx().unwrap().parts) {
        stories.intern(&part.story).unwrap();
    }
    let delta = after.changes().expect("a version after another has changes");
    let mut differences = Vec::new();
    // the same function on indexes built from scratch
    let (fresh_old, fresh_new) = (Docx::open(old.package()).unwrap(), Docx::open(new.package()).unwrap());
    let fresh = VersionDelta::between(&fresh_old, &fresh_new, &mut stories.clone());
    if *delta != fresh {
        differences.push(format!("delta {delta:?} != from scratch {fresh:?}"));
    }
    // where every key follows from content, the keys name the same blocks in both
    let by_content = |docx: &Docx| docx.parts.iter().all(|p| p.ids.keys_by_content());
    if by_content(&fresh_old) && by_content(&fresh_new) {
        let interned: Vec<StoryIx> = delta.stories.iter().map(|s| s.story).collect();
        let scratch = from_scratch(&fresh_old, &fresh_new, stories);
        let derived = of_delta(&delta, &interned);
        if scratch != derived {
            differences.push(format!("keys {derived:?} != from scratch {scratch:?}"));
        }
    }
    let touched = touched_of(old, new, &delta).unwrap();
    let reference = delta_reference::touched::touched(old, new);
    if touched != reference {
        differences.push(format!(
            "touched {touched:?} != reference {reference:?}; delta {delta:?}"
        ));
    }
    let ids = ids_delta_of(old, new, &delta).unwrap();
    let reference = delta_reference::ids::ids_delta(old, new);
    if ids != reference {
        differences.push(format!("ids {ids:?} != reference {reference:?}"));
    }
    differences
}

#[test]
fn a_versions_change_set_is_the_index_diff_and_gives_todays_touched_and_ids() {
    let seeds: u64 = std::env::var("DELTA_SEEDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(6);
    let steps = 14;
    let mut found: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut compared = 0;
    for name in FIXTURES {
        let bytes = fixture(name);
        for seed in 1..=seeds {
            let editor = editor(&bytes);
            let mut stories = StoryTable::new();
            let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
            for writer in 0..steps {
                let before = editor.store().snapshot();
                if !step(&editor, &mut rng, writer) {
                    continue;
                }
                let after = editor.store().snapshot();
                compared += 1;
                for difference in compare_step(&before, &after, &mut stories) {
                    found
                        .entry(format!("{name} seed {seed} step {writer}"))
                        .or_default()
                        .push(difference);
                }
            }
        }
    }
    assert!(compared > 100, "{compared} steps compared");
    assert!(
        found.is_empty(),
        "{} of {compared} steps differ: {found:#?}",
        found.len()
    );
}
