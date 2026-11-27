//! Owns: the session's datasets (design phase 3, section 5; ooxml-edit's `DatasetEntry`, the
//! session's store and the dispatcher's part, ported exactly): a dataset kept under its id
//! with where it came from ([`DatasetEntry`], [`Derivation`]: its source document and that
//! document's version, the other sources it also read); the store, in the order defined,
//! within its limits ([`DatasetStore`], [`DatasetLimits`]); what one call (or batch) defines
//! and names before it ends ([`CallDatasets`]), kept only when it succeeds
//! ([`CallDatasets::finish`]); a dataset or a run of its rows as a source's rows
//! ([`CallDatasets::source`], `d:x/#9:#16`); and staleness: a source's version against its
//! head now ([`Heads`]), frozen when its document closes, said on every use
//! ([`stale_warning`]). A kept entry as JSON, for the application's own host (`json`).
//!
//! A dataset is session state, not document content: undo never removes one, defining one
//! is not an undo step, and a reading call may define one. It is never refreshed: when its
//! source changes it is stale, and undoing that change makes it current again (equal
//! versions are equal content). A scoped undo leaves a state that never existed: stale.
//!
//! Must not: make a dataset (the formats' producers do), resolve another document's rows
//! (the host's [`crate::sources`] does), or lock: the host keeps the store behind its own.

use crate::DocKind;
use ooxml_tools::datasets::{
    check_dataset_id, dataset_id, row_range, rows_text, Dataset, BRIEF_COLUMNS, BRIEF_DATASETS, DATASET_PREFIX,
};
use ooxml_tools::sources::SourceRows;
use ooxml_tools::{Failure, Refusal, ToolError, TryCall, WayOut};
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::sync::Arc;

mod json;

/// The version each open document is at now: `None` for one that is not open.
pub trait Heads {
    /// `doc`'s state number now.
    fn head(&self, doc: &str) -> Option<u64>;
}

impl<F: Fn(&str) -> Option<u64>> Heads for F {
    fn head(&self, doc: &str) -> Option<u64> {
        self(doc)
    }
}

/// The session's limits on datasets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DatasetLimits {
    /// Datasets the session keeps at once; defining an id that exists replaces it.
    pub max_datasets: usize,
    /// Values (rows times columns) in one dataset; its sources are not counted.
    pub max_cells: usize,
}

impl Default for DatasetLimits {
    fn default() -> Self {
        Self {
            max_datasets: 50,
            max_cells: 50_000,
        }
    }
}

/// A document a dataset was made from, as the call that defines it names it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocMeta {
    /// Its handle (`d2`).
    pub doc: String,
    /// Its kind.
    pub kind: DocKind,
    /// Its name (`stock.xlsx`).
    pub name: String,
}

/// Where a dataset's values came from: a document at a version, or nothing (`doc` `None`:
/// never stale on its account); and the other sources it also read, each its own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Derivation {
    /// The dataset whose definition this is (a batch op's time is looked up by it).
    id: String,
    /// The source document's handle.
    pub doc: Option<String>,
    /// Its kind.
    pub kind: Option<DocKind>,
    /// Its name.
    pub name: Option<String>,
    /// Its version when the dataset was defined (`None` until the defining call ends).
    pub version: Option<u64>,
    /// Whether the source was closed, after which it no longer changes.
    closed: bool,
    /// Whether it is stale for good: closed while stale, or born stale.
    for_good: bool,
    /// The invalidated formula caches its values were read from.
    invalidated: u64,
    /// The other sources it read: it is stale when any of them is.
    pub also: Vec<Derivation>,
}

impl Derivation {
    fn new(id: &str, of: Option<&DocMeta>, invalidated: u64) -> Derivation {
        Derivation {
            id: id.to_owned(),
            doc: of.map(|m| m.doc.clone()),
            kind: of.map(|m| m.kind),
            name: of.map(|m| m.name.clone()),
            version: None,
            closed: false,
            for_good: false,
            invalidated,
            also: Vec::new(),
        }
    }

    /// Whether the source document was closed (one without a document never is).
    #[must_use]
    pub fn closed(&self) -> bool {
        self.doc.is_some() && self.closed
    }

