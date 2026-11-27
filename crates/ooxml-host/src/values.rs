//! Owns: steps 6 to 8 of a call (design phase 3, sections 5 and 6; ooxml-edit's dispatcher
//! `_sources`, `_with_datasets`, `_with_facts` and docx-agent's adapters' host part, ported):
//! every address in a declared source qualified with its document (`t:sales` ->
//! `d2/t:sales`) and that document held for reading, a refusal of one that names no rows
//! refined (round 56: [`CallSources::refine`]); every dataset a declared argument names
//! checked; every reference in a declared text replaced by the value it names
//! ([`expand_arguments`]); and what the call's format reads of its sources, handed to it as
//! values ([`CallValues`]): the rows each declared source names, or the host's refusal of an
//! unknown dataset, keyed by the value as given; and, for a source naming one value, its
//! text and record as a value written ([`Written`]) and where it is in the rows it reads
//! ([`OneValue`]), or its refusal. At the call's end the datasets it named are kept
//! ([`CallDatasets::finish`], [`HostSession::finish_datasets`]).
//!
//! Must not: know a format's rules for what it reads (which rows a Word table takes, a
//! value that is not text refused as Markdown: the format's), or hold the stores' lock
//! while a document's rows are read.

use crate::catalogue::Entry;
use crate::session::HostSession;
use ooxml_session::datasets::{CallDatasets, DatasetStore};
use ooxml_session::facts::{expand_arguments, known_prefixes, one_value, qualify_text, walk_texts, FactEnv};
use ooxml_session::sources::{offered, CallSources, FormatOpen, FormatSources, SourceHost};
use ooxml_session::{AnyVersion, DocId, Format, FormatDoc};
use ooxml_tools::datasets::{dataset_id, row_range, DATASET_PREFIX};
use ooxml_tools::sources::qualify;
use ooxml_tools::sources::{SourceRefused, SourceRows};
use ooxml_tools::written::{CallValues, HeldTable, HeldValues, OneValue, Written};
use ooxml_tools::{ErrorCode, Holding, Refusal, ToolError, TryCall, WayOut};
use serde_json::{Map, Value};
use std::cell::RefCell;
use std::sync::Arc;

/// An open document as the call's sources read it.
struct Open {
    handle: String,
    name: String,
    format: Arc<dyn Format>,
    doc: Arc<dyn FormatDoc>,
    at: AnyVersion,
}

/// A call's arguments with its sources qualified and its references written, and the
/// values handed to its format.
pub(crate) struct WithValues {
    /// The arguments.
    pub(crate) arguments: Map<String, Value>,
    /// The values.
    pub(crate) values: CallValues,
    /// The other documents it holds for reading (those its sources name).
    pub(crate) held: Vec<String>,
}

/// A refusal at steps 6 to 8: where it came (`before`: a source qualified or a dataset
/// named, before the documents are held; else after), and the documents held then.
pub(crate) struct Refused {
    /// The refusal.
    pub(crate) error: ToolError,
    /// Whether it came before the call's documents were held.
    pub(crate) before: bool,
    /// The other documents held when it came.
    pub(crate) held: Vec<String>,
}

impl Refused {
    fn after(error: ToolError, held: &[String]) -> Refused {
        Refused {
            error,
            before: false,
            held: held.to_vec(),
        }
    }
}

