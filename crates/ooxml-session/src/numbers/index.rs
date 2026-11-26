//! Owns: the index of values a check of numbers compares with (ooxml-edit's `numbers.Index`,
//! ported exactly): every number of some datasets and tables, sorted twice -- as stored (and
//! as each column's number format shows it), and times 100 (a fraction quoted as a
//! percentage) -- the values a number written matches within the rounding it shows
//! ([`Index::matches`]), and the nearest ([`Index::nearest`]).
//!
//! Must not: read a text, or resolve a source (the host gives the datasets).

use super::read::{shown, Found};
use crate::facts::is_number;
use ooxml_numfmt::Locale;
use ooxml_tools::datasets::Dataset;
use ooxml_tools::sources::{label, Cell};
use std::sync::Arc;

/// A dataset or table whose values are indexed: the address a reference names it by
/// (`d:by_topic`, `t:Scores`, `d2/t:Scores`), its form as written to the model, the rows.
#[derive(Debug, Clone)]
pub struct Source {
    /// The address a reference names it by.
    pub address: String,
    /// Its unqualified form where one document takes its prefix.
    pub shown: String,
    /// Its rows.
    pub dataset: Arc<Dataset>,
}

/// One indexed value: its source, row and column, the number, whether it is a percentage
/// (a `percent` column, a `%` format) and its format's reading ([`shown`]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Value {
    /// Its source's place in the index.
    pub source: usize,
    /// Its row.
    pub row: usize,
    /// Its column.
    pub column: usize,
    /// The number.
    pub number: f64,
    /// Whether it is a percentage's fraction.
    pub percent: bool,
    /// What its number format shows of it.
    pub format: Option<(usize, bool, u64)>,
}

/// Where a number is looked for: as stored (and shown), or times 100.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum How {
    /// As stored, or as its format shows it.
    Plain,
    /// Times 100: a fraction quoted as a percentage.
    Hundred,
}

/// Keys sorted, each with its value's place.
#[derive(Debug, Default)]
struct Table {
    keys: Vec<f64>,
    ids: Vec<usize>,
}

impl Table {
    fn of(mut pairs: Vec<(f64, usize)>) -> Table {
        pairs.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        Table {
            keys: pairs.iter().map(|(k, _)| *k).collect(),
            ids: pairs.iter().map(|(_, i)| *i).collect(),
        }
    }

    /// The values whose keys are within `low..=high`.
    fn range(&self, low: f64, high: f64) -> &[usize] {
        let first = self.keys.partition_point(|k| *k < low);
        let last = self.keys.partition_point(|k| *k <= high);
        self.ids.get(first..last.max(first)).unwrap_or_default()
    }
}

/// Every number of some datasets and tables.
#[derive(Debug)]
pub struct Index {
    /// The sources, in order.
    pub sources: Vec<Source>,
    /// The values, in order.
    pub values: Vec<Value>,
    keys: Vec<Vec<Option<String>>>,
    plain: Table,
    hundred: Table,
}

/// `value` rounded half up to `decimals` (`_rounded`): an index key, compared within a
/// tolerance, so a float's last bit does not matter.
fn rounded(value: f64, decimals: usize) -> f64 {
    let scale = 10_f64.powi(i32::try_from(decimals).unwrap_or(i32::MAX));
    (value * scale + 0.5 + 1e-9).floor() / scale
}

/// A number cell's value as a float (`float(item)`).
fn float_of(cell: &Cell) -> Option<f64> {
    match cell {
        Cell::Float(f) => Some(*f),
        Cell::Int(digits) => digits.parse().ok(),
        _ => None,
    }
}

impl Index {
    /// The index of `sources`' numbers.
    #[must_use]
    pub fn new(sources: Vec<Source>) -> Index {
        let mut values = Vec::new();
        let mut plain: Vec<(f64, usize)> = Vec::new();
        let mut hundred: Vec<(f64, usize)> = Vec::new();
        for (at, source) in sources.iter().enumerate() {
            let dataset = &source.dataset;
            let formats = dataset.formats().unwrap_or_default();
            let types = dataset.types().unwrap_or_default();
            let readings: Vec<Option<(usize, bool, u64)>> = (0..dataset.columns().len())
                .map(|c| shown(formats.get(c).and_then(Option::as_deref)))
                .collect();
            let percents: Vec<bool> = readings
                .iter()
                .enumerate()
                .map(|(c, reading)| {
                    types.get(c).and_then(Option::as_deref) == Some("percent") || reading.is_some_and(|r| r.1)
                })
                .collect();
            for (r, row) in dataset.rows().iter().enumerate() {
                for (c, item) in row.iter().enumerate() {
                    let Some(number) = float_of(item).filter(|_| is_number(item)) else {
                        continue;
                    };
                    let reading = readings.get(c).copied().flatten();
                    let i = values.len();
                    values.push(Value {
                        source: at,
                        row: r,
                        column: c,
                        number,
                        percent: percents.get(c).copied().unwrap_or(false),
                        format: reading,
                    });
                    let magnitude = number.abs();
                    plain.push((magnitude, i));
                    hundred.push((magnitude * 100.0, i));
                    if let Some((decimals, pct, divide)) = reading {
                        #[allow(clippy::cast_precision_loss)]
                        let display = magnitude * if pct { 100.0 } else { 1.0 } / divide as f64;
                        let round = rounded(display, decimals);
                        #[allow(clippy::float_cmp)]
                        if round != display {
                            if pct { &mut hundred } else { &mut plain }.push((round, i));
                        }
                        if divide != 1 {
                            plain.push((display, i));
                        }
                    }
                }
            }
        }
        let keys = sources.iter().map(|s| keys_of(&s.dataset)).collect();
        Index {
            sources,
            values,
            keys,
            plain: Table::of(plain),
            hundred: Table::of(hundred),
        }
    }