    /// The source's version now; `None` once closed (or with no source).
    #[must_use]
    pub fn source_version(&self, heads: &dyn Heads) -> Option<u64> {
        match &self.doc {
            Some(doc) if !self.closed => heads.head(doc),
            _ => None,
        }
    }

    /// Whether its own source makes it stale (not one of [`Derivation::also`]), its source
    /// at `head`.
    fn own_stale_at(&self, head: Option<u64>) -> bool {
        self.for_good
            || self.invalidated > 0
            || (self.doc.is_some() && !self.closed && head.is_some() && head != self.version)
    }

    /// Whether its own source makes it stale now.
    #[must_use]
    pub fn own_stale(&self, heads: &dyn Heads) -> bool {
        self.own_stale_at(self.source_version(heads))
    }

    /// Whether the source has changed since -- now, not as of any earlier look -- or any
    /// other it read has, or the values came from invalidated formula caches.
    #[must_use]
    pub fn stale(&self, heads: &dyn Heads) -> bool {
        self.own_stale(heads) || self.also.iter().any(|other| other.stale(heads))
    }

    /// Stale for good: the state it was derived from never had a version.
    pub fn mark_stale(&mut self) {
        self.for_good = true;
    }

    /// The session closed `doc`, which was at `head`: every source of this that is that
    /// document -- its own, or one it also read, at any depth -- keeps its staleness as it
    /// is now, for good.
    fn document_closed(&mut self, doc: &str, head: Option<u64>) {
        if self.doc.as_deref() == Some(doc) && !self.closed {
            self.for_good = self.own_stale_at(head);
            self.closed = true;
        }
        for other in &mut self.also {
            other.document_closed(doc, head);
        }
    }

    /// The same source, its definition `id`'s.
    fn as_of(&self, id: &str, invalidated: u64) -> Derivation {
        Derivation {
            id: id.to_owned(),
            invalidated,
            ..self.clone()
        }
    }
}

/// A dataset in a session: the [`Dataset`], its id, where it came from.
#[derive(Debug, Clone, PartialEq)]
pub struct DatasetEntry {
    /// Its id (`by_region`).
    pub id: String,
    /// The values.
    pub dataset: Arc<Dataset>,
    /// The address in its source it was derived from (`t:sales`), or the dataset
    /// (`d:by_region`); `None` when the producer did not say.
    pub origin: Option<String>,
    /// The session clock's time of the call that defined it; `None` for one the
    /// application kept itself.
    pub defined_at: Option<String>,
    /// Its source.
    pub source: Derivation,
}

impl DatasetEntry {
    /// A dataset made from `of` (a document; `None`: none, never stale on its account).
    #[must_use]
    pub fn new(id: &str, dataset: Arc<Dataset>, of: Option<&DocMeta>) -> DatasetEntry {
        let invalidated = dataset.cache().map_or(0, |c| c.count("invalidated"));
        DatasetEntry {
            id: id.to_owned(),
            source: Derivation::new(id, of, invalidated),
            dataset,
            origin: None,
            defined_at: None,
        }
    }

    /// Its address: `d:` and its id.
    #[must_use]
    pub fn address(&self) -> String {
        format!("{DATASET_PREFIX}{}", self.id)
    }

    /// The source document's handle.
    #[must_use]
    pub fn doc(&self) -> Option<&str> {
        self.source.doc.as_deref()
    }

    /// Its source's version when it was defined.
    #[must_use]
    pub fn version(&self) -> Option<u64> {
        self.source.version
    }

    /// Whether the source document was closed.
    #[must_use]
    pub fn closed(&self) -> bool {
        self.source.closed()
    }

    /// Whether it is stale now ([`Derivation::stale`]).
    #[must_use]
    pub fn stale(&self, heads: &dyn Heads) -> bool {
        self.source.stale(heads)
    }

    /// Stale for good.
    pub fn mark_stale(&mut self) {
        self.source.mark_stale();
    }

    /// A dataset derived from this one: the same source, version and fate; its origin this
    /// dataset.
    #[must_use]
    pub fn derived(&self, id: &str, dataset: Arc<Dataset>) -> DatasetEntry {
        let invalidated = dataset.cache().map_or(0, |c| c.count("invalidated"));
        DatasetEntry {
            id: id.to_owned(),
            source: self.source.as_of(id, invalidated),
            dataset,
            origin: Some(self.address()),
            defined_at: None,
        }
    }