impl HostSession {
    /// Steps 6 to 8 for a call of `entry` on `doc` with `arguments` (refs resolved): its
    /// sources qualified, its datasets checked, its references written, what its format
    /// reads of its sources resolved; what it names noted in `datasets`.
    ///
    /// # Errors
    /// The refusal of a source, a dataset or a reference ([`Refused`]).
    pub(crate) fn with_values(
        &self,
        entry: &Entry,
        doc: &str,
        arguments: Map<String, Value>,
        retry: &TryCall,
        datasets: &mut CallDatasets,
        held: &[HeldValues],
    ) -> Result<WithValues, Refused> {
        let sources = entry.spec.paths(Holding::Sources);
        let named = entry.spec.paths(Holding::Datasets);
        let facts = entry.spec.paths(Holding::Facts);
        if sources.is_empty() && named.is_empty() && facts.is_empty() {
            return Ok(WithValues {
                arguments,
                values: CallValues::default(),
                held: Vec::new(),
            });
        }
        let opens = self.opens(held);
        let host = sources_of(&self.host.formats, &opens, retry);
        // (the session's datasets as they stand: read without the stores' lock held)
        let store: DatasetStore = self.stores().datasets.clone();
        let heads = |d: &str| opens.iter().find(|o| o.handle == d).map(|o| o.at.number());
        let given = arguments.clone();
        let mut held: Vec<String> = Vec::new();
        let row_prefixes = host_prefixes(&self.host.formats);
        let rows: Vec<&str> = row_prefixes.iter().map(String::as_str).collect();
        let known = known_prefixes(&rows, &[]);
        let paths = Paths { sources, named, facts };
        // 6. the sources, qualified (their documents held), and the datasets named
        let qualified = {
            let call = CallSources {
                host: &host,
                store: &store,
                datasets: RefCell::new(&mut *datasets),
                heads: &heads,
                tool: entry.name(),
                arguments: &given,
                primary: Some(doc),
                held: &[],
                retry,
                own: None,
            };
            qualify_sources(&call, arguments, &paths, &known, &mut held).map_err(|error| Refused {
                error,
                before: true,
                held: Vec::new(),
            })?
        };
        // 7. the references, and 8. what the format reads of its sources
        let locale = self.session.config().locale.clone().unwrap_or_default();
        let call = CallSources {
            host: &host,
            store: &store,
            datasets: RefCell::new(&mut *datasets),
            heads: &heads,
            tool: entry.name(),
            arguments: &given,
            primary: Some(doc),
            held: &held,
            retry,
            own: None,
        };
        let ids: Vec<String> = {
            let mut ids = call.datasets.borrow().ids();
            ids.extend(store.ids());
            ids.dedup();
            ids
        };
        let env = FactEnv {
            known: &known,
            own: &[],
            datasets: &ids,
            locale: &locale,
            retry,
        };
        let (arguments, mut values) =
            read_values(&call, &env, &store, qualified, &paths).map_err(|e| Refused::after(e, &held))?;
        if opens.len() > 1 {
            values.tables = held_tables(&host, &store, retry);
        }
        Ok(WithValues {
            arguments,
            values,
            held,
        })
    }

    /// `read` run with what a reference writes in this session (the dispatcher's `Call.fact`):
    /// the text of the value an address names (`d:x/North/avg:0.0`, `d2/t:Scores/door/mean`),
    /// every open document held for reading; the refusal's message where it is refused.
    pub(crate) fn with_facts<R>(
        &self,
        retry: &TryCall,
        held: &[HeldValues],
        read: impl FnOnce(&dyn Fn(&str) -> Result<String, String>) -> R,
    ) -> R {
        let opens = self.opens(held);
        let host = sources_of(&self.host.formats, &opens, retry);
        let store: DatasetStore = self.stores().datasets.clone();
        let heads = |d: &str| opens.iter().find(|o| o.handle == d).map(|o| o.at.number());
        let held: Vec<String> = opens.iter().map(|o| o.handle.clone()).collect();
        let row_prefixes = host_prefixes(&self.host.formats);
        let rows: Vec<&str> = row_prefixes.iter().map(String::as_str).collect();
        let known = known_prefixes(&rows, &[]);
        let locale = self.session.config().locale.clone().unwrap_or_default();
        let ids = store.ids();
        let arguments = Map::new();
        let mut datasets = CallDatasets::new();
        let call = CallSources {
            host: &host,
            store: &store,
            datasets: RefCell::new(&mut datasets),
            heads: &heads,
            tool: retry.tool.as_str(),
            arguments: &arguments,
            primary: None,
            held: &held,
            retry,
            own: None,
        };
        let env = FactEnv {
            known: &known,
            own: &[],
            datasets: &ids,
            locale: &locale,
            retry,
        };
        let fact = |address: &str| {
            ooxml_session::facts::fact(address, "", &env, &call)
                .map(|(text, _)| text)
                .map_err(|error| error.message().to_owned())
        };
        read(&fact)
    }

