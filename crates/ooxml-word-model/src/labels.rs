//! Owns: list labels as Word counts them (measured, docx2svg's `ListCounters`;
//! `make_numbering_probe.py`): instances over one abstract definition share its counts; a
//! `w:startOverride` restarts a level at the instance's first item there; an item sets
//! every shallower level without a count to its start and restarts the deeper levels its
//! `w:lvlRestart` allows; a `w:isLgl` level shows every number in decimal.
//!
//! Two ways to the same labels: [`Counters`], from scratch in document order (the oracle),
//! and [`Lists`], the counting as a monoid -- each item a transform of its list's counters,
//! transforms composing associatively -- so that a story's labels are kept in a
//! `ooxml_derive::MonoidTree` and an edit recounts O(log n) (design section 7, spike 3).
//! Per level a transform leaves the count as it is, sets it, or "starts from `d` if unset, adds
//! `k`"; an override's restart, which depends on whether its pair was used before, is kept
//! as a condition resolved when segments compose.
//!
//! Must not: resolve which list a paragraph is in (`crate::resolve`).

use crate::numbering::{Level, ListKey, Numbering};
use ooxml_derive::{Measured, Monoid};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;

/// A list instance and level: `(numId, ilvl)`.
pub type Pair = (i64, i64);

fn start_of(level: Option<&Arc<Level>>) -> i64 {
    level.and_then(|l| l.start).unwrap_or(1)
}

fn roman(mut value: i64) -> String {
    let mut out = String::new();
    for (number, letters) in [
        (1000, "m"),
        (900, "cm"),
        (500, "d"),
        (400, "cd"),
        (100, "c"),
        (90, "xc"),
        (50, "l"),
        (40, "xl"),
        (10, "x"),
        (9, "ix"),
        (5, "v"),
        (4, "iv"),
        (1, "i"),
    ] {
        while value >= number {
            out.push_str(letters);
            value -= number;
        }
    }
    out
}

/// A number in a list level's format (`decimal`, `lowerLetter`, `upperRoman`...).
pub fn format_number(value: i64, format: Option<&str>) -> String {
    match format {
        Some(f @ ("lowerLetter" | "upperLetter")) => {
            let letter =
                char::from_u32(u32::from(b'a') + u32::try_from((value - 1).rem_euclid(26)).unwrap_or(0)).unwrap_or('a');
            let repeat = usize::try_from((value - 1).div_euclid(26) + 1).unwrap_or(0);
            let s: String = std::iter::repeat_n(letter, repeat).collect();
            if f == "upperLetter" {
                s.to_uppercase()
            } else {
                s
            }
        }
        Some(f @ ("lowerRoman" | "upperRoman")) => {
            let r = roman(value);
            if f == "upperRoman" {
                r.to_uppercase()
            } else {
                r
            }
        }
        Some("bullet" | "none") => String::new(),
        _ => value.to_string(),
    }
}

/// The label text of an item at `ilvl` whose list's counts are `count` (`None`: unset).
fn label_text(levels: &BTreeMap<i64, Arc<Level>>, ilvl: i64, count: &dyn Fn(i64) -> Option<i64>) -> Option<String> {
    let level = levels.get(&ilvl)?;
    let mut text = level.text.as_deref()?.to_owned();
    for number in 0..9 {
        let token = format!("%{}", number + 1);
        if !text.contains(&token) {
            continue;
        }
        let other = levels.get(&number);
        let value = count(number).unwrap_or_else(|| start_of(other));
        let mut format = other.and_then(|o| o.format.as_deref());
        if level.legal && !matches!(format, Some("bullet" | "none")) {
            format = Some("decimal");
        }
        text = text.replace(&token, &format_number(value, format));
    }
    Some(text)
}

/// The running numbers of a story's lists, counted from scratch in document order.
#[derive(Debug, Clone, Default)]
pub struct Counters {
    counts: HashMap<ListKey, BTreeMap<i64, i64>>,
    used: BTreeSet<Pair>,
}

impl Counters {
    /// Count the next item of instance `num_id` at `ilvl`: its label and running number,
    /// both `None` where the instance has no such level or the level no `w:lvlText`.
    pub fn item_number(&mut self, numbering: &Numbering, num_id: i64, ilvl: i64) -> (Option<String>, Option<i64>) {
        let Some(label) = self.item(numbering, num_id, ilvl) else {
            return (None, None);
        };
        let number = self
            .counts
            .get(&numbering.list_of(num_id))
            .and_then(|c| c.get(&ilvl))
            .copied();
        (Some(label), number)
    }