    /// What `describe` lists of it: id, columns (the first [`BRIEF_COLUMNS`], the rest
    /// counted), the type labels it has, row count, and `from`: `d3/t:sales` (a document's
    /// address), `d:by_region` (a dataset), `d3` (a document, no address said) or
    /// `application` (kept by the application, not by a call). `stale` (as given) and
    /// `closed` only when true. Never its values.
    #[must_use]
    pub fn brief(&self, stale: bool) -> Map<String, Value> {
        let columns = self.dataset.columns();
        let shown: Vec<&String> = columns.iter().take(BRIEF_COLUMNS).collect();
        let mut out = Map::new();
        out.insert("id".into(), Value::from(self.address()));
        out.insert(
            "columns".into(),
            shown.iter().map(|c| Value::from(c.as_str())).collect(),
        );
        if columns.len() > BRIEF_COLUMNS {
            out.insert("more_columns".into(), Value::from(columns.len() - BRIEF_COLUMNS));
        }
        let types: Map<String, Value> = shown
            .iter()
            .zip(self.dataset.types().unwrap_or_default())
            .filter_map(|(name, label)| Some(((*name).clone(), Value::from(label.as_deref()?))))
            .collect();
        if !types.is_empty() {
            out.insert("types".into(), Value::Object(types));
        }
        out.insert("rows".into(), Value::from(self.dataset.rows().len()));
        let doc = self.doc();
        let from = match (&self.origin, doc) {
            (Some(origin), Some(doc)) if !origin.starts_with(DATASET_PREFIX) => Some(format!("{doc}/{origin}")),
            (Some(origin), _) => Some(origin.clone()),
            (None, Some(doc)) => Some(doc.to_owned()),
            (None, None) => self.defined_at.is_none().then(|| "application".to_owned()),
        };
        if let Some(from) = from {
            out.insert("from".into(), Value::from(from));
        }
        if stale {
            out.insert("stale".into(), Value::Bool(true));
        }
        if self.closed() {
            out.insert("closed".into(), Value::Bool(true));
        }
        out
    }

    /// What a result says about it: never its values.
    #[must_use]
    pub fn describe(&self, heads: &dyn Heads) -> Map<String, Value> {
        let mut out = Map::new();
        out.insert("id".into(), Value::from(self.address()));
        out.insert(
            "columns".into(),
            self.dataset.columns().iter().map(|c| Value::from(c.as_str())).collect(),
        );
        out.insert("rows".into(), Value::from(self.dataset.rows().len()));
        out.insert("doc".into(), Value::from(self.doc()));
        out.insert("version".into(), Value::from(self.version()));
        out.insert("stale".into(), Value::Bool(self.stale(heads)));
        if self.closed() {
            out.insert("closed".into(), Value::Bool(true));
        }
        out
    }
}

/// Why `source` is stale (`_stale_reason`).
fn stale_reason(source: &Derivation, heads: &dyn Heads) -> String {
    if !source.own_stale(heads) {
        if let Some(other) = source.also.iter().find(|o| o.stale(heads)) {
            return stale_reason(other, heads);
        }
    }
    let doc = source.doc.as_deref().unwrap_or("None");
    if source.closed() {
        return format!("{doc} changed after it was derived, before it was closed");
    }
    if let (Some(version), Some(now)) = (source.version, source.source_version(heads)) {
        if version != now {
            return format!(
                "{doc} has changed since it was derived (version {version}, now {now}); derive it again for current values"
            );
        }
    }
    if source.invalidated > 0 {
        return format!(
            "it was computed from formula values whose cache is invalidated ({} cell(s)): they are as last saved, and \
             nothing here recalculates; recalculate the source in its application, then derive it again",
            source.invalidated
        );
    }
    format!("{doc} was changed after it was derived in the same batch; derive it again for current values")
}

