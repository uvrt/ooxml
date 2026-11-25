//! A toy host session over `TestFormat`'s documents, as ooxml-edit's dataset and source
//! tests use their toolbox: decks (`pptx`, pages with a title each, every page its own undo
//! scope), texts (`docx`, a list of items) and grids (`xlsx`, named tables of cells resolving
//! `t:<name>` to rows, a source per cell `G!B2`, runs of cells merged as `G!B2:B4`). Its
//! dispatcher runs a tool as ooxml-edit's does around a handler: the declared sources
//! qualified and held, the declared datasets checked, the handler, then the change applied
//! and the datasets kept -- or nothing kept when it failed.
//!
//! Every name and figure here is made up.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use crate::test_format::{Content, TestFormat};
use ooxml_session::datasets::{CallDatasets, DatasetLimits, DatasetStore, DocMeta};
use ooxml_session::sources::{CallSources, OpenDoc, SourceHost};
use ooxml_tools::datasets::{Cache, Dataset};
use ooxml_tools::sources::{Cell, Prefixes};
use ooxml_tools::{Refusal, ToolError, TryCall, WayOut};
use serde_json::{json, Map, Value};
use std::cell::{Cell as Counter, RefCell};
use std::collections::BTreeMap;

/// A grid's table: its header and rows.
pub type Table = (Vec<String>, Vec<Vec<Cell>>);

/// A toy grid: named tables, each one's formula caches, the reads made of it.
#[derive(Default)]
pub struct Grid {
    pub tables: BTreeMap<String, Table>,
    pub caches: BTreeMap<String, Vec<(&'static str, i64)>>,
    pub reads: Counter<usize>,
}

/// The made-up sales table.
pub fn sales() -> Table {
    let row = |region: &str, units: Cell, note: &str, year: i64| {
        vec![
            Cell::Text(region.into()),
            units,
            Cell::Text(note.into()),
            Cell::Int(year.to_string()),
        ]
    };
    (
        ["region", "units", "note", "year"].map(str::to_owned).to_vec(),
        vec![
            row("North", Cell::Int("3".into()), "ok", 2024),
            row("South", Cell::Int("5".into()), "late", 2025),
            row("East", Cell::Float(2.5), "", 2026),
        ],
    )
}

fn letter(column: usize) -> char {
    char::from(b'A' + u8::try_from(column).unwrap())
}

/// Each value's source in a grid: `G!B2` for the second column's first row.
pub fn grid_sources(columns: usize, rows: usize) -> Vec<Vec<Vec<String>>> {
    (0..rows)
        .map(|r| {
            (0..columns)
                .map(|c| vec![format!("G!{}{}", letter(c), r + 2)])
                .collect()
        })
        .collect()
}

/// The grid format's resolver (`_table`).
fn table(grid: &Grid, address: &str) -> Result<Dataset, ToolError> {
    let name = address.strip_prefix("t:").unwrap_or(address);
    let Some((header, rows)) = grid.tables.get(name) else {
        return Err(ToolError::refusal(
            Refusal::NotFound,
            format!("no table '{name}'"),
            WayOut::names(grid.tables.keys().map(|n| format!("t:{n}"))).unwrap_or_else(|| WayOut::one(Value::Null)),
        ));
    };
    grid.reads.set(grid.reads.get() + 1);
    let cache = grid
        .caches
        .get(name)
        .map(|c| Cache::new(c).unwrap())
        .unwrap_or_default();
    Ok(Dataset::new(header.clone(), rows.clone())
        .unwrap()
        .with_sources(grid_sources(header.len(), rows.len()))
        .unwrap()
        .with_keys(1)
        .unwrap()
        .with_cache(cache))
}

/// Runs of cells in one column, rows in a row, as ranges: `G!B2, G!B3` -> `G!B2:B3`.
fn merge(addresses: Vec<String>) -> Vec<String> {
    let mut out: Vec<(String, char, u32, u32)> = Vec::new();
    for address in addresses {
        let (sheet, cell) = address.split_once('!').unwrap();
        let column = cell.chars().next().unwrap();
        let row: u32 = cell[1..].parse().unwrap();
        match out.last_mut() {
            Some(last) if last.0 == sheet && last.1 == column && last.3 + 1 == row => last.3 = row,
            _ => out.push((sheet.to_owned(), column, row, row)),
        }
    }
    out.into_iter()
        .map(|(s, c, a, b)| {
            if a == b {
                format!("{s}!{c}{a}")
            } else {
                format!("{s}!{c}{a}:{c}{b}")
            }
        })
        .collect()
}

/// The deck format's hooks, where a test gives them.
#[derive(Default, Clone, Copy)]
pub struct DeckHooks {
    /// `source_hint`.
    pub hint: Option<fn(&str) -> Option<String>>,
    /// `own_source`.
    pub own: Option<fn(&str) -> Option<String>>,
}

/// The session: its documents and what each holds, its datasets.
pub struct Sess {
    pub format: TestFormat,
    pub docs: Vec<OpenDoc>,
    pub grids: BTreeMap<String, Grid>,
    pub store: DatasetStore,
    pub deck: DeckHooks,
    /// Whether the grid format lists its tables (`value_sources`).
    pub listing: bool,
    /// The deck's hint asked: what it was asked.
    pub asked: RefCell<Vec<String>>,
    prefixes: Prefixes,
    given: u64,
}

/// What the session clock says.
pub const NOW: &str = "2026-10-13T12:00:00+00:00";

impl Sess {
    pub fn new(limits: DatasetLimits) -> Sess {
        Sess {
            format: TestFormat::default(),
            docs: Vec::new(),
            grids: BTreeMap::new(),
            store: DatasetStore::new(limits),
            deck: DeckHooks::default(),
            listing: false,
            asked: RefCell::new(Vec::new()),
            prefixes: Prefixes::new([("xlsx", vec!["t:"]), ("pptx", vec![]), ("docx", vec![])]),
            given: 0,
        }
    }