    /// The call's datasets kept as it ends (at publish for a changing call): the warnings
    /// of the stale ones it named.
    ///
    /// # Errors
    /// `limit`: the session would keep over its limit.
    pub(crate) fn finish_datasets(&self, datasets: CallDatasets) -> Result<Vec<String>, ToolError> {
        let opens = self.opens(&[]);
        let heads = |d: &str| opens.iter().find(|o| o.handle == d).map(|o| o.at.number());
        datasets.finish(&mut self.stores().datasets, &heads)
    }

    /// The open documents, each at its head; one the application's own host holds as the
    /// call reads it, with what the application resolved of it (`held`).
    fn opens(&self, held: &[HeldValues]) -> Vec<Open> {
        let mut out = Vec::new();
        for id in self.session.documents() {
            let (Ok(opened), Ok(doc)) = (self.session.snapshot(&id), self.session.format_doc(&id)) else {
                continue;
            };
            let doc = self.answering(&id, doc, held);
            let Some(format) = self.host.formats.iter().find(|f| f.kind() == opened.kind()) else {
                continue;
            };
            out.push(Open {
                handle: id.to_string(),
                name: opened.name().to_owned(),
                format: Arc::clone(format),
                doc,
                at: opened.any().clone(),
            });
        }
        out
    }
}

impl HostSession {
    /// `doc`, the open document `id`, as the call reads it: a stand-in of a document the
    /// application's own host holds answering with what the application resolved of it for
    /// the call (`held`; nothing resolved: every read refused); any other as it is.
    pub(crate) fn answering(&self, id: &DocId, doc: Arc<dyn FormatDoc>, held: &[HeldValues]) -> Arc<dyn FormatDoc> {
        let handle = id.to_string();
        let Some(stand_in) = self.stores().held.get(&handle).cloned() else {
            return doc;
        };
        let values = held.iter().find(|v| v.doc == handle).cloned().unwrap_or(HeldValues {
            doc: handle,
            ..HeldValues::default()
        });
        stand_in.answering(values)
    }
}

/// The values a call wrote for references, in its reply's `data.facts` where its tool said
/// none of its own (ooxml-edit's `_report_facts`): each reference, the text written and its
/// sources.
pub(crate) fn report_facts(result: &mut ooxml_tools::ToolResult<Value>, written: &[Written]) {
    if written.is_empty() {
        return;
    }
    let ooxml_tools::Outcome::Done(done) = &mut result.outcome else {
        return;
    };
    let said: Vec<Value> = written.iter().map(|w| Value::Object(w.brief.clone())).collect();
    match &mut done.data {
        None => {
            let mut data = Map::new();
            data.insert("facts".into(), Value::from(said));
            done.data = Some(Value::Object(data));
        }
        Some(Value::Object(data)) if !data.contains_key("facts") => {
            data.insert("facts".into(), Value::from(said));
        }
        Some(_) => {}
    }
}

/// The argument paths a tool declares: sources, datasets, texts with references.
struct Paths {
    sources: Vec<String>,
    named: Vec<String>,
    facts: Vec<String>,
}

/// Step 6: `arguments` with every address of a declared source (and of a reference in a
/// declared text) qualified with its document, each document noted in `held`; every
/// dataset a declared argument names checked.
fn qualify_sources(
    call: &CallSources<'_>,
    arguments: Map<String, Value>,
    paths: &Paths,
    known: &[String],
    held: &mut Vec<String>,
) -> Result<Value, ToolError> {
    let mut out = Value::Object(arguments);
    for path in &paths.sources {
        out = walk(&out, path, &mut |value, field| {
            if value.starts_with(DATASET_PREFIX) {
                call.name_dataset(value, field, true)?;
                return Ok(value.to_owned());
            }
            match call.qualify(value, field, false)? {
                Some((found, written)) => {
                    push(held, found);
                    Ok(written)
                }
                None => Ok(value.to_owned()),
            }
        })?;
    }
    for path in &paths.named {
        walk(&out, path, &mut |value, field| {
            if value.starts_with(DATASET_PREFIX) {
                call.name_dataset(value, field, false)?;
            }
            Ok(value.to_owned())
        })?;
    }
    for path in &paths.facts {
        out = walk(&out, path, &mut |text, field| {
            if !text.contains("{{") {
                return Ok(text.to_owned());
            }
            qualify_text(text, known, &[], |address| {
                match call.qualify(address, field, true)? {
                    Some((found, written)) => {
                        push(held, found);
                        Ok(written)
                    }
                    None => Ok(address.to_owned()),
                }
            })
        })?;
    }
    Ok(out)
}