/// The warning a stale dataset gets on every use (`_stale_warning`).
#[must_use]
pub fn stale_warning(entry: &DatasetEntry, heads: &dyn Heads) -> String {
    let address = entry.address();
    if !entry.source.own_stale(heads) {
        if let Some(other) = entry.source.also.iter().find(|o| o.stale(heads)) {
            return format!(
                "{address} is stale: {} (a source it also read)",
                stale_reason(other, heads)
            );
        }
    }
    format!("{address} is stale: {}", stale_reason(&entry.source, heads))
}

/// A way out naming `options`, else the call as given.
fn options_or(options: Vec<String>, retry: &TryCall) -> WayOut {
    WayOut::names(options).unwrap_or_else(|| WayOut::call(retry.clone()))
}

/// The `limit` of a session keeping `count` datasets already.
fn full(count: usize, limits: &DatasetLimits, known: Vec<String>) -> ToolError {
    let error = ToolError::failure(
        Failure::Limit,
        format!(
            "{count} datasets are kept; at most {}: reuse an id to replace one",
            limits.max_datasets
        ),
    )
    .with_limit(limits.max_datasets, count);
    match WayOut::names(known) {
        Some(way_out) => error.with_way_out(way_out),
        None => error,
    }
}

/// The session's datasets, in the order defined (a redefined id goes last).
#[derive(Debug, Clone, Default)]
pub struct DatasetStore {
    entries: Vec<DatasetEntry>,
    /// Each entry's serial, in step with `entries`: a number no other keeping was given,
    /// so a copy read of an entry is known for that keeping's alone ([`DatasetStore::serial`]).
    serials: Vec<u64>,
    /// The serial the next entry kept is given.
    next: u64,
    /// Its limits.
    pub limits: DatasetLimits,
}

impl DatasetStore {
    /// An empty store within `limits`.
    #[must_use]
    pub fn new(limits: DatasetLimits) -> DatasetStore {
        DatasetStore {
            entries: Vec::new(),
            serials: Vec::new(),
            next: 1,
            limits,
        }
    }

    /// Every dataset, in the order defined.
    #[must_use]
    pub fn entries(&self) -> &[DatasetEntry] {
        &self.entries
    }

    /// Every dataset's address, `d:` and its id, in the order defined.
    #[must_use]
    pub fn ids(&self) -> Vec<String> {
        self.entries.iter().map(DatasetEntry::address).collect()
    }

    /// The dataset `id` is (no `d:`), if there is one.
    #[must_use]
    pub fn find(&self, id: &str) -> Option<&DatasetEntry> {
        self.entries.iter().find(|e| e.id == id)
    }

    /// The dataset `id` is, to change (the application's housekeeping): its values kept
    /// anew, under a new serial.
    pub fn find_mut(&mut self, id: &str) -> Option<&mut DatasetEntry> {
        let at = self.entries.iter().position(|e| e.id == id)?;
        *self.serials.get_mut(at)? = self.next;
        self.next += 1;
        self.entries.get_mut(at)
    }

    /// The serial of the dataset `id` is (no `d:`): given when it was kept, never given
    /// again -- a dataset kept anew under its id, or dropped and kept again, has another.
    #[must_use]
    pub fn serial(&self, id: &str) -> Option<u64> {
        let at = self.entries.iter().position(|e| e.id == id)?;
        self.serials.get(at).copied()
    }

    /// The dataset `address` names (`d:by_region`, `d:by_region[total]`, or the bare id);
    /// `not_found` lists the ids there are.
    ///
    /// # Errors
    /// No such dataset.
    pub fn get(&self, address: &str, field: Option<&str>, retry: &TryCall) -> Result<&DatasetEntry, ToolError> {
        let id = if address.starts_with(DATASET_PREFIX) {
            dataset_id(address).unwrap_or(address)
        } else {
            address
        };
        self.find(id).ok_or_else(|| {
            let error = ToolError::refusal(
                Refusal::NotFound,
                format!("no dataset {DATASET_PREFIX}{id}"),
                options_or(self.ids(), retry),
            );
            match field {
                Some(field) => error.at(field),
                None => error,
            }
        })
    }