    /// Open a document of `kind` named `name`: its handle.
    pub fn open(&mut self, kind: &str, name: &str) -> String {
        self.given += 1;
        let handle = format!("d{}", self.given);
        self.format.open(&handle);
        self.docs.push(OpenDoc {
            handle: handle.clone(),
            kind: kind.into(),
            name: name.into(),
        });
        handle
    }

    /// Open a grid of `tables`.
    pub fn adopt(&mut self, tables: &[(&str, Table)], name: &str) -> String {
        let handle = self.open("xlsx", name);
        let grid = Grid {
            tables: tables.iter().map(|(n, t)| ((*n).to_owned(), t.clone())).collect(),
            ..Grid::default()
        };
        self.grids.insert(handle.clone(), grid);
        handle
    }

    pub fn meta(&self, doc: &str) -> DocMeta {
        let found = self.docs.iter().find(|d| d.handle == doc).unwrap();
        DocMeta {
            doc: found.handle.clone(),
            kind: found.kind.clone(),
            name: found.name.clone(),
        }
    }

    /// Each open document's version now.
    pub fn head(&self, doc: &str) -> Option<u64> {
        self.docs
            .iter()
            .any(|d| d.handle == doc)
            .then(|| self.format.doc(doc).version)
    }

    /// Close `doc`: the datasets derived from it, which stay.
    pub fn close(&mut self, doc: &str) -> Vec<String> {
        let head = self.head(doc);
        self.docs.retain(|d| d.handle != doc);
        self.store.document_closed(doc, head)
    }

    pub fn version(&self, doc: &str) -> u64 {
        self.format.doc(doc).version
    }

    pub fn stale(&self, id: &str) -> bool {
        let heads = |doc: &str| self.head(doc);
        self.store.find(id).unwrap().stale(&heads)
    }

    pub fn describe(&self) -> Option<Value> {
        let heads = |doc: &str| self.head(doc);
        self.store.describe(&heads)
    }
}

impl SourceHost for Sess {
    fn documents(&self) -> Vec<OpenDoc> {
        self.docs.clone()
    }

    fn prefixes(&self) -> &Prefixes {
        &self.prefixes
    }

    fn resolve(&self, doc: &str, address: &str) -> Result<Dataset, ToolError> {
        table(&self.grids[doc], address)
    }

    fn value_sources(&self, doc: &str) -> Option<Vec<String>> {
        let grid = self.grids.get(doc).filter(|_| self.listing)?;
        Some(grid.tables.keys().map(|n| format!("t:{n}")).collect())
    }

    fn own_source(&self, doc: &str, address: &str) -> Option<String> {
        let deck = self.docs.iter().any(|d| d.handle == doc && d.kind == "pptx");
        let own = self.deck.own.filter(|_| deck)?;
        self.asked.borrow_mut().push(address.to_owned());
        own(address)
    }