/// Steps 7 and 8: the references in the declared texts written, and what the format reads
/// of each declared source.
fn read_values(
    call: &CallSources<'_>,
    env: &FactEnv<'_>,
    store: &DatasetStore,
    qualified: Value,
    paths: &Paths,
) -> Result<(Map<String, Value>, CallValues), ToolError> {
    let (expanded, found) = if paths.facts.is_empty() {
        (qualified, Vec::new())
    } else {
        expand_arguments(&qualified, &paths.facts, env, call)?
    };
    let mut values = CallValues {
        written: found
            .iter()
            .map(|fact| Written {
                field: fact.field.clone(),
                start: fact.start,
                end: fact.end,
                text: fact.text.clone(),
                record: fact.record.clone(),
                brief: fact.brief(),
                markdown: fact_markdown(&fact.address, &fact.field, call, env, store),
            })
            .collect(),
        ..CallValues::default()
    };
    for path in &paths.sources {
        walk(&expanded, path, &mut |value, field| {
            read_source(value, field, call, env, store, &mut values)?;
            Ok(value.to_owned())
        })?;
    }
    let Value::Object(arguments) = expanded else {
        return Err(ToolError::internal("sources resolved to no object", "host:sources"));
    };
    Ok((arguments, values))
}

/// The tables of the open documents, each as a `from` names it (docx-agent's
/// `_host_tables`: `sources.offered`'s, each qualified with its document).
fn held_tables(host: &FormatSources<'_>, store: &DatasetStore, retry: &TryCall) -> Vec<HeldTable> {
    let documents = host.documents();
    let kinds: Vec<(String, String)> = documents
        .iter()
        .map(|d| (d.handle.clone(), d.kind.as_str().to_owned()))
        .collect();
    let (_, tables) = offered(host, &store.ids(), &[]);
    tables
        .into_iter()
        .filter_map(|name| {
            let (doc, _) = qualify(&name, &kinds, host.prefixes(), None, retry).ok()?;
            let kind = documents.iter().find(|d| d.handle == doc)?.kind.as_str().to_owned();
            Some(HeldTable { name, doc, kind })
        })
        .collect()
}

/// The sources of the open documents `opens`, as `formats` read them.
fn sources_of<'a>(formats: &'a [Arc<dyn Format>], opens: &'a [Open], retry: &TryCall) -> FormatSources<'a> {
    let formats: Vec<&dyn Format> = formats.iter().map(|f| &**f).collect();
    let docs = opens
        .iter()
        .map(|o| FormatOpen {
            handle: o.handle.clone(),
            name: o.name.clone(),
            format: &*o.format,
            doc: &*o.doc,
            at: Some(o.at.clone()),
        })
        .collect();
    FormatSources::new(formats, docs, retry)
}

/// The address prefixes the formats resolve to rows.
fn host_prefixes(formats: &[Arc<dyn Format>]) -> Vec<String> {
    formats.iter().flat_map(|f| f.source_prefixes()).collect()
}

fn push(held: &mut Vec<String>, doc: String) {
    if !held.contains(&doc) {
        held.push(doc);
    }
}

/// `node` with `visit(value, field)` applied to every non-empty string at the argument
/// path `path` (`cells[].from`).
fn walk(
    node: &Value,
    path: &str,
    visit: &mut dyn FnMut(&str, &str) -> Result<String, ToolError>,
) -> Result<Value, ToolError> {
    let parts: Vec<&str> = path.split('.').collect();
    walk_texts(node, &parts, "", &mut |value, field| {
        // ("": a strict decoder's placeholder for a source not given: left for the tool)
        if value.is_empty() {
            Ok(String::new())
        } else {
            visit(value, field)
        }
    })
}