    /// Keep the datasets a call defined: all, or none and `limit`. An id that exists is
    /// replaced (and goes last); a new one needs room. A source closed while the call ran
    /// keeps its staleness for good; a source given no version (an application's own
    /// dataset, kept as a call defines one) is of its document's head now.
    ///
    /// # Errors
    /// `limit`: the store would hold more than its limit.
    pub fn keep(&mut self, entries: Vec<DatasetEntry>, heads: &dyn Heads) -> Result<(), ToolError> {
        if entries.is_empty() {
            return Ok(());
        }
        let mut new: Vec<&str> = entries
            .iter()
            .map(|e| e.id.as_str())
            .filter(|id| self.find(id).is_none())
            .collect();
        new.sort_unstable();
        new.dedup();
        if self.entries.len() + new.len() > self.limits.max_datasets {
            return Err(full(self.entries.len(), &self.limits, self.ids()));
        }
        for mut entry in entries {
            let docs: Vec<String> = std::iter::once(&entry.source)
                .chain(&entry.source.also)
                .filter_map(|s| s.doc.clone())
                .collect();
            for doc in docs {
                if heads.head(&doc).is_none() {
                    entry.source.document_closed(&doc, None); // closed while the call ran
                }
            }
            fill(&mut entry.source, false, heads);
            for other in &mut entry.source.also {
                fill(other, false, heads);
            }
            self.forget(&entry.id);
            self.entries.push(entry);
            self.serials.push(self.next);
            self.next += 1;
        }
        Ok(())
    }

    /// The session closes `doc`, now at `head`: the addresses of the datasets derived from
    /// it, which stay, each as stale or current as it is now, for good (and so does every
    /// dataset that also read it).
    pub fn document_closed(&mut self, doc: &str, head: Option<u64>) -> Vec<String> {
        let mut kept = Vec::new();
        for entry in &mut self.entries {
            if entry.doc() == Some(doc) && !entry.closed() {
                kept.push(entry.address());
            }
            entry.source.document_closed(doc, head);
        }
        kept
    }

    /// Forget a dataset (the application's housekeeping; no tool does this).
    ///
    /// # Errors
    /// No such dataset.
    pub fn drop_dataset(&mut self, address: &str, retry: &TryCall) -> Result<(), ToolError> {
        let id = self.get(address, None, retry)?.id.clone();
        self.forget(&id);
        Ok(())
    }

    /// The entry `id` and its serial taken out, where there is one.
    fn forget(&mut self, id: &str) {
        if let Some(at) = self.entries.iter().position(|e| e.id == id) {
            self.entries.remove(at);
            self.serials.remove(at);
        }
    }

    /// What the session's `describe` says of its datasets: each one's [`DatasetEntry::
    /// describe`]; `None` when there are none.
    #[must_use]
    pub fn describe(&self, heads: &dyn Heads) -> Option<Value> {
        (!self.entries.is_empty()).then(|| self.entries.iter().map(|e| Value::Object(e.describe(heads))).collect())
    }

    /// `describe`'s `datasets` (and `more_datasets`, the earlier ones not listed): the
    /// latest [`BRIEF_DATASETS`], each [`DatasetEntry::brief`]. Empty when there are none.
    #[must_use]
    pub fn briefs(&self, heads: &dyn Heads) -> Map<String, Value> {
        let mut out = Map::new();
        if self.entries.is_empty() {
            return out;
        }
        let skip = self.entries.len().saturating_sub(BRIEF_DATASETS);
        out.insert(
            "datasets".into(),
            self.entries
                .iter()
                .skip(skip)
                .map(|e| Value::Object(e.brief(e.stale(heads))))
                .collect(),
        );
        if skip > 0 {
            out.insert("more_datasets".into(), Value::from(skip));
        }
        out
    }
}

