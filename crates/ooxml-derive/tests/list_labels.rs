//! The reference test of ooxml-derive (design section 15): list labels kept incrementally
//! equal a full recompute after every edit of randomized edit sequences -- text edits,
//! level and list changes, paragraphs made list items and taken out, blocks inserted and
//! removed -- on made-up documents.
// Tests may unwrap, panic and index (docs/architecture/README.md, section 2.5).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

#[path = "support/labels.rs"]
mod labels;

use labels::{full, generate, labels, some_item, Block, Budget, ListItems, Rng};
use ooxml_derive::{BlockFacts, Edit};
use std::sync::Arc;

fn check(facts: &BlockFacts<ListItems>, blocks: &[Block], rng: &mut Rng) {
    let oracle = full(blocks);
    assert_eq!(labels(facts.values(), 0, blocks.len()).unwrap(), oracle);
    let from = rng.below(blocks.len() as u64 + 1) as usize;
    let to = from + rng.below((blocks.len() - from) as u64 + 1) as usize;
    assert_eq!(labels(facts.values(), from, to).unwrap(), oracle[from..to]);
}

#[test]
fn incremental_labels_equal_a_full_recompute_after_every_edit() {
    for seed in 1..=8u64 {
        let mut rng = Rng(seed.wrapping_mul(0x2545_F491_4F6C_DD1D));
        let mut blocks = generate(1_500, seed);
        let mut facts = BlockFacts::new(ListItems, Arc::new(Budget::new(1 << 20)), &blocks);
        check(&facts, &blocks, &mut rng);
        for step in 0..400 {
            let at = rng.below(blocks.len() as u64) as usize;
            match rng.below(6) {
                0 => blocks[at].text.push_str(" edited"),
                1 if blocks[at].list != 0 => blocks[at].level = rng.below(9) as u8,
                2 => (blocks[at].list, blocks[at].level) = some_item(&mut rng),
                3 => (blocks[at].list, blocks[at].level) = (0, 0),
                4 => {
                    let (list, level) = if rng.below(2) == 0 { some_item(&mut rng) } else { (0, 0) };
                    let block = Block {
                        text: format!("inserted {step}"),
                        list,
                        level,
                    };
                    facts.apply(Edit::Insert { at, block: &block }).unwrap();
                    blocks.insert(at, block);
                    check(&facts, &blocks, &mut rng);
                    continue;
                }
                5 if blocks.len() > 1 => {
                    facts.apply(Edit::Remove { at }).unwrap();
                    blocks.remove(at);
                    check(&facts, &blocks, &mut rng);
                    continue;
                }
                _ => {}
            }
            facts.apply(Edit::Set { at, block: &blocks[at] }).unwrap();
            check(&facts, &blocks, &mut rng);
        }
        assert!(facts.values().is_well_formed());
        assert!(facts.memo_stats().hits > 0, "unchanged content is not computed again");
    }
}

#[test]
fn at_full_size_one_edit_changes_exactly_the_labels_a_recompute_changes() {
    let mut blocks = generate(70_273, 61);
    let mut facts = BlockFacts::new(ListItems, Arc::new(Budget::new(64 << 20)), &blocks);
    let before = full(&blocks);
    let at = (5_000..blocks.len())
        .find(|&b| blocks[b].list == 4 && blocks[b].level == 0)
        .unwrap();
    blocks[at].level = 1;
    facts.apply(Edit::Set { at, block: &blocks[at] }).unwrap();
    let after = full(&blocks);
    assert_eq!(labels(facts.values(), 0, blocks.len()).unwrap(), after);
    assert!(
        before.iter().zip(&after).filter(|(a, b)| a != b).count() > 1_000,
        "a level change late-numbers its list"
    );
}
