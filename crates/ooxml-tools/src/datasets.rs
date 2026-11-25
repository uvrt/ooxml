//! Owns: a dataset as a value (design phase 3, section 5; ooxml-edit's `datasets.py`, ported
//! exactly): [`Dataset`] -- columns, rows of [`Cell`]s, each value's own sources, how many
//! leading columns name a row, a type label per column, what the producer keeps for itself,
//! the formula caches its values were read from ([`Cache`]), each column's number format, and
//! the Markdown of its rich text values, given or worked out when first asked for
//! ([`Markdown`], [`LazyMarkdown`]) --, checked as it is made ([`DatasetError`]), made of
//! records ([`Dataset::from_records`]), a run of its rows ([`Dataset::row_slice`]) and the
//! rows a source hands a call ([`Dataset::source_rows`]); and a dataset's address: its id
//! ([`dataset_id`], [`check_dataset_id`]) and a run of its rows (`d:x/#9:#16`:
//! [`row_range`], [`rows_text`]).
//!
//! A dataset is format-neutral and immutable: the tool layer stores it and hands it on,
//! and reads its sources, its detail and its formats never.
//!
//! Must not: keep datasets, say whether one is stale (the session's store does), or
//! resolve a source.

use crate::sources::{Cell, SourceRows};
use crate::{Refusal, ToolError, TryCall, WayOut};
use ooxml_base::pyfmt::repr_str;
use serde_json::{Map, Value};
use std::any::Any;
use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

/// What every dataset address starts with: `d:by_region`. A document handle (`d1`) has no
/// colon, so the two never meet.
pub const DATASET_PREFIX: &str = "d:";

/// The pattern a dataset's id matches, as a refusal names it.
pub const DATASET_ID: &str = r"^[A-Za-z_][A-Za-z0-9_.\-]{0,63}$";

/// The column type labels a dataset may carry: what the format libraries read.
pub const TYPE_LABELS: [&str; 8] = ["number", "percent", "date", "time", "datetime", "text", "bool", "error"];

/// A formula's cached value, as a producer reads it: as last saved, invalidated since, or
/// none at all.
pub const CACHE_STATUSES: [&str; 3] = ["cached_unverified", "invalidated", "absent"];

/// What [`Cache`] counts beside the statuses: formulas that refer to another file.
pub const CACHE_EXTERNAL: &str = "external";

/// The most columns `describe` names per dataset, and the most datasets it lists.
pub const BRIEF_COLUMNS: usize = 20;

/// The most datasets `describe` lists, the latest defined.
pub const BRIEF_DATASETS: usize = 20;

/// Python's `$`: the text without one final newline, which a pattern's end also matches
/// before.
fn py_end(text: &str) -> &str {
    text.strip_suffix('\n').unwrap_or(text)
}

/// Whether `id` is a dataset's id ([`DATASET_ID`]).
fn is_dataset_id(id: &str) -> bool {
    let id = py_end(id);
    let mut chars = id.chars();
    let Some(first) = chars.next() else { return false };
    (first.is_ascii_alphabetic() || first == '_')
        && id.chars().count() <= 64
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
}

/// The id an address names: `by_region` of `d:by_region`, `d:by_region[total]` or
/// `d:by_region/North/total` (what follows the id is the format's to read); `None` when
/// `address` is no `d:` address.
#[must_use]
pub fn dataset_id(address: &str) -> Option<&str> {
    let body = address.strip_prefix(DATASET_PREFIX)?;
    Some(body.find(['[', '/']).map_or(body, |end| &body[..end]))
}

/// The digits of a row number in a run (`[1-9][0-9]{0,6}`), as a number.
fn row_number(text: &str) -> Option<usize> {
    let ok = text.len() <= 7
        && text.starts_with(|c: char| ('1'..='9').contains(&c))
        && text.bytes().all(|b| b.is_ascii_digit());
    ok.then(|| text.parse().ok()).flatten()
}