    /// Whether the next item of instance `num_id` at `ilvl` begins its count, read before it
    /// is counted: no item of its list at that level since the count last began (a
    /// shallower item, the story's start), or a restart (`w:startOverride`) its instance
    /// makes at its first item there.
    #[must_use]
    pub fn begins(&self, numbering: &Numbering, num_id: i64, ilvl: i64) -> bool {
        let counted = self
            .counts
            .get(&numbering.list_of(num_id))
            .is_some_and(|c| c.contains_key(&ilvl));
        !counted || (!self.used.contains(&(num_id, ilvl)) && numbering.restarts.contains(&(num_id, ilvl)))
    }

    /// Count the next item of instance `num_id` at `ilvl`; its label, or `None` where the
    /// instance has no such level or the level no `w:lvlText`.
    pub fn item(&mut self, numbering: &Numbering, num_id: i64, ilvl: i64) -> Option<String> {
        let empty = BTreeMap::new();
        let levels = if num_id == 0 {
            &empty
        } else {
            numbering.levels.get(&num_id).unwrap_or(&empty)
        };
        let level = levels.get(&ilvl).filter(|l| l.text.is_some())?;
        let counts = self.counts.entry(numbering.list_of(num_id)).or_default();
        let first = self.used.insert((num_id, ilvl));
        let next = if first && numbering.restarts.contains(&(num_id, ilvl)) {
            start_of(Some(level))
        } else {
            counts.get(&ilvl).copied().unwrap_or(start_of(Some(level)) - 1) + 1
        };
        counts.insert(ilvl, next);
        for shallower in 0..ilvl {
            counts
                .entry(shallower)
                .or_insert_with(|| start_of(levels.get(&shallower)));
        }
        let deeper: Vec<i64> = counts.range(ilvl + 1..).map(|(k, _)| *k).collect();
        for k in deeper {
            if ilvl < levels.get(&k).and_then(|l| l.restart).unwrap_or(k) {
                counts.remove(&k);
            }
        }
        let snapshot = counts.clone();
        label_text(levels, ilvl, &|n| snapshot.get(&n).copied())
    }
}

/// What a segment does to one level of a list's counters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LevelFn {
    /// Unchanged.
    Same,
    /// Set (to nothing: removed).
    Set(Option<i64>),
    /// `x -> (x, or d where unset) + k`.
    Shift(i64, i64),
}

impl LevelFn {
    /// `self` then `next`.
    fn then(self, next: LevelFn) -> LevelFn {
        match (self, next) {
            (f, LevelFn::Same) => f,
            (LevelFn::Same, g) | (_, g @ LevelFn::Set(_)) => g,
            (LevelFn::Set(c), LevelFn::Shift(d, k)) => LevelFn::Set(Some(c.unwrap_or(d) + k)),
            (LevelFn::Shift(d, a), LevelFn::Shift(_, b)) => LevelFn::Shift(d, a + b),
        }
    }

    fn apply(self, x: Option<i64>) -> Option<i64> {
        match self {
            LevelFn::Same => x,
            LevelFn::Set(c) => c,
            LevelFn::Shift(d, k) => Some(x.unwrap_or(d) + k),
        }
    }
}

/// One level's transform: as though every restart in it were used before, and the
/// restarts that are their pair's first in the segment, each with the value the level has
/// at the segment's end when it is the last to restart.
#[derive(Debug, Clone, PartialEq, Eq)]
struct LevelSummary {
    base: LevelFn,
    conds: Vec<(Pair, Option<i64>)>,
}

impl LevelSummary {
    fn then(&self, next: &LevelSummary, used_before: &BTreeSet<Pair>) -> LevelSummary {
        let base = self.base.then(next.base);
        let mut conds = if matches!(next.base, LevelFn::Set(_)) {
            Vec::new()
        } else {
            self.conds
                .iter()
                .map(|(p, c)| (*p, LevelFn::Set(*c).then(next.base).apply(None)))
                .collect()
        };
        conds.extend(next.conds.iter().filter(|(p, _)| !used_before.contains(p)).copied());
        LevelSummary { base, conds }
    }

    fn value(&self, x: Option<i64>) -> Option<i64> {
        match self.conds.last() {
            Some((_, c)) => *c,
            None => self.base.apply(x),
        }
    }
}

/// What a segment does to one list's counters. `used` holds the restarting pairs (those
/// with a `w:startOverride`) the segment has items of: only they read whether a pair was
/// used before.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ListFn {
    levels: BTreeMap<i64, LevelSummary>,
    used: BTreeSet<Pair>,
}

impl ListFn {
    fn then(&self, next: &ListFn) -> ListFn {
        let mut levels = self.levels.clone();
        for (k, n) in &next.levels {
            let made = match self.levels.get(k) {
                Some(mine) => mine.then(n, &self.used),
                None => LevelSummary {
                    base: n.base,
                    conds: n
                        .conds
                        .iter()
                        .filter(|(p, _)| !self.used.contains(p))
                        .copied()
                        .collect(),
                },
            };
            levels.insert(*k, made);
        }
        let used = if next.used.is_empty() {
            self.used.clone()
        } else {
            self.used.union(&next.used).copied().collect()
        };
        ListFn { levels, used }
    }

