//! Times the reference fact (design section 15, acceptance 2): list labels kept with
//! ooxml-derive after one block edit, on a made-up document the size of the 3k report
//! (70,273 blocks, about one in eight a list item), measured the way spike 3 was: an edit
//! after block 5,000, then the labels of 300 blocks read late in the document or at the
//! edit. Each kind of edit is timed on 101 different blocks and every result is checked
//! against a full recompute (untimed).
//!
//! `cargo run --release -p ooxml-derive --example list_labels -- [blocks] [seed]` prints one
//! JSON object: wall time per update and read in µs (minimum and median over the blocks), of
//! the update alone (`apply_*`), the build
//! and the full recompute. It runs on one thread, so its CPU time is at most its wall time;
//! measure the process's CPU with `/usr/bin/time`.

#[path = "../tests/support/labels.rs"]
mod labels;

use labels::{full, generate, labels, Block, Budget, ListItems};
use ooxml_derive::{BlockFacts, DeriveError, Edit};
use std::sync::Arc;
use std::time::Instant;

/// Blocks timed per kind of edit.
const TRIALS: usize = 101;
/// Blocks read after an edit.
const READ: usize = 300;

/// What an edit does to a block, and how to put it back.
#[derive(Clone, Copy)]
enum Change {
    Text,
    Level,
    MadeItem,
    TakenOut,
}

impl Change {
    fn name(self) -> &'static str {
        match self {
            Change::Text => "text only (no list change)",
            Change::Level => "level of an item in a long numbered list",
            Change::MadeItem => "a paragraph made a list item",
            Change::TakenOut => "a list item taken out of its list",
        }
    }

    /// Whether the edit applies to `block`.
    fn fits(self, block: &Block) -> bool {
        match self {
            Change::Text | Change::MadeItem => block.list == 0,
            Change::Level | Change::TakenOut => block.list == 4 && block.level == 0,
        }
    }

    fn apply(self, block: &mut Block) {
        match self {
            Change::Text => block.text.push_str(" edited"),
            Change::Level => block.level = 1,
            Change::MadeItem => (block.list, block.level) = (4, 0),
            Change::TakenOut => (block.list, block.level) = (0, 0),
        }
    }
}

/// Wall times in µs: the smallest and the median.
fn spread(mut times: Vec<f64>) -> (f64, f64) {
    times.sort_by(f64::total_cmp);
    let min = times.first().copied().unwrap_or(f64::NAN);
    let median = times.get(times.len() / 2).copied().unwrap_or(f64::NAN);
    (min, median)
}

fn micros(since: Instant) -> f64 {
    since.elapsed().as_secs_f64() * 1e6
}

struct Row {
    change: Change,
    read: &'static str,
    min_us: f64,
    median_us: f64,
    apply_min_us: f64,
    apply_median_us: f64,
}

/// Time `change` on `TRIALS` blocks after block 5,000, reading late or at the edit; each
/// block is put back afterwards (untimed).
fn time_change(
    facts: &mut BlockFacts<ListItems>,
    blocks: &mut [Block],
    change: Change,
    late: bool,
) -> Result<(Vec<f64>, Vec<f64>), Box<dyn std::error::Error>> {
    let targets: Vec<usize> = (5_000..blocks.len())
        .filter(|&at| blocks.get(at).is_some_and(|b| change.fits(b)))
        .take(TRIALS)
        .collect();
    let mut times = Vec::with_capacity(targets.len());
    let mut applies = Vec::with_capacity(targets.len());
    for at in targets {
        let original = blocks
            .get(at)
            .cloned()
            .ok_or(DeriveError::OutOfRange { at, len: blocks.len() })?;
        let mut edited = original.clone();
        change.apply(&mut edited);
        let (from, to) = if late {
            (60_000, 60_000 + READ)
        } else {
            (at.saturating_sub(10), at.saturating_sub(10) + READ)
        };
        let to = to.min(blocks.len());
        let start = Instant::now();
        facts.apply(Edit::Set { at, block: &edited })?;
        applies.push(micros(start));
        let read = labels(facts.values(), from.min(to), to)?;
        times.push(micros(start));
        if let Some(slot) = blocks.get_mut(at) {
            *slot = edited;
        }
        if full(blocks).get(from.min(to)..to) != Some(read.as_slice()) {
            return Err(format!("labels differ from a full recompute after editing block {at}").into());
        }
        facts.apply(Edit::Set { at, block: &original })?;
        if let Some(slot) = blocks.get_mut(at) {
            *slot = original;
        }
    }
    Ok((times, applies))
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let count: usize = args.next().map_or(Ok(70_273), |a| a.parse())?;
    let seed: u64 = args.next().map_or(Ok(61), |a| a.parse())?;
    let mut blocks = generate(count, seed);
    if count < 60_000 + READ {
        return Err(format!("needs at least {} blocks to read late", 60_000 + READ).into());
    }

    let start = Instant::now();
    let oracle = full(&blocks);
    let full_us = micros(start);
    let start = Instant::now();
    let mut facts = BlockFacts::new(ListItems, Arc::new(Budget::new(64 << 20)), &blocks);
    let build_us = micros(start);
    if labels(facts.values(), 0, blocks.len())? != oracle {
        return Err("the built labels differ from a full recompute".into());
    }

    let mut rows = Vec::new();
    for change in [Change::Text, Change::Level, Change::MadeItem, Change::TakenOut] {
        for (read, late) in [("late chapter (blocks 60,000-60,300)", true), ("at the edit", false)] {
            let (times, applies) = time_change(&mut facts, &mut blocks, change, late)?;
            let ((min_us, median_us), (apply_min_us, apply_median_us)) = (spread(times), spread(applies));
            rows.push(Row {
                change,
                read,
                min_us,
                median_us,
                apply_min_us,
                apply_median_us,
            });
        }
    }

    let items = blocks.iter().filter(|b| b.list != 0).count();
    let lists = blocks
        .iter()
        .filter(|b| b.list != 0)
        .map(|b| b.list)
        .collect::<std::collections::BTreeSet<_>>()
        .len();
    let stats = facts.memo_stats();
    let rows: Vec<String> = rows
        .iter()
        .map(|r| {
            format!(
                "{{\"edit\": \"{}\", \"read\": \"{}\", \"trials\": {TRIALS}, \"min_us\": {:.2}, \"median_us\": {:.2}, \
                 \"apply_min_us\": {:.2}, \"apply_median_us\": {:.2}}}",
                r.change.name(),
                r.read,
                r.min_us,
                r.median_us,
                r.apply_min_us,
                r.apply_median_us
            )
        })
        .collect();
    println!(
        "{{\"blocks\": {count}, \"seed\": {seed}, \"list_items\": {items}, \"lists\": {lists}, \"all_equal_to_full\": true, \
         \"full_recompute_us\": {full_us:.1}, \"build_us\": {build_us:.1}, \"memo\": {{\"hits\": {}, \"misses\": {}, \"entries\": {}, \"bytes\": {}}}, \
         \"updates\": [{}]}}",
        stats.hits,
        stats.misses,
        stats.entries,
        stats.bytes,
        rows.join(", ")
    );
    Ok(())
}