/// The id and the rows a run of a dataset's rows names: `("by_region", 9, Some(16))` of
/// `d:by_region/#9:#16` (from 1, the last included; given the other way round, the same),
/// `("by_region", 9, None)` of `d:by_region/#9:` (to the last row), `("by_region", 9,
/// Some(9))` of `d:by_region/#9`. `None` for anything else: a whole dataset, a column, a
/// row by key.
#[must_use]
pub fn row_range(address: &str) -> Option<(&str, usize, Option<usize>)> {
    let id = dataset_id(address)?;
    let rest = py_end(address.get(DATASET_PREFIX.len() + id.len()..)?);
    let rest = rest.strip_prefix("/#")?;
    let (first, last) = match rest.split_once(':') {
        None => (rest, Some(rest)),
        Some((first, "")) => (first, None),
        Some((first, last)) => (first, Some(last.strip_prefix('#')?)),
    };
    let first = row_number(first)?;
    match last {
        None => Some((id, first, None)),
        Some(last) => {
            let last = row_number(last)?;
            Some((id, first.min(last), Some(first.max(last))))
        }
    }
}

/// A run of rows as an address writes it after an id: `/#9:#16`, `/#9:`, `/#9`.
#[must_use]
pub fn rows_text(first: usize, last: Option<usize>) -> String {
    match last {
        None => format!("/#{first}:"),
        Some(last) if last == first => format!("/#{first}"),
        Some(last) => format!("/#{first}:#{last}"),
    }
}

/// `id` without a `d:` (given or not), or `invalid_arguments` at `field` (the call as given
/// its way out).
///
/// # Errors
/// An id that does not match [`DATASET_ID`].
pub fn check_dataset_id<'a>(id: &'a str, field: &str, retry: &TryCall) -> Result<&'a str, ToolError> {
    let id = id.strip_prefix(DATASET_PREFIX).unwrap_or(id);
    if is_dataset_id(id) {
        return Ok(id);
    }
    Err(ToolError::refusal(
        Refusal::InvalidArguments,
        format!("a dataset id must match {DATASET_ID}"),
        WayOut::call(retry.clone()),
    )
    .at(field))
}

/// A dataset the producer made wrongly (Python's `ValueError`): its words.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct DatasetError(pub String);

fn bad<T>(message: impl Into<String>) -> Result<T, DatasetError> {
    Err(DatasetError(message.into()))
}

/// How many of the formula cells a dataset's values were read from had each of
/// [`CACHE_STATUSES`], and how many refer to another file ([`CACHE_EXTERNAL`]): zeros left
/// out, in that order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cache(Vec<(&'static str, u64)>);

impl Cache {
    /// The counts by status, checked; `None` for none (all zero).
    ///
    /// # Errors
    /// A status that is none of the known ones, or a count under 0.
    pub fn new(counts: &[(&str, i64)]) -> Result<Option<Cache>, DatasetError> {
        let known: Vec<&'static str> = CACHE_STATUSES.iter().copied().chain([CACHE_EXTERNAL]).collect();
        let unknown: Vec<String> = counts
            .iter()
            .filter(|(key, _)| !known.contains(key))
            .map(|(key, _)| repr_str(key))
            .collect();
        if !unknown.is_empty() {
            return bad(format!(
                "a dataset's cache counts {}; not {}",
                known.join(", "),
                unknown.join(", ")
            ));
        }
        let mut out = Vec::new();
        for key in known {
            // the last given wins, as a mapping keeps it
            let count = counts.iter().rev().find(|(k, _)| *k == key).map_or(0, |(_, n)| *n);
            let Ok(count) = u64::try_from(count) else {
                return bad(format!("a dataset's cache count is a whole number from 0, not {count}"));
            };
            if count > 0 {
                out.push((key, count));
            }
        }
        Ok((!out.is_empty()).then_some(Cache(out)))
    }