    /// How many values it holds.
    #[must_use]
    pub fn len(&self) -> usize {
        self.values.len()
    }

    /// Whether it holds none.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    /// The rows' keys of the source `source`: each first column's value as a cell writes it.
    #[must_use]
    pub fn keys(&self, source: usize) -> &[Option<String>] {
        self.keys.get(source).map_or(&[], Vec::as_slice)
    }

    fn table(&self, how: How) -> &Table {
        match how {
            How::Plain => &self.plain,
            How::Hundred => &self.hundred,
        }
    }

    /// Where `number` is looked for (`targets`): `(table, centre, half width, percentages)`
    /// -- `percentages`: whether a percentage's value may match there. A percentage
    /// matches only when written as one, as its fraction; a number written without `%`, `‰`
    /// or a percent word matches plain values only.
    fn targets(number: &Found) -> Vec<(How, f64, f64, bool)> {
        let t = number.value.to_f64();
        let half = 0.5 * 10_f64.powi(-i32::try_from(number.decimals).unwrap_or(i32::MAX));
        match number.percent {
            Some(100) => vec![(How::Hundred, t, half, true), (How::Plain, t, half, false)],
            Some(_) => vec![
                (How::Plain, t / 1000.0, half / 1000.0, true),
                (How::Plain, t, half, false),
            ],
            None => {
                let mut out = vec![(How::Plain, t, half, false)];
                if let Some(scale) = number.scale {
                    #[allow(clippy::cast_precision_loss)]
                    let factor = scale as f64;
                    out.push((How::Plain, t * factor, half * factor, false));
                }
                out
            }
        }
    }

    /// Whether the value `i` may be found for `number` where percentages are or are not
    /// taken: a percentage only where they are, a positive value never for a sign written.
    fn takes(&self, i: usize, number: &Found, percentages: bool) -> bool {
        self.values
            .get(i)
            .is_some_and(|v| !(v.percent && !percentages) && !(number.negative && v.number > 0.0))
    }

    /// The values `number` matches, each once, with how (`matches`), by place.
    #[must_use]
    pub fn matches(&self, number: &Found) -> Vec<(usize, How)> {
        let mut found: Vec<(usize, How)> = Vec::new();
        for (how, centre, half, percentages) in Self::targets(number) {
            let slack = half * (1.0 + 1e-9) + centre.abs() * 1e-12;
            for &i in self.table(how).range(centre - slack, centre + slack) {
                if self.takes(i, number, percentages) && !found.iter().any(|(j, _)| *j == i) {
                    found.push((i, how));
                }
            }
        }
        found.sort_by_key(|(i, _)| *i);
        found
    }

    /// The `count` values nearest `number` by relative distance, each once: `(value, how,
    /// distance)` (`nearest`).
    #[must_use]
    pub fn nearest(&self, number: &Found, count: usize) -> Vec<(usize, How, f64)> {
        let mut best: Vec<(usize, f64, How)> = Vec::new();
        for (how, centre, _, percentages) in Self::targets(number) {
            let table = self.table(how);
            let at = table.keys.partition_point(|k| *k < centre);
            let first = at.saturating_sub(count + 2);
            let last = (at + count + 2).min(table.keys.len());
            for j in first..last {
                let (Some(&i), Some(&key)) = (table.ids.get(j), table.keys.get(j)) else {
                    continue;
                };
                if !self.takes(i, number, percentages) {
                    continue;
                }
                let distance = (key - centre).abs() / centre.abs().max(1e-9);
                match best.iter_mut().find(|(k, _, _)| *k == i) {
                    Some(kept) if distance < kept.1 => *kept = (i, distance, how),
                    Some(_) => {}
                    None => best.push((i, distance, how)),
                }
            }
        }
        best.sort_by(|a, b| a.1.total_cmp(&b.1));
        best.into_iter().take(count).map(|(i, d, how)| (i, how, d)).collect()
    }
}

/// The rows' keys of `dataset`: each first column's value as a cell writes it, in no locale.
fn keys_of(dataset: &Dataset) -> Vec<Option<String>> {
    let kind = dataset.types().and_then(|t| t.first()).and_then(Option::as_deref);
    let locale = Locale::default();
    dataset
        .rows()
        .iter()
        .map(|row| row.first().and_then(|cell| label(cell, &locale, kind, None)))
        .collect()
}