/// Whether a source names one field of one row rather than rows: a dataset address past its
/// id that is no run of rows (`d:notes/#12/body`, `d:notes/Doors/body`).
#[must_use]
pub fn names_one_value(address: &str) -> bool {
    if !address.starts_with(DATASET_PREFIX) {
        return false;
    }
    let body = ooxml_session::facts::strip_format(address);
    let rest = body.get(DATASET_PREFIX.len()..).unwrap_or_default();
    rest.contains('/') && row_range(body).is_none()
}

/// Whether a source that is no dataset (`t:checks/Alpha/note`, `d2/t:...`) has the parts of
/// one value -- a table, a row and a column -- so that a refusal as rows is worth reading
/// it as one value.
#[must_use]
pub fn might_name_one_value(address: &str) -> bool {
    !address.starts_with(DATASET_PREFIX) && ooxml_session::facts::split(address).1.len() >= 2
}

/// What the format reads of the source `value` at `field`: its rows (or the host's refusal
/// of an unknown dataset); where it names one value -- or, being no dataset, has the parts
/// of one and names no rows -- that value, or its refusal (docx-agent's `field_from`: a
/// source that is no dataset is read as rows first, as one value only when rows refuse it,
/// and refused as rows when both do).
fn read_source(
    value: &str,
    field: &str,
    call: &CallSources<'_>,
    env: &FactEnv<'_>,
    store: &DatasetStore,
    values: &mut CallValues,
) -> Result<(), ToolError> {
    let certain = names_one_value(value);
    let mut rows_refused = None;
    if !certain {
        match call.source(value, field, false) {
            Ok(rows) => {
                if !values.sources.rows.iter().any(|r| r.given == rows.given) {
                    values.sources.rows.push(rows);
                }
                return Ok(());
            }
            Err(error) if value.starts_with(DATASET_PREFIX) && is_not_found(&error) => {
                values.sources.refused.push(refused(value, None, &error));
                return Ok(());
            }
            Err(error) if !might_name_one_value(value) => return Err(error),
            Err(error) => rows_refused = Some(error),
        }
    }
    let read = one_value(value, field, env, call)
        .and_then(|(rows, row, column)| Ok((ooxml_session::facts::fact(value, field, env, call)?, rows, row, column)));
    match (read, rows_refused) {
        (Ok(((text, record), rows, row, column)), _) => {
            let markdown = value_markdown(value, store, &rows, (row, column), call);
            values.written.push(as_written(value, field, text, record, &markdown));
            values.one_values.push(OneValue {
                given: value.to_owned(),
                field: field.to_owned(),
                shown: shown(value).to_owned(),
                rows,
                row,
                column,
                markdown,
            });
        }
        (Err(_), Some(error)) => values.sources.refused.push(refused(value, None, &error)),
        (Err(error), None) => {
            let error = with_rows(value, field, error, call);
            values.sources.refused.push(refused(value, Some(field), &error));
        }
    }
    Ok(())
}

/// The Markdown of the one value a reference writes (`{{d:notes/Roof/body}}`,
/// `{{t:Notes/#2/Body}}`), where its source holds one apart from its text (docx-agent's
/// `written_json`: a reference standing alone as a paragraph writes it as blocks); `None` for
/// any other.
fn fact_markdown(
    address: &str,
    field: &str,
    call: &CallSources<'_>,
    env: &FactEnv<'_>,
    store: &DatasetStore,
) -> Option<String> {
    let (rows, row, column) = one_value(address, field, env, call).ok()?;
    value_markdown(address, store, &rows, (row, column), call)
}

/// The Markdown of the value at `(row, column)` of `rows`, which the source `address` names:
/// a dataset's ([`dataset_markdown`]), or the rows its document's format gives
/// (`Dataset::markdown_at`: a workbook cell's rich text).
fn value_markdown(
    address: &str,
    store: &DatasetStore,
    rows: &SourceRows,
    (row, column): (usize, usize),
    call: &CallSources<'_>,
) -> Option<String> {
    if address.starts_with(DATASET_PREFIX) {
        return dataset_markdown(address, store, rows, row, column);
    }
    let doc = rows.doc.as_deref()?;
    let read = rows.address.strip_prefix(doc)?.strip_prefix('/')?;
    call.host.resolve(doc, read).ok()?.markdown_at(row, column)
}