    /// The count of `status` (0 for none).
    #[must_use]
    pub fn count(&self, status: &str) -> u64 {
        self.0.iter().find(|(k, _)| *k == status).map_or(0, |(_, n)| *n)
    }

    /// The counts as a result writes them: `{"cached_unverified": 2, "external": 1}`.
    #[must_use]
    pub fn to_json(&self) -> Map<String, Value> {
        self.0.iter().map(|(k, n)| ((*k).to_owned(), Value::from(*n))).collect()
    }
}

/// The Markdown of a dataset's text values worked out when first asked for (a producer's
/// rich text, read from its source only for the values a tool writes as blocks).
pub trait LazyMarkdown: Send + Sync {
    /// Every place, `(row, column)` from 0, that may have Markdown; nothing worked out.
    fn places(&self) -> Vec<(usize, usize)>;

    /// The Markdown at `(row, column)`, worked out now; `None` for a value that turns out to
    /// be its text.
    fn markdown(&self, row: usize, column: usize) -> Option<String>;
}

/// The Markdown of a dataset's text values whose source holds more than their characters,
/// by `(row, column)` from 0.
#[derive(Clone)]
pub enum Markdown {
    /// Given whole.
    Given(BTreeMap<(usize, usize), String>),
    /// Worked out when asked for: a producer's, and a run of its rows (from `first`, before
    /// `end`, renumbered from 0).
    Lazy {
        /// The producer's.
        whole: Arc<dyn LazyMarkdown>,
        /// The first row of the run, in the producer's numbers.
        first: usize,
        /// The row after the run's last.
        end: usize,
        /// The places in the run, renumbered.
        places: Vec<(usize, usize)>,
    },
}

impl fmt::Debug for Markdown {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Markdown::Given(map) => f.debug_tuple("Given").field(map).finish(),
            Markdown::Lazy { first, end, places, .. } => f
                .debug_struct("Lazy")
                .field("first", first)
                .field("end", end)
                .field("places", places)
                .finish_non_exhaustive(),
        }
    }
}

impl Markdown {
    /// A producer's lazy Markdown over all its rows.
    #[must_use]
    pub fn lazy(whole: Arc<dyn LazyMarkdown>) -> Markdown {
        let places = whole.places();
        Markdown::Lazy {
            whole,
            first: 0,
            end: usize::MAX,
            places,
        }
    }

    /// Every place with Markdown, `(row, column)` from 0; nothing worked out.
    #[must_use]
    pub fn places(&self) -> Vec<(usize, usize)> {
        match self {
            Markdown::Given(map) => map.keys().copied().collect(),
            Markdown::Lazy { places, .. } => places.clone(),
        }
    }

    /// Whether `(row, column)` may have Markdown, without working it out.
    #[must_use]
    pub fn contains(&self, row: usize, column: usize) -> bool {
        match self {
            Markdown::Given(map) => map.contains_key(&(row, column)),
            Markdown::Lazy { places, .. } => places.contains(&(row, column)),
        }
    }

    /// The Markdown at `(row, column)`, worked out now where it is lazy.
    #[must_use]
    pub fn get(&self, row: usize, column: usize) -> Option<String> {
        match self {
            Markdown::Given(map) => map.get(&(row, column)).cloned(),
            Markdown::Lazy {
                whole, first, places, ..
            } => places
                .contains(&(row, column))
                .then(|| whole.markdown(row + first, column))
                .flatten(),
        }
    }

    fn len(&self) -> usize {
        match self {
            Markdown::Given(map) => map.len(),
            Markdown::Lazy { places, .. } => places.len(),
        }
    }