    fn source_hint(&self, doc: &str, value: &str) -> Option<String> {
        let deck = self.docs.iter().any(|d| d.handle == doc && d.kind == "pptx");
        let hint = self.deck.hint.filter(|_| deck)?;
        self.asked.borrow_mut().push(value.to_owned());
        hint(value)
    }

    fn merge_sources(&self, kind: &str, sources: Vec<String>) -> Vec<String> {
        if kind == "xlsx" {
            merge(sources)
        } else {
            sources
        }
    }
}

/// What a toy handler is given.
pub struct Ctx<'a> {
    pub sess: &'a Sess,
    pub datasets: &'a mut CallDatasets,
    pub doc: Option<String>,
    pub arguments: &'a Map<String, Value>,
    pub held: Vec<String>,
    pub retry: TryCall,
    pub tool: &'static str,
    /// The change the handler made: its scope and the content after.
    pub change: Option<(Option<String>, Content)>,
}

impl Ctx<'_> {
    pub fn content(&self) -> Content {
        self.sess.format.doc(self.doc.as_deref().unwrap()).content().clone()
    }

    /// Run `read` with the call's sources.
    pub fn sources<T>(&mut self, read: impl FnOnce(&CallSources<'_>) -> T) -> T {
        let heads = |doc: &str| self.sess.head(doc);
        let sources = CallSources {
            host: self.sess,
            store: &self.sess.store,
            datasets: RefCell::new(&mut *self.datasets),
            heads: &heads,
            tool: self.tool,
            arguments: self.arguments,
            primary: self.doc.as_deref(),
            held: &self.held,
            retry: &self.retry,
            own: None,
        };
        read(&sources)
    }
}

/// A toy handler: its data, or its refusal.
pub type Handler = fn(&mut Ctx<'_>) -> Result<Value, ToolError>;

/// A toy tool: its name, the argument paths it declares as datasets and as sources, its
/// handler.
#[derive(Clone, Copy)]
pub struct Tool {
    pub name: &'static str,
    pub datasets: &'static [&'static str],
    pub sources: &'static [&'static str],
    pub run: Handler,
}

/// What a toy call answered.
#[derive(Debug, Default)]
pub struct Reply {
    pub error: Option<Value>,
    pub data: Value,
    pub warnings: Vec<String>,
    pub datasets: Vec<String>,
}

impl Reply {
    pub fn ok(&self) -> bool {
        self.error.is_none()
    }

    /// The refusal, asserted to have `code`.
    pub fn failed(&self, code: &str) -> &Value {
        let error = self
            .error
            .as_ref()
            .unwrap_or_else(|| panic!("not refused: {:?}", self.data));
        assert_eq!(error["code"], code, "{error}");
        error
    }
}

/// The string values at `path` (`from`, `items[].from`, `also[]`), with their fields.
fn values(arguments: &Map<String, Value>, path: &str) -> Vec<(String, String)> {
    let (head, rest) = path.split_once('.').map_or((path, None), |(h, r)| (h, Some(r)));
    let (key, each) = head.strip_suffix("[]").map_or((head, false), |k| (k, true));
    let Some(found) = arguments.get(key) else {
        return Vec::new();
    };
    let items: Vec<(String, &Value)> = if each {
        found
            .as_array()
            .map(|a| a.iter().enumerate().map(|(i, v)| (format!("{key}[{i}]"), v)).collect())
            .unwrap_or_default()
    } else {
        vec![(key.to_owned(), found)]
    };
    let mut out = Vec::new();
    for (field, value) in items {
        match rest {
            None => {
                if let Some(text) = value.as_str() {
                    out.push((field, text.to_owned()));
                }
            }
            Some(rest) => {
                if let Some(inner) = value.as_object() {
                    out.extend(
                        values(inner, rest)
                            .into_iter()
                            .map(|(f, v)| (format!("{field}.{f}"), v)),
                    );
                }
            }
        }
    }
    out
}

fn set_at(arguments: &mut Map<String, Value>, field: &str, value: &str) {
    // only top-level fields are qualified by these tests' tools
    arguments.insert(field.to_owned(), Value::from(value));
}