    /// The level's count after the segment, from no counts.
    pub fn count(&self, level: i64) -> Option<i64> {
        self.levels.get(&level).and_then(|l| l.value(None))
    }
}

/// What a segment does to every list's counters: the aggregate of a story's labels.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Lists(BTreeMap<ListKey, Arc<ListFn>>);

impl Monoid for Lists {
    fn empty() -> Self {
        Lists(BTreeMap::new())
    }

    fn combine(&self, other: &Self) -> Self {
        if other.0.is_empty() {
            return self.clone();
        }
        let mut out = self.0.clone();
        for (k, f) in &other.0 {
            let made = match self.0.get(k) {
                Some(mine) => Arc::new(mine.then(f)),
                None => Arc::clone(f),
            };
            out.insert(*k, made);
        }
        Lists(out)
    }
}

impl Lists {
    /// The list's transform, if the segment has items in it.
    pub fn list(&self, key: ListKey) -> Option<&ListFn> {
        self.0.get(&key).map(|f| &**f)
    }
}

/// One counted list item: its instance, level and list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Item {
    /// `numId`.
    pub num_id: i64,
    /// `ilvl`.
    pub ilvl: i64,
}

/// The transform one item makes, or `None` where it counts nothing.
pub fn item_lists(numbering: &Numbering, item: Item, top_level: i64) -> Option<Lists> {
    let Item { num_id, ilvl } = item;
    let levels = numbering.levels.get(&num_id).filter(|_| num_id != 0)?;
    let level = levels.get(&ilvl).filter(|l| l.text.is_some())?;
    let start = start_of(Some(level));
    let mut f = ListFn::default();
    let restart = numbering.restarts.contains(&(num_id, ilvl));
    let own = LevelSummary {
        base: LevelFn::Shift(start - 1, 1),
        conds: if restart {
            vec![((num_id, ilvl), Some(start))]
        } else {
            Vec::new()
        },
    };
    f.levels.insert(ilvl, own);
    for shallower in 0..ilvl {
        let base = LevelFn::Shift(start_of(levels.get(&shallower)), 0);
        f.levels.insert(
            shallower,
            LevelSummary {
                base,
                conds: Vec::new(),
            },
        );
    }
    for k in ilvl + 1..=top_level {
        if ilvl < levels.get(&k).and_then(|l| l.restart).unwrap_or(k) {
            f.levels.insert(
                k,
                LevelSummary {
                    base: LevelFn::Set(None),
                    conds: Vec::new(),
                },
            );
        }
    }
    if restart {
        f.used.insert((num_id, ilvl));
    }
    Some(Lists(BTreeMap::from([(numbering.list_of(num_id), Arc::new(f))])))
}

/// The deepest level any definition of the part has: the counters' extent.
pub fn top_level(numbering: &Numbering) -> i64 {
    numbering
        .levels
        .values()
        .flat_map(|l| l.keys())
        .copied()
        .max()
        .unwrap_or(0)
        .max(8)
}

/// The label of an item at `(num_id, ilvl)` given everything up to and including it.
pub fn label_after(numbering: &Numbering, prefix: &Lists, item: Item) -> Option<String> {
    let empty = BTreeMap::new();
    let levels = numbering
        .levels
        .get(&item.num_id)
        .filter(|_| item.num_id != 0)
        .unwrap_or(&empty);
    levels.get(&item.ilvl).filter(|l| l.text.is_some())?;
    let list = prefix.list(numbering.list_of(item.num_id));
    label_text(levels, item.ilvl, &|n| list.and_then(|l| l.count(n)))
}

/// A block's list items, and their composed transform: what a story's tree holds per
/// top-level block.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BlockItems {
    /// Each paragraph's list item (`None`: not one) with its transform (`None`: it counts
    /// nothing), in document order, cells included.
    pub items: Arc<[(Option<Item>, Option<Lists>)]>,
    summary: Lists,
}

impl BlockItems {
    /// A block's items.
    pub fn new(items: Vec<(Option<Item>, Option<Lists>)>) -> BlockItems {
        let summary = items
            .iter()
            .filter_map(|(_, l)| l.as_ref())
            .fold(Lists::empty(), |acc, l| acc.combine(l));
        BlockItems {
            items: Arc::from(items),
            summary,
        }
    }
}

impl Measured for BlockItems {
    type Summary = Lists;
    fn measure(&self) -> Lists {
        self.summary.clone()
    }
}

impl ooxml_derive::Weigh for BlockItems {
    fn weight(&self) -> usize {
        64 + self.items.len() * 96
    }
}