    /// Rows `first` to before `end` (from 0), renumbered from 0; `None` when none is left.
    fn slice(&self, first: usize, end: usize) -> Option<Markdown> {
        let out = match self {
            Markdown::Given(map) => Markdown::Given(
                map.iter()
                    .filter(|((row, _), _)| (first..end).contains(row))
                    .map(|(&(row, column), text)| ((row - first, column), text.clone()))
                    .collect(),
            ),
            Markdown::Lazy {
                whole,
                first: offset,
                places,
                ..
            } => Markdown::Lazy {
                whole: Arc::clone(whole),
                first: offset + first,
                end: offset + end,
                places: places
                    .iter()
                    .filter(|(row, _)| (first..end).contains(row))
                    .map(|&(row, column)| (row - first, column))
                    .collect(),
            },
        };
        (out.len() > 0).then_some(out)
    }
}

/// What the producer keeps for itself (its own result object), which no other library
/// reads.
pub type Detail = Arc<dyn Any + Send + Sync>;

/// Columns, rows and the sources of each value: format-neutral, immutable. Compared by its
/// columns, rows, sources, keys and type labels; its detail, cache, formats and Markdown are
/// not compared.
#[derive(Clone)]
pub struct Dataset {
    columns: Vec<String>,
    rows: Vec<Vec<Cell>>,
    sources: Option<Vec<Vec<Vec<String>>>>,
    keys: usize,
    types: Option<Vec<Option<String>>>,
    detail: Option<Detail>,
    cache: Option<Cache>,
    formats: Option<Vec<Option<String>>>,
    markdown: Option<Markdown>,
}

impl fmt::Debug for Dataset {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Dataset")
            .field("columns", &self.columns)
            .field("rows", &self.rows)
            .field("sources", &self.sources)
            .field("keys", &self.keys)
            .field("types", &self.types)
            .field("cache", &self.cache)
            .field("formats", &self.formats)
            .finish_non_exhaustive()
    }
}

impl PartialEq for Dataset {
    fn eq(&self, other: &Self) -> bool {
        self.columns == other.columns
            && self.rows == other.rows
            && self.sources == other.sources
            && self.keys == other.keys
            && self.types == other.types
    }
}

/// A dataset's type labels, as [`Dataset::from_records`] takes them.
#[derive(Debug, Clone)]
pub enum Labels {
    /// A label by column name; the columns not named unlabelled.
    ByName(Vec<(String, String)>),
    /// A label per column, in column order (`None` for none).
    PerColumn(Vec<Option<String>>),
}

/// Records as Python code has them, as [`Dataset::from_records`] takes them.
#[derive(Debug, Clone)]
pub enum Records {
    /// Mappings by column name.
    Maps(Vec<Vec<(String, Cell)>>),
    /// Values in column order (they need the columns).
    Rows(Vec<Vec<Cell>>),
}

impl Dataset {
    /// A dataset of `columns` and `rows` (values in column order), no sources, no keys.
    ///
    /// # Errors
    /// No column, a column named by empty text, two of one name, a row of the wrong length.
    pub fn new(columns: Vec<String>, rows: Vec<Vec<Cell>>) -> Result<Dataset, DatasetError> {
        if columns.is_empty() || columns.iter().any(String::is_empty) {
            return bad("a dataset has at least one column, each named by a string");
        }
        let mut seen = std::collections::HashSet::new();
        if !columns.iter().all(|c| seen.insert(c)) {
            return bad("a dataset's column names are unique");
        }
        if let Some(row) = rows.iter().find(|row| row.len() != columns.len()) {
            return bad(format!("a row has {} values for {} columns", row.len(), columns.len()));
        }
        Ok(Dataset {
            columns,
            rows,
            sources: None,
            keys: 0,
            types: None,
            detail: None,
            cache: None,
            formats: None,
            markdown: None,
        })
    }

    /// With each value's sources: a list per row with a list per column.
    ///
    /// # Errors
    /// Not a row per row and a column per column.
    pub fn with_sources(mut self, sources: Vec<Vec<Vec<String>>>) -> Result<Dataset, DatasetError> {
        if sources.len() != self.rows.len() || sources.iter().any(|row| row.len() != self.columns.len()) {
            return bad("a dataset's sources have a row per row and a column per column");
        }
        self.sources = Some(sources);
        Ok(self)
    }