/// What a dataset a call defines was derived from.
#[derive(Debug, Clone, Copy)]
pub enum Of<'a> {
    /// A document (the call's own, by default).
    Doc(&'a DocMeta),
    /// A dataset: it inherits that one's source, version and fate.
    Dataset(&'a DatasetEntry),
    /// Nothing: never stale on its source's account.
    Nothing,
}

/// Another source a dataset also read (a join's other table or dataset).
#[derive(Debug, Clone, Copy)]
pub enum Also<'a> {
    /// A document, its version taken when the call ends.
    Doc(&'a DocMeta),
    /// A dataset, with its inherited source.
    Dataset(&'a DatasetEntry),
}

/// What [`CallDatasets::mark`] saves: a batch op's state before it ran.
#[derive(Debug, Clone)]
pub struct DatasetsMark {
    defined: Vec<DatasetEntry>,
    ops: HashMap<String, usize>,
    used: Vec<DatasetEntry>,
    addresses: Vec<String>,
}

/// What one call (or batch) defines and names, before it ends: kept by
/// [`CallDatasets::finish`] when it succeeds, dropped when it fails.
#[derive(Debug, Clone, Default)]
pub struct CallDatasets {
    defined: Vec<DatasetEntry>,
    ops: HashMap<String, usize>,
    used: Vec<DatasetEntry>,
    addresses: Vec<String>,
    op: usize,
    changed_at: HashMap<String, usize>,
}

impl CallDatasets {
    /// Nothing defined yet.
    #[must_use]
    pub fn new() -> CallDatasets {
        CallDatasets::default()
    }

    /// The batch's op `op` (from 0) runs next.
    pub fn start_op(&mut self, op: usize) {
        self.op = op;
    }

    /// The op running changed `doc`.
    pub fn changed(&mut self, doc: &str) {
        self.changed_at.insert(doc.to_owned(), self.op);
    }

    /// The state before an op, for [`CallDatasets::reset`] when it fails.
    #[must_use]
    pub fn mark(&self) -> DatasetsMark {
        DatasetsMark {
            defined: self.defined.clone(),
            ops: self.ops.clone(),
            used: self.used.clone(),
            addresses: self.addresses.clone(),
        }
    }

    /// Forget what a failed op defined and named.
    pub fn reset(&mut self, mark: DatasetsMark) {
        self.defined = mark.defined;
        self.ops = mark.ops;
        self.used = mark.used;
        self.addresses = mark.addresses;
    }

    /// The addresses the call defined, in order: a result's `datasets`.
    #[must_use]
    pub fn addresses(&self) -> &[String] {
        &self.addresses
    }

    /// The ids the call defined so far, in order.
    #[must_use]
    pub fn ids(&self) -> Vec<String> {
        self.defined.iter().map(|e| e.id.clone()).collect()
    }

    /// Keep `dataset` as `d:<id>` once the call succeeds; its address. `of`: what it was
    /// derived from (its version taken when the call ends, so a changing call defines it
    /// from the state it leaves); `origin`: the address it was derived from (`t:sales`), a
    /// dataset's own address when derived from one unless given; `also`: the other
    /// sources it read. An existing id is replaced. `defined_at`: the session clock's time.
    ///
    /// # Errors
    /// `invalid_arguments` for an id of the wrong shape (at `field`); `limit` over the
    /// cells a dataset may hold, or when the session would keep over its limit.
    #[allow(clippy::too_many_arguments)]
    pub fn define(
        &mut self,
        id: &str,
        dataset: Arc<Dataset>,
        of: Of<'_>,
        origin: Option<&str>,
        also: &[Also<'_>],
        store: &DatasetStore,
        defined_at: &str,
        field: &str,
        retry: &TryCall,
    ) -> Result<String, ToolError> {
        let id = check_dataset_id(id, field, retry)?.to_owned();
        let limits = &store.limits;
        if dataset.cells() > limits.max_cells {
            return Err(ToolError::failure(
                Failure::Limit,
                format!(
                    "the dataset has {} values ({} rows of {} columns); at most {}: group, filter or take the top rows",
                    dataset.cells(),
                    dataset.rows().len(),
                    dataset.columns().len(),
                    limits.max_cells
                ),
            )
            .with_limit(limits.max_cells, dataset.cells()));
        }
        let mut known: Vec<String> = store
            .ids()
            .into_iter()
            .chain(self.defined.iter().map(DatasetEntry::address))
            .collect();
        known.sort();
        known.dedup();
        let address = format!("{DATASET_PREFIX}{id}");
        if !known.contains(&address) && known.len() >= limits.max_datasets {
            return Err(full(known.len(), limits, known));
        }
        let invalidated = dataset.cache().map_or(0, |c| c.count("invalidated"));
        let mut entry = match of {
            Of::Dataset(parent) => parent.derived(&id, Arc::clone(&dataset)),
            Of::Doc(meta) => DatasetEntry::new(&id, Arc::clone(&dataset), Some(meta)),
            Of::Nothing => DatasetEntry::new(&id, Arc::clone(&dataset), None),
        };
        if let Some(origin) = origin {
            entry.origin = Some(origin.to_owned());
        }
        entry.defined_at = Some(defined_at.to_owned());
        for other in also {
            entry.source.also.push(match other {
                Also::Dataset(found) => found.source.as_of(&found.id, found.source.invalidated),
                Also::Doc(meta) => Derivation::new(&id, Some(meta), invalidated),
            });
        }
        self.defined.retain(|e| e.id != id);
        self.defined.push(entry);
        self.ops.insert(id, self.op);
        if !self.addresses.contains(&address) {
            self.addresses.push(address.clone());
        }
        Ok(address)
    }

    /// The dataset `address` names (`d:by_region`, with or without what follows the id):
    /// one this call (or an earlier op of its batch) defined, else the session's;
    /// `not_found` lists the ids there are.
    ///
    /// # Errors
    /// No such dataset.
    pub fn dataset(
        &self,
        address: &str,
        field: Option<&str>,
        store: &DatasetStore,
        retry: &TryCall,
    ) -> Result<DatasetEntry, ToolError> {
        let id = if address.starts_with(DATASET_PREFIX) {
            dataset_id(address).unwrap_or(address)
        } else {
            address
        };
        if let Some(pending) = self.defined.iter().find(|e| e.id == id) {
            return Ok(pending.clone());
        }
        store.get(address, field, retry).cloned().map_err(|error| {
            let mut options = store.ids();
            for pending in &self.defined {
                let address = pending.address();
                if !options.contains(&address) {
                    options.push(address);
                }
            }
            let rebuilt = ToolError::refusal(Refusal::NotFound, error.message(), options_or(options, retry));
            match error.field() {
                Some(field) => rebuilt.at(field),
                None => rebuilt,
            }
        })
    }

    /// The dataset a declared argument names, checked before the handler runs, and noted
    /// as named by the call (its staleness warned of when the call ends).
    ///
    /// # Errors
    /// No such dataset.
    pub fn name(
        &mut self,
        address: &str,
        field: &str,
        store: &DatasetStore,
        retry: &TryCall,
    ) -> Result<DatasetEntry, ToolError> {
        let entry = self.dataset(address, Some(field), store, retry)?;
        self.note_used(&entry);
        Ok(entry)
    }

    fn note_used(&mut self, entry: &DatasetEntry) {
        if !self.used.iter().any(|e| e.id == entry.id) {
            self.used.push(entry.clone());
        }
    }

    /// Whether a later op of the batch changed the source `source` names.
    fn changed_since(&self, source: &Derivation) -> bool {
        let Some(doc) = &source.doc else { return false };
        let at = self.ops.get(&source.id).copied().unwrap_or(self.op);
        self.changed_at.get(doc).is_some_and(|changed| *changed > at)
    }

    /// Whether `source`, of a dataset this call (or batch) defined, is stale so far: before
    /// the call ends it has no version to compare.
    fn pending_stale(&self, source: &Derivation, heads: &dyn Heads) -> bool {
        let own = if source.version.is_some() {
            source.stale(heads)
        } else {
            source.invalidated > 0 || self.changed_since(source)
        };
        own || source.also.iter().any(|other| self.pending_stale(other, heads))
    }

    /// Whether `entry` is stale as the call sees it: one it defined so far by its ops,
    /// the session's now.
    #[must_use]
    pub fn is_stale(&self, entry: &DatasetEntry, heads: &dyn Heads) -> bool {
        if self.defined.iter().any(|e| e.id == entry.id) {
            self.pending_stale(&entry.source, heads)
        } else {
            entry.stale(heads)
        }
    }

    /// The rows a dataset source names: a whole dataset (`d:by_region`) or a run of its rows
    /// (`d:by_region/#9:#16`, `/#9:` to the last, `/#9`), each value with its own sources.
    /// A row past the last is refused, never cut.
    ///
    /// # Errors
    /// No such dataset; another part of one; a row past its last.
    pub fn source(
        &mut self,
        value: &str,
        field: &str,
        store: &DatasetStore,
        heads: &dyn Heads,
        retry: &TryCall,
    ) -> Result<SourceRows, ToolError> {
        let entry = self.dataset(value, Some(field), store, retry)?;
        let whole = entry.address();
        let mut address = whole.clone();
        let mut part = None;
        if value != whole {
            let Some((_, first, last)) = row_range(value) else {
                return Err(ToolError::refusal(
                    Refusal::InvalidArguments,
                    format!(
                        "a source is a whole dataset ({whole}) or a run of its rows ({whole}/#1:#9, /#10: to the last), not \
                         another part of one"
                    ),
                    options_or(vec![whole.clone(), format!("{whole}/#1:")], retry),
                )
                .at(field));
            };
            let count = entry.dataset.rows().len();
            let past = first.max(last.unwrap_or(first));
            if past > count {
                let options = if count > 0 {
                    vec![format!("{whole}{}", rows_text(first.min(count), Some(count)))]
                } else {
                    Vec::new()
                };
                return Err(ToolError::refusal(
                    Refusal::InvalidArguments,
                    format!("{whole} has {count} rows; there is no row #{past} (nothing is cut)"),
                    options_or(options, retry),
                )
                .at(field));
            }
            part = Some(entry.dataset.row_slice(first, last).map_err(|e| {
                ToolError::refusal(Refusal::InvalidArguments, e.0, WayOut::call(retry.clone())).at(field)
            })?);
            address = format!("{whole}{}", rows_text(first, last));
        }
        self.note_used(&entry);
        let stale = self.is_stale(&entry, heads);
        let version = entry.version().and_then(|v| i64::try_from(v).ok());
        let rows = part.as_ref().unwrap_or(&entry.dataset);
        Ok(rows.source_rows(
            value,
            &address,
            entry.doc(),
            entry.source.name.as_deref(),
            version,
            stale,
        ))
    }

    /// The kind of the document a dataset of the call came from (`d:x...`), where it says.
    #[must_use]
    pub fn kind_of(&self, address: &str, store: &DatasetStore) -> Option<DocKind> {
        let id = dataset_id(address)?;
        let entry = self.defined.iter().find(|e| e.id == id).or_else(|| store.find(id))?;
        entry.source.kind
    }

    /// The call succeeded: keep what it defined, each with its source's version now (a
    /// dataset whose source a later op of the batch changed is stale for good: the state
    /// it came from had no version of its own); the warnings of the stale ones it defined
    /// or named, in the order named.
    ///
    /// # Errors
    /// `limit`: the session would keep over its limit (nothing is kept).
    pub fn finish(mut self, store: &mut DatasetStore, heads: &dyn Heads) -> Result<Vec<String>, ToolError> {
        let defined = std::mem::take(&mut self.defined);
        let mut kept = Vec::with_capacity(defined.len());
        for mut entry in defined {
            let later = self.changed_since(&entry.source);
            fill(&mut entry.source, later, heads);
            let changed: Vec<bool> = entry.source.also.iter().map(|o| self.changed_since(o)).collect();
            for (other, later) in entry.source.also.iter_mut().zip(changed) {
                fill(other, later, heads);
            }
            kept.push(entry);
        }
        store.keep(kept.clone(), heads)?;
        let mut named: Vec<DatasetEntry> = Vec::new();
        for entry in self.used.iter().chain(&kept) {
            match named.iter_mut().find(|e| e.id == entry.id) {
                Some(found) => *found = entry.clone(),
                None => named.push(entry.clone()),
            }
        }
        Ok(named
            .iter()
            .map(|e| store.find(&e.id).unwrap_or(e))
            .filter(|e| e.stale(heads))
            .map(|e| stale_warning(e, heads))
            .collect())
    }
}

/// A source of a dataset the call defined given its version as the call ends (`later`: a
/// later op of the batch changed it first).
fn fill(source: &mut Derivation, later: bool, heads: &dyn Heads) {
    if source.version.is_none() && source.doc.is_some() && !source.closed {
        if later {
            source.mark_stale();
        }
        source.version = source.source_version(heads);
    }
}