impl Sess {
    /// One op, as the dispatcher runs it: the declared sources qualified and held, the
    /// declared datasets checked, the handler, the change applied. Nothing is kept here.
    fn op(&mut self, datasets: &mut CallDatasets, tool: Tool, given: &Value) -> Result<Value, ToolError> {
        let mut arguments = given.as_object().cloned().unwrap_or_default();
        let doc = arguments.get("doc").and_then(Value::as_str).map(str::to_owned);
        let retry = TryCall {
            tool: tool.name.into(),
            arguments: arguments.clone(),
            fill: Vec::new(),
        };
        let mut held = Vec::new();
        {
            let heads = |d: &str| self.head(d);
            let sources = CallSources {
                host: &*self,
                store: &self.store,
                datasets: RefCell::new(&mut *datasets),
                heads: &heads,
                tool: tool.name,
                arguments: &arguments,
                primary: doc.as_deref(),
                held: &[],
                retry: &retry,
                own: None,
            };
            let mut qualified = Vec::new();
            for path in tool.sources {
                for (field, value) in values(&arguments, path) {
                    if value.starts_with("d:") {
                        sources.name_dataset(&value, &field, true)?;
                    } else if let Some((found, written)) = sources.qualify(&value, &field, false)? {
                        held.push(found);
                        qualified.push((field, written));
                    }
                }
            }
            for path in tool.datasets {
                for (field, value) in values(&arguments, path) {
                    if value.starts_with("d:") {
                        sources.name_dataset(&value, &field, false)?;
                    }
                }
            }
            for (field, written) in qualified {
                set_at(&mut arguments, &field, &written);
            }
        }
        let mut ctx = Ctx {
            sess: self,
            datasets,
            doc: doc.clone(),
            arguments: &arguments,
            held,
            retry,
            tool: tool.name,
            change: None,
        };
        let data = (tool.run)(&mut ctx)?;
        if let Some((scope, content)) = ctx.change.take() {
            let doc = doc.unwrap();
            ctx.datasets.changed(&doc);
            self.format
                .docs
                .get_mut(&doc)
                .unwrap()
                .change(scope.as_deref(), |c| *c = content);
        }
        Ok(data)
    }

    /// One call: kept when it succeeds, nothing when it fails.
    pub fn call(&mut self, tool: Tool, arguments: Value) -> Reply {
        let mut datasets = CallDatasets::new();
        let mut reply = Reply::default();
        match self.op(&mut datasets, tool, &arguments) {
            Ok(data) => {
                reply.datasets = datasets.addresses().to_vec();
                let heads = |doc: &str| {
                    self.docs
                        .iter()
                        .any(|d| d.handle == doc)
                        .then(|| self.format.doc(doc).version)
                };
                match datasets.finish(&mut self.store, &heads) {
                    Ok(warnings) => {
                        reply.warnings = warnings;
                        reply.data = data;
                    }
                    Err(error) => reply.error = Some(error.to_json()),
                }
            }
            Err(error) => reply.error = Some(error.to_json()),
        }
        reply
    }

    /// A batch of ops: atomic, all or nothing; else a failed op is skipped, what it defined
    /// forgotten.
    pub fn batch(&mut self, ops: &[(Tool, Value)], atomic: bool) -> Reply {
        let mut datasets = CallDatasets::new();
        let mut reply = Reply::default();
        let mut answers = Vec::new();
        let mut failed = Vec::new();
        for (i, (tool, arguments)) in ops.iter().enumerate() {
            datasets.start_op(i);
            let mark = datasets.mark();
            match self.op(&mut datasets, *tool, arguments) {
                Ok(data) => answers.push(json!({"data": data})),
                Err(error) => {
                    let mut error = error.to_json();
                    let field = error["field"].as_str().map(|f| format!("arguments.{f}"));
                    if atomic {
                        if let Some(field) = field {
                            error["field"] = Value::from(format!("ops[{i}].{field}"));
                        }
                        reply.error = Some(error);
                        return reply;
                    }
                    if let Some(field) = field {
                        error["field"] = Value::from(field);
                    }
                    datasets.reset(mark);
                    failed.push(i);
                    answers.push(json!({"error": error}));
                }
            }
        }
        reply.datasets = datasets.addresses().to_vec();
        let heads = |doc: &str| {
            self.docs
                .iter()
                .any(|d| d.handle == doc)
                .then(|| self.format.doc(doc).version)
        };
        match datasets.finish(&mut self.store, &heads) {
            Ok(warnings) => reply.warnings = warnings,
            Err(error) => reply.error = Some(error.to_json()),
        }
        reply.data = json!({"ops": answers, "failed": failed});
        reply
    }
}