    /// With `keys` leading columns naming a row.
    ///
    /// # Errors
    /// More keys than columns.
    pub fn with_keys(mut self, keys: usize) -> Result<Dataset, DatasetError> {
        if keys > self.columns.len() {
            return bad(format!("keys is 0 to {} (the leading columns)", self.columns.len()));
        }
        self.keys = keys;
        Ok(self)
    }

    /// With a type label per column (`None` for none; [`TYPE_LABELS`] are the ones the
    /// format libraries read).
    ///
    /// # Errors
    /// Not a label per column.
    pub fn with_types(mut self, types: Vec<Option<String>>) -> Result<Dataset, DatasetError> {
        if types.len() != self.columns.len() {
            return bad("a dataset has a type label per column");
        }
        self.types = Some(types);
        Ok(self)
    }

    /// With the formula caches its values were read from ([`Cache::new`]).
    #[must_use]
    pub fn with_cache(mut self, cache: Option<Cache>) -> Dataset {
        self.cache = cache;
        self
    }

    /// With each column's number format as its source shows it (`None` for General or
    /// none).
    ///
    /// # Errors
    /// Not a format per column.
    pub fn with_formats(mut self, formats: Vec<Option<String>>) -> Result<Dataset, DatasetError> {
        if formats.len() != self.columns.len() {
            return bad("a dataset has a number format (a string, or None) per column");
        }
        self.formats = Some(formats);
        Ok(self)
    }

    /// With the Markdown of its rich text values; none kept when it has no place.
    ///
    /// # Errors
    /// A place outside the dataset.
    pub fn with_markdown(mut self, markdown: Markdown) -> Result<Dataset, DatasetError> {
        let (rows, columns) = (self.rows.len(), self.columns.len());
        for (row, column) in markdown.places() {
            if row >= rows || column >= columns {
                return bad(format!(
                    "a dataset's markdown names ({row}, {column}), outside its {rows} rows and {columns} columns (from 0)"
                ));
            }
        }
        self.markdown = (markdown.len() > 0).then_some(markdown);
        Ok(self)
    }

    /// With what the producer keeps for itself.
    #[must_use]
    pub fn with_detail(mut self, detail: Detail) -> Dataset {
        self.detail = Some(detail);
        self
    }

    /// A dataset of records, as Python code has them (`Dataset.from_records`): mappings,
    /// read by column name -- a name a mapping lacks is a blank there -- or rows in column
    /// order, which need `columns`. Without `columns`, every name the mappings use, in the
    /// order first met. A name in `columns` no mapping has is refused (a misspelling would
    /// otherwise be a blank column). `types`: the labels ([`TYPE_LABELS`]). No sources:
    /// its values came from the caller.
    ///
    /// # Errors
    /// An unknown label, a label for a column there is not, rows of the wrong length, rows
    /// without `columns`, no columns.
    pub fn from_records(
        records: Records,
        types: Option<Labels>,
        columns: Option<Vec<String>>,
        keys: usize,
    ) -> Result<Dataset, DatasetError> {
        let columns = match (columns, &records) {
            (Some(columns), _) => columns,
            (None, Records::Rows(rows)) if !rows.is_empty() => {
                return bad("rows that are not mappings need columns: Dataset.from_records(rows, columns=[...])")
            }
            (None, Records::Rows(_)) => {
                return bad("records without names have no columns: give columns");
            }
            (None, Records::Maps(maps)) => {
                let mut names: Vec<String> = Vec::new();
                for (name, _) in maps.iter().flatten() {
                    if !names.contains(name) {
                        names.push(name.clone());
                    }
                }
                if names.is_empty() {
                    return bad("records without names have no columns: give columns");
                }
                names
            }
        };
        if columns.iter().any(String::is_empty) {
            return bad("a dataset's columns are named by strings");
        }
        let rows = match records {
            Records::Rows(rows) => rows,
            Records::Maps(maps) => {
                if !maps.is_empty() {
                    if let Some(unused) = columns
                        .iter()
                        .find(|name| !maps.iter().any(|m| m.iter().any(|(n, _)| n == *name)))
                    {
                        let mut names: Vec<&String> = maps.iter().flatten().map(|(n, _)| n).collect();
                        names.sort();
                        names.dedup();
                        let shown: Vec<String> = names.iter().take(50).map(|n| repr_str(n)).collect();
                        return bad(format!(
                            "no record has {}; the records' names are [{}]",
                            repr_str(unused),
                            shown.join(", ")
                        ));
                    }
                }
                maps.iter()
                    .map(|m| {
                        columns
                            .iter()
                            .map(|name| {
                                m.iter()
                                    .rev()
                                    .find(|(n, _)| n == name)
                                    .map_or(Cell::Blank, |(_, v)| v.clone())
                            })
                            .collect()
                    })
                    .collect()
            }
        };
        let labels = labels(types, &columns)?;
        let mut dataset = Dataset::new(columns, rows)?.with_keys(keys)?;
        if let Some(labels) = labels {
            dataset = dataset.with_types(labels)?;
        }
        Ok(dataset)
    }