/// A source address as a message shows it: without its document handle or format.
fn shown(address: &str) -> &str {
    unhandled(ooxml_session::facts::strip_format(address))
}

fn is_not_found(error: &ToolError) -> bool {
    error.code() == ErrorCode::Refusal(Refusal::NotFound)
}

/// The host's refusal of `given` (at `field`, for one value), as the format words it.
fn refused(given: &str, field: Option<&str>, error: &ToolError) -> SourceRefused {
    SourceRefused {
        given: given.to_owned(),
        message: error.message().to_owned(),
        options: error
            .way_out()
            .map_or(&[][..], WayOut::valid_options)
            .iter()
            .map(|o| o.as_str().map_or_else(|| o.to_string(), str::to_owned))
            .collect(),
        field: field.map(str::to_owned),
        code: error.code(),
        details: error.details().clone(),
    }
}

/// A value a source names, written as a value by reference is (docx-agent's `as_fact`): all
/// of its text, the address without its document handle or format.
fn as_written(
    address: &str,
    field: &str,
    text: String,
    record: Map<String, Value>,
    markdown: &Option<String>,
) -> Written {
    let fact = ooxml_session::facts::Fact {
        field: field.to_owned(),
        start: 0,
        end: text.chars().count(),
        text,
        address: shown(address).to_owned(),
        record,
    };
    Written {
        brief: fact.brief(),
        field: fact.field,
        start: fact.start,
        end: fact.end,
        text: fact.text,
        record: fact.record,
        markdown: markdown.clone(),
    }
}

/// `address` without its document handle (`d2/t:x` -> `t:x`).
fn unhandled(address: &str) -> &str {
    match address.split_once('/') {
        Some((handle, rest)) if DocId::parse(handle).is_some() => rest,
        _ => address,
    }
}

/// The Markdown a dataset holds for the one value `address` names apart from its text (a
/// workbook cell's emphasis and list lines), where the rows are the whole dataset's.
fn dataset_markdown(
    address: &str,
    store: &DatasetStore,
    rows: &SourceRows,
    row: usize,
    column: usize,
) -> Option<String> {
    let id = dataset_id(address)?;
    let entry = store.find(id)?;
    (rows.address == format!("{DATASET_PREFIX}{id}")).then(|| entry.dataset.markdown_at(row, column))?
}

/// A refusal of a row or column a dataset lacks given the way out (docx-agent's
/// `with_rows`, rule 2): whole addresses -- the first and last rows with the same field, the
/// row's fields -- in place of none or of bare column names.
fn with_rows(address: &str, field: &str, error: ToolError, call: &CallSources<'_>) -> ToolError {
    if !is_not_found(&error) || !address.starts_with(DATASET_PREFIX) {
        return error;
    }
    let (source, segments, _format) = ooxml_session::facts::split(address);
    let [row, column] = segments.as_slice() else {
        return error;
    };
    let Ok(rows) = call.source(&source, field, false) else {
        return error;
    };
    let quote = ooxml_session::facts::quote;
    let row = if row.1 { quote(&row.0) } else { row.0.clone() };
    let count = rows.rows.len();
    let options = error.way_out().map_or(&[][..], WayOut::valid_options).to_vec();
    let numbered = row
        .strip_prefix('#')
        .is_some_and(|n| n.starts_with(|c: char| ('1'..='9').contains(&c)) && n.bytes().all(|b| b.is_ascii_digit()));
    let named: Option<Vec<String>> = options.iter().map(|o| o.as_str().map(str::to_owned)).collect();
    let replaced: Option<Vec<String>> = if options.is_empty() && numbered && count > 0 {
        let mut found = vec![
            format!("{source}/#1/{}", quote(&column.0)),
            format!("{source}/#{count}/{}", quote(&column.0)),
        ];
        found.dedup();
        Some(found)
    } else {
        named
            .filter(|n| !n.is_empty() && n.iter().all(|c| rows.columns.contains(c)))
            .map(|n| n.iter().map(|c| format!("{source}/{row}/{}", quote(c))).collect())
    };
    match replaced.and_then(WayOut::names) {
        Some(way_out) => error.with_way_out(way_out),
        None => error,
    }
}