    /// The columns' names.
    #[must_use]
    pub fn columns(&self) -> &[String] {
        &self.columns
    }

    /// The values, row by row.
    #[must_use]
    pub fn rows(&self) -> &[Vec<Cell>] {
        &self.rows
    }

    /// Each value's sources, where the producer gave them.
    #[must_use]
    pub fn sources(&self) -> Option<&[Vec<Vec<String>>]> {
        self.sources.as_deref()
    }

    /// How many leading columns name a row.
    #[must_use]
    pub fn keys(&self) -> usize {
        self.keys
    }

    /// The type label per column, where it has them.
    #[must_use]
    pub fn types(&self) -> Option<&[Option<String>]> {
        self.types.as_deref()
    }

    /// What the producer keeps for itself.
    #[must_use]
    pub fn detail(&self) -> Option<&Detail> {
        self.detail.as_ref()
    }

    /// The formula caches its values were read from.
    #[must_use]
    pub fn cache(&self) -> Option<&Cache> {
        self.cache.as_ref()
    }

    /// Each column's number format, where the producer knows them.
    #[must_use]
    pub fn formats(&self) -> Option<&[Option<String>]> {
        self.formats.as_deref()
    }

    /// The Markdown of its rich text values.
    #[must_use]
    pub fn markdown(&self) -> Option<&Markdown> {
        self.markdown.as_ref()
    }

    /// Whether any value was read from a formula whose cached value is invalidated.
    #[must_use]
    pub fn invalidated(&self) -> bool {
        self.cache.as_ref().is_some_and(|c| c.count("invalidated") > 0)
    }

    /// Rows times columns: what the session's limit on a dataset counts.
    #[must_use]
    pub fn cells(&self) -> usize {
        self.rows.len() * self.columns.len()
    }

    /// The position of `column`; `not_found` with the columns.
    ///
    /// # Errors
    /// No such column.
    pub fn index(&self, column: &str) -> Result<usize, ToolError> {
        self.columns.iter().position(|c| c == column).ok_or_else(|| {
            ToolError::refusal(
                Refusal::NotFound,
                format!("no column {} in the dataset", repr_str(column)),
                WayOut::names(self.columns.iter().cloned()).unwrap_or_else(|| WayOut::one(Value::Null)),
            )
        })
    }

    /// The values of one column, row by row.
    ///
    /// # Errors
    /// No such column.
    pub fn column(&self, name: &str) -> Result<Vec<&Cell>, ToolError> {
        let at = self.index(name)?;
        Ok(self.rows.iter().filter_map(|row| row.get(at)).collect())
    }

    /// The Markdown of the value at `row` and `column` (both from 0) when its source holds
    /// more than its text, else `None`: the value is its text.
    #[must_use]
    pub fn markdown_at(&self, row: usize, column: usize) -> Option<String> {
        self.markdown.as_ref()?.get(row, column)
    }

    /// The rows as mappings by column name.
    #[must_use]
    pub fn records(&self) -> Vec<Vec<(&str, &Cell)>> {
        self.rows
            .iter()
            .map(|row| self.columns.iter().map(String::as_str).zip(row).collect())
            .collect()
    }

    /// Rows `first` to `last` (from 1, both included; `None`: to the last row), with their
    /// sources, keys, types, cache, formats and Markdown; no detail (the producer's, which
    /// describes every row). Nothing is cut.
    ///
    /// # Errors
    /// A row past the last, or `first` after `last`.
    pub fn row_slice(&self, first: usize, last: Option<usize>) -> Result<Dataset, DatasetError> {
        let count = self.rows.len();
        let end = last.unwrap_or(count);
        if !(1 <= first && first <= end) {
            return bad(format!("rows run from 1 to the last, first to last: {first} to {end}"));
        }
        if end > count {
            return bad(format!("the dataset has {count} rows; there is no row {end}"));
        }
        let rows = self.rows.get(first - 1..end).unwrap_or_default().to_vec();
        let sources = self
            .sources
            .as_ref()
            .map(|s| s.get(first - 1..end).unwrap_or_default().to_vec());
        Ok(Dataset {
            columns: self.columns.clone(),
            rows,
            sources,
            keys: self.keys,
            types: self.types.clone(),
            detail: None,
            cache: self.cache.clone(),
            formats: self.formats.clone(),
            markdown: self.markdown.as_ref().and_then(|m| m.slice(first - 1, end)),
        })
    }

    /// The rows a source hands a call (`Rows`): `given` as the call gave it, `address` as
    /// resolved, the document they came from and its version, whether stale; its values,
    /// type labels, sources and cache counts.
    #[must_use]
    pub fn source_rows(
        &self,
        given: &str,
        address: &str,
        doc: Option<&str>,
        name: Option<&str>,
        version: Option<i64>,
        stale: bool,
    ) -> SourceRows {
        SourceRows {
            given: given.to_owned(),
            address: address.to_owned(),
            columns: self.columns.clone(),
            types: self.types.clone(),
            rows: self.rows.clone(),
            sources: self.sources.clone(),
            doc: doc.map(str::to_owned),
            name: name.map(str::to_owned),
            version,
            stale,
            cache: self.cache.as_ref().map(Cache::to_json),
        }
    }
}

/// `types` as a label per column, each one of [`TYPE_LABELS`] or `None`.
fn labels(types: Option<Labels>, columns: &[String]) -> Result<Option<Vec<Option<String>>>, DatasetError> {
    let labels = match types {
        None => return Ok(None),
        Some(Labels::ByName(named)) => {
            if let Some((name, _)) = named.iter().find(|(name, _)| !columns.contains(name)) {
                let shown: Vec<String> = columns.iter().map(|c| repr_str(c)).collect();
                return bad(format!(
                    "types names {}, which is no column; the columns are [{}]",
                    repr_str(name),
                    shown.join(", ")
                ));
            }
            columns
                .iter()
                .map(|c| named.iter().rev().find(|(n, _)| n == c).map(|(_, l)| l.clone()))
                .collect()
        }
        Some(Labels::PerColumn(labels)) => {
            if labels.len() != columns.len() {
                return bad(format!(
                    "{} type labels for {} columns: give one per column (None for none), or a mapping by name",
                    labels.len(),
                    columns.len()
                ));
            }
            labels
        }
    };
    if let Some(label) = labels.iter().flatten().find(|l| !TYPE_LABELS.contains(&l.as_str())) {
        return bad(format!(
            "{} is not a type label: one of {} (or None)",
            repr_str(label),
            TYPE_LABELS.join(", ")
        ));
    }
    Ok(Some(labels))
}

#[cfg(test)]
mod tests;
