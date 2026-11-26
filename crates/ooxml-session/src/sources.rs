//! Owns: a call's sources as the host resolves them (design phase 3, section 5;
//! ooxml-edit's `sources.py` and the dispatcher's `Call.source`, ported exactly): a dataset
//! or a run of its rows ([`crate::datasets`]), else an address of another open document,
//! qualified with its document ([`qualify`]) and read now by that document's format
//! ([`SourceHost::resolve`], `FormatDoc::source`), within the cells a dataset may hold
//! ([`CallSources::source`]); the refusal of a source that names no rows, offering only
//! what resolves (round 56, R120: [`refusal`], [`CallSources::refine`]); and what a text's
//! references read ([`CallSources`] is the facts' [`FactSources`]).
//!
//! [`FormatSources`] is the [`SourceHost`] of the session's formats: prefixes from each
//! [`Format`], rows and words from each [`FormatDoc`] at the version the call reads.
//!
//! Must not: hold a document or its lock (the host says which documents a call holds and
//! which it can read now), or know a format: each format's hooks come through
//! [`SourceHost`].

use crate::datasets::{CallDatasets, DatasetStore, Heads};
use crate::facts::FactSources;
use crate::{no_source, AnyVersion, DocKind, Format, FormatDoc};
use ooxml_tools::datasets::{dataset_id, Dataset, DATASET_PREFIX};
use ooxml_tools::sources::{doc_order, qualified, qualify, Prefixes, SourceRows};
use ooxml_tools::{ErrorCode, Failure, Refusal, ToolError, TryCall, WayOut};
use serde_json::{Map, Value};
use std::cell::RefCell;

/// The most sources a refusal offers (its options and its words); the rest are counted.
pub const MAX_OFFERED: usize = 12;

/// An open document, as a source's refusal and rows name it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenDoc {
    /// Its handle (`d2`).
    pub handle: String,
    /// Its kind.
    pub kind: DocKind,
    /// Its name (`stock.xlsx`).
    pub name: String,
}

/// What the host gives the sources of a call: the open documents, the prefixes each kind
/// resolves to rows, and each document's format's hooks, read at the version the host lets
/// the call read (a document it cannot read now gives nothing to a refusal's words).
pub trait SourceHost {
    /// The open documents.
    fn documents(&self) -> Vec<OpenDoc>;

    /// The address prefixes each kind resolves to rows (`t:`).
    fn prefixes(&self) -> &Prefixes;

    /// The rows `address` names in document `doc`, read by its format
    /// (`FormatDoc::source`).
    ///
    /// # Errors
    /// The format's refusal (`not_found` with the names there are), or none: its kind
    /// takes no such address.
    fn resolve(&self, doc: &str, address: &str) -> Result<Dataset, ToolError>;

    /// The addresses of the rows `doc` holds (its format's `value_sources`); `None` when its
    /// format says none, or the document cannot be read now.
    fn value_sources(&self, doc: &str) -> Option<Vec<String>> {
        let _ = doc;
        None
    }

    /// What `address` names in `doc` that gives no rows (`"a table"`: its format's
    /// `own_source`); `None` for nothing.
    fn own_source(&self, doc: &str, address: &str) -> Option<String> {
        let _ = (doc, address);
        None
    }

    /// What `doc`'s format says of a source value that names nothing (`source_hint`).
    fn source_hint(&self, doc: &str, value: &str) -> Option<String> {
        let _ = (doc, value);
        None
    }

    /// A run of a `kind` document's own addresses written compactly (`merge_sources`); as
    /// they are by default.
    fn merge_sources(&self, kind: DocKind, sources: Vec<String>) -> Vec<String> {
        let _ = kind;
        sources
    }
}

/// An open document as its format keeps it, for [`FormatSources`]: its handle and name, its
/// format, the document, and the version the call may read (`None`: it cannot be read now,
/// so it gives a refusal no words).
pub struct FormatOpen<'a> {
    /// Its handle (`d2`).
    pub handle: String,
    /// Its name.
    pub name: String,
    /// Its format.
    pub format: &'a dyn Format,
    /// The document.
    pub doc: &'a dyn FormatDoc,
    /// The version the call reads.
    pub at: Option<AnyVersion>,
}

/// The sources of a call as the session's formats give them (design section 5): each
/// kind's prefixes from its [`Format`], each document's rows and words from its
/// [`FormatDoc`] at the version the call reads.
pub struct FormatSources<'a> {
    docs: Vec<FormatOpen<'a>>,
    formats: Vec<&'a dyn Format>,
    prefixes: Prefixes,
    retry: TryCall,
}

impl<'a> FormatSources<'a> {
    /// The sources of `docs`, the formats loaded being `formats`; `retry`: the call as
    /// given, the way out of [`no_source`].
    #[must_use]
    pub fn new(formats: Vec<&'a dyn Format>, docs: Vec<FormatOpen<'a>>, retry: &TryCall) -> Self {
        let prefixes = Prefixes::new(formats.iter().map(|f| (f.kind().as_str(), f.source_prefixes())));
        Self {
            docs,
            formats,
            prefixes,
            retry: retry.clone(),
        }
    }

    fn open(&self, doc: &str) -> Option<&FormatOpen<'a>> {
        self.docs.iter().find(|d| d.handle == doc)
    }
}

impl SourceHost for FormatSources<'_> {
    fn documents(&self) -> Vec<OpenDoc> {
        self.docs
            .iter()
            .map(|d| OpenDoc {
                handle: d.handle.clone(),
                kind: d.format.kind(),
                name: d.name.clone(),
            })
            .collect()
    }

    fn prefixes(&self) -> &Prefixes {
        &self.prefixes
    }

    fn resolve(&self, doc: &str, address: &str) -> Result<Dataset, ToolError> {
        let Some(open) = self.open(doc) else {
            return Err(ToolError::refusal(
                Refusal::NotFound,
                format!("no open document {}", ooxml_base::pyfmt::repr_str(doc)),
                WayOut::call(self.retry.clone()),
            ));
        };
        let kind = open.format.kind();
        let found = open.at.as_ref().and_then(|at| open.doc.source(at, address));
        found.unwrap_or_else(|| Err(no_source(kind, address, &self.retry)))
    }

    fn value_sources(&self, doc: &str) -> Option<Vec<String>> {
        let open = self.open(doc)?;
        open.doc.value_sources(open.at.as_ref()?)
    }

    fn own_source(&self, doc: &str, address: &str) -> Option<String> {
        let open = self.open(doc)?;
        open.doc.own_source(open.at.as_ref()?, address)
    }

    fn source_hint(&self, doc: &str, value: &str) -> Option<String> {
        let open = self.open(doc)?;
        open.doc.source_hint(open.at.as_ref()?, value)
    }

    fn merge_sources(&self, kind: DocKind, sources: Vec<String>) -> Vec<String> {
        match self.formats.iter().find(|f| f.kind() == kind) {
            Some(format) => format.merge_sources(sources),
            None => sources,
        }
    }
}

/// The open documents' handles and kinds, as [`qualify`] takes them.
fn kinds(documents: &[OpenDoc]) -> Vec<(String, String)> {
    documents
        .iter()
        .map(|d| (d.handle.clone(), d.kind.as_str().to_owned()))
        .collect()
}

/// The sources there are, as a `from` takes them (`offered`): the datasets (`pending`: ids
/// an earlier op of the batch defined, then the session's) and each open document's
/// tables, written bare when one open document alone takes their prefix, else with its
/// handle (`d2/t:stock`).
#[must_use]
pub fn offered(host: &dyn SourceHost, session: &[String], pending: &[String]) -> (Vec<String>, Vec<String>) {
    let mut ids: Vec<String> = Vec::new();
    for id in pending
        .iter()
        .map(|p| format!("{DATASET_PREFIX}{p}"))
        .chain(session.iter().cloned())
    {
        if !ids.contains(&id) {
            ids.push(id);
        }
    }
    let taken = host.prefixes();
    let mut documents = host.documents();
    documents.sort_by(|a, b| doc_order(&a.handle).cmp(&doc_order(&b.handle)));
    let mut tables: Vec<String> = Vec::new();
    for entry in &documents {
        if taken.of(entry.kind.as_str()).is_empty() {
            continue;
        }
        let Some(found) = host.value_sources(&entry.handle) else {
            continue;
        };
        for address in found {
            let Some(prefix) = taken
                .of(entry.kind.as_str())
                .iter()
                .find(|p| address.starts_with(p.as_str()))
            else {
                continue;
            };
            let sharing = documents
                .iter()
                .filter(|d| taken.of(d.kind.as_str()).contains(prefix))
                .count();
            let table = if sharing == 1 {
                address
            } else {
                format!("{}/{address}", entry.handle)
            };
            if !tables.contains(&table) {
                tables.push(table);
            }
        }
    }
    (ids, tables)
}

/// `option` as a source that resolves -- a dataset there is, an address that names rows of
/// an open document -- else `None` (`resolving`). `doc`: the document a bare address was
/// looked up in (a resolver's own suggestions), which it is then read in, written with the
/// handle when bare it would name another document or several.
#[must_use]
pub fn resolving(
    option: &str,
    host: &dyn SourceHost,
    session: &[String],
    pending: &[String],
    doc: Option<&str>,
    retry: &TryCall,
) -> Option<String> {
    if option.is_empty() {
        return None;
    }
    if option.starts_with(DATASET_PREFIX) {
        let found = dataset_id(option)?;
        let known = pending.iter().any(|p| p == found) || session.iter().any(|a| dataset_id(a) == Some(found));
        return known.then(|| option.to_owned());
    }
    let documents = kinds(&host.documents());
    let taken = host.prefixes();
    let named = match doc {
        Some(doc) if qualified(option).is_none() => format!("{doc}/{option}"),
        _ => option.to_owned(),
    };
    let (handle, address) = qualify(&named, &documents, taken, None, retry).ok()?;
    if let Err(error) = host.resolve(&handle, &address) {
        if error.code() != ErrorCode::Failure(Failure::Limit) {
            return None; // too many values: there, but too big
        }
    }
    let bare = qualify(&address, &documents, taken, None, retry).is_ok_and(|(found, _)| found == handle);
    Some(if bare { address } else { format!("{handle}/{address}") })
}

/// Whether a refusal's option is a source address (a dataset, an open document's rows),
/// which [`refusal`] offers only when it resolves.
#[must_use]
pub fn source_like(option: &Value, taken: &Prefixes) -> bool {
    let Some(option) = option.as_str() else { return false };
    let body = qualified(option).map_or(option, |(_, body)| body);
    option.starts_with(DATASET_PREFIX) || taken.known().iter().any(|p| body.starts_with(p.as_str()))
}

/// What a source is, in words: `a dataset (d:<id>) or rows of an open <kind> document
/// (t:<name>)`.
#[must_use]
pub fn shapes(taken: &Prefixes) -> String {
    let kinds = taken
        .kinds()
        .filter(|(_, found)| !found.is_empty())
        .map(|(kind, found)| {
            let names: Vec<String> = found.iter().map(|p| format!("{p}<name>")).collect();
            format!("rows of an open {kind} document ({})", names.join(", "))
        });
    std::iter::once(format!("a dataset ({DATASET_PREFIX}<id>)"))
        .chain(kinds)
        .collect::<Vec<_>>()
        .join(" or ")
}

/// `datasets d:a, d:b and tables t:stock`: at most [`MAX_OFFERED`] in all, the rest
/// counted.
fn listing(ids: &[String], tables: &[String]) -> String {
    let mut room = MAX_OFFERED;
    let mut parts = Vec::new();
    for (name, found) in [("datasets", ids), ("tables", tables)] {
        if found.is_empty() {
            continue;
        }
        let shown = found.get(..room.min(found.len())).unwrap_or(found);
        room -= shown.len();
        let rest = found.len() - shown.len();
        let mut words = shown.join(", ");
        if rest > 0 {
            words.push_str(&format!(" and {rest} more"));
        }
        parts.push(if shown.is_empty() {
            format!("{rest} {name}")
        } else {
            format!("{name} {words}")
        });
    }
    parts.join(" and ")
}

/// Whether `value` names a whole source -- no dataset, no row or column of one (`t:stock`,
/// `d2/t:stock`): what a dataset may stand in for.
fn whole(value: &str) -> bool {
    let body = qualified(value).map_or(value, |(_, body)| body);
    !body.starts_with(DATASET_PREFIX) && !body.contains('/')
}

/// `(doc, address, what)` when `value` names a thing of a document that gives no rows --
/// the call's own document's table (bare, or with its handle), or another's named with its
/// handle -- by that format's `own_source`; `None` otherwise (`own`). A value named with
/// another handle whose document has no such thing is looked up in the call's own document
/// too: a bare `t:` was qualified with the one document taking `t:` before it got here.
#[must_use]
pub fn own(value: &str, host: &dyn SourceHost, primary: Option<&str>) -> Option<(String, String, String)> {
    let (handle, address) = qualified(value).map_or((primary, value), |(h, a)| (Some(h), a));
    let documents = host.documents();
    let mut tried: Vec<&str> = Vec::new();
    for doc in [handle, primary].into_iter().flatten() {
        if tried.contains(&doc) {
            continue;
        }
        tried.push(doc);
        if !documents.iter().any(|d| d.handle == doc) {
            continue;
        }
        if let Some(what) = host.own_source(doc, address) {
            let what = what.trim();
            if !what.is_empty() {
                return Some((doc.to_owned(), address.to_owned(), what.to_owned()));
            }
        }
    }
    None
}

/// The call that would work with an address in a refused value's place.
pub type MeantCall<'a> = &'a dyn Fn(&str) -> Option<Value>;

/// What a source refusal is about, beside the error (`refusal`'s keywords).
#[derive(Default)]
pub struct Refusing<'a> {
    /// The argument, where the error names none.
    pub field: Option<&'a str>,
    /// The call's document.
    pub primary: Option<&'a str>,
    /// The ids an earlier op of the batch defined.
    pub pending: &'a [String],
    /// The document whose resolver refused (`None`: the qualifying did).
    pub doc: Option<&'a str>,
    /// The call that would work with an address in the value's place (`details.call`).
    pub call: Option<MeantCall<'a>>,
    /// What the call's format adds to a `not_found` (`source_hint`).
    pub hint: Option<String>,
    /// The value is a reference's address in a text (`{{t:stock/#1/count}}`).
    pub fact: bool,
}

/// Python's `json.dumps` of text: ASCII, the rest escaped.
fn json_text(text: &str) -> String {
    let mut out = String::from("\"");
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            c if c.is_ascii() && !c.is_ascii_control() => out.push(c),
            c => {
                let mut units = [0u16; 2];
                for unit in c.encode_utf16(&mut units) {
                    out.push_str(&format!("\\u{unit:04x}"));
                }
            }
        }
    }
    out.push('"');
    out
}

/// A refusal of `code` with `options` as its way out (the call as given when there are
/// none), its field and details.
fn refused(
    code: Refusal,
    message: String,
    field: Option<&str>,
    options: Vec<Value>,
    details: Map<String, Value>,
    retry: &TryCall,
) -> ToolError {
    let way_out = WayOut::options(options).unwrap_or_else(|| WayOut::call(retry.clone()));
    let mut error = ToolError::refusal(code, message, way_out);
    if let Some(field) = field {
        error = error.at(field);
    }
    for (key, value) in details {
        error = error.with_detail(&key, value);
    }
    error
}

/// The refusal of a value that names a thing of a document that gives no rows ([`own`]).
fn own_refusal(
    (handle, address, what): (String, String, String),
    host: &dyn SourceHost,
    at: &Refusing<'_>,
    place: Option<&str>,
    (names, tables): (&[String], &[String]),
    retry: &TryCall,
) -> ToolError {
    let taken = host.prefixes();
    let every: Vec<String> = names.iter().chain(tables).cloned().collect();
    let shown: Vec<String> = every.iter().take(MAX_OFFERED).cloned().collect();
    let kind = host
        .documents()
        .into_iter()
        .find(|d| d.handle == handle)
        .map_or("", |d| d.kind.as_str());
    let whose = if Some(handle.as_str()) == at.primary {
        format!("{handle}, the document this call edits")
    } else {
        format!("{handle} ({kind})")
    };
    let mut details = Map::new();
    let names_it = serde_json::json!({"doc": handle, "address": address, "is": what});
    let none = "; there is none: define a dataset or open a document with tables first";
    let shown_values: Vec<Value> = shown.iter().map(|s| Value::from(s.as_str())).collect();
    if at.fact {
        let mut message = format!(
            "{address} is {what} of {whose}, not a value to quote: a reference in {} names one value of {}, as \
             {{{{<source>/#<row>/<column>}}}}",
            place.unwrap_or("a text"),
            shapes(taken)
        );
        if every.is_empty() {
            message.push_str(none);
        } else {
            message.push_str(&format!("; there are {}", listing(names, tables)));
        }
        details.insert("names".into(), names_it);
        return refused(Refusal::InvalidArguments, message, place, shown_values, details, retry);
    }
    let mut message = format!(
        "{address} is {what} of {whose}, not rows to take: {} takes {}",
        place.unwrap_or("a source"),
        shapes(taken)
    );
    if let Some(first) = shown.first() {
        message.push_str(&format!("; there are {}", listing(names, tables)));
        message.push_str(&format!("; e.g. {}: {}", place.unwrap_or("from"), json_text(first)));
    } else {
        message.push_str(none);
    }
    details.insert("names".into(), names_it);
    if let ([one], Some(call)) = (every.as_slice(), at.call) {
        if let Some(meant) = call(one) {
            details.insert("call".into(), meant);
        }
    }
    refused(Refusal::InvalidArguments, message, place, shown_values, details, retry)
}

/// `error` with `message` in place of its own: the code, field, way out and details stay.
fn reworded(error: &ToolError, message: String) -> ToolError {
    let mut out = match error.code() {
        ErrorCode::Refusal(code) => ToolError::refusal(
            code,
            message,
            error.way_out().cloned().unwrap_or_else(|| WayOut::one(Value::Null)),
        ),
        ErrorCode::Failure(code) => {
            let out = ToolError::failure(code, message);
            match error.way_out() {
                Some(way_out) => out.with_way_out(way_out.clone()),
                None => out,
            }
        }
    };
    if let Some(field) = error.field() {
        out = out.at(field);
    }
    for (key, value) in error.details() {
        out = out.with_detail(key, value.clone());
    }
    out
}

/// `error` with a format's hint added to its message (`_with_hint`): the code, field,
/// options and details stay as they are.
#[must_use]
pub fn with_hint(error: ToolError, hint: Option<&str>) -> ToolError {
    match hint.filter(|h| !h.is_empty()) {
        Some(hint) => {
            let message = format!("{}; {hint}", error.message());
            reworded(&error, message)
        }
        None => error,
    }
}

/// A source refusal that offers only what resolves (round 56, R120; `sources.refusal`).
/// `error` is what resolving `value` gave (the qualifying, or the resolver of the document
/// `at.doc`); `session`: the session's dataset addresses.
///
/// * `value` names a thing of a document that gives no rows ([`own`]): refused as
///   `invalid_arguments` saying so, what a source is ([`shapes`]), the datasets and tables
///   there are ([`offered`]) as the options, an example call; with one source alone,
///   `details.call` is the call with it.
/// * Otherwise the error stands, its source options kept only when they resolve
///   ([`resolving`]); when none is left, the sources there are are offered in their place
///   -- and a `not_found` offers the datasets beside the tables.
///
/// Other codes (`limit`) are returned as they are.
#[must_use]
pub fn refusal(
    error: ToolError,
    value: &str,
    host: &dyn SourceHost,
    session: &[String],
    at: &Refusing<'_>,
    retry: &TryCall,
) -> ToolError {
    let ErrorCode::Refusal(code @ (Refusal::NotFound | Refusal::InvalidArguments | Refusal::Ambiguous)) = error.code()
    else {
        return error;
    };
    let taken = host.prefixes();
    let place = error.field().or(at.field).map(str::to_owned);
    let place = place.as_deref();
    let (names, tables) = offered(host, session, at.pending);
    let every: Vec<String> = names.iter().chain(&tables).cloned().collect();
    let shown: Vec<String> = every.iter().take(MAX_OFFERED).cloned().collect();
    let mut details = error.details().clone();
    if let Some(found) = own(value, host, at.primary) {
        return own_refusal(found, host, at, place, (&names, &tables), retry);
    }
    let given = error.way_out().map(|w| w.valid_options().to_vec()).unwrap_or_default();
    let mut options: Vec<Value> = Vec::new();
    let mut dropped = false;
    for option in given {
        if !source_like(&option, taken) {
            options.push(option);
            continue;
        }
        match resolving(
            option.as_str().unwrap_or_default(),
            host,
            session,
            at.pending,
            at.doc,
            retry,
        ) {
            Some(kept) => options.push(Value::from(kept)),
            None => dropped = true,
        }
    }
    let mut message = error.message().to_owned();
    if let (Some(hint), Refusal::NotFound) = (&at.hint, code) {
        message.push_str(&format!("; {hint}"));
    }
    let sourced =
        |options: &[Value]| -> Vec<Value> { options.iter().filter(|o| source_like(o, taken)).cloned().collect() };
    if dropped && sourced(&options).is_empty() && !every.is_empty() {
        message.push_str(&format!("; the sources there are: {}", listing(&names, &tables)));
        options.extend(shown.iter().map(|s| Value::from(s.as_str())));
    } else if code == Refusal::NotFound
        && !names.is_empty()
        && !at.fact
        && whole(value)
        && !options
            .iter()
            .any(|o| o.as_str().is_some_and(|s| s.starts_with(DATASET_PREFIX)))
    {
        let room = MAX_OFFERED.saturating_sub(options.len()).max(1);
        let extra: Vec<&String> = names.iter().take(room).collect();
        let rest = names.len() - extra.len();
        let words: Vec<&str> = extra.iter().map(|s| s.as_str()).collect();
        message.push_str(&format!("; or a dataset: {}", words.join(", ")));
        if rest > 0 {
            message.push_str(&format!(" and {rest} more"));
        }
        options.extend(extra.into_iter().map(|s| Value::from(s.as_str())));
    }
    let mut unique: Vec<Value> = Vec::with_capacity(options.len());
    for option in options {
        if !option.is_string() || !unique.contains(&option) {
            unique.push(option);
        }
    }
    let every_values: Vec<Value> = every.iter().map(|s| Value::from(s.as_str())).collect();
    if let ([one], Some(call)) = (every.as_slice(), at.call) {
        if sourced(&unique) == every_values {
            if let Some(meant) = call(one) {
                details.insert("call".into(), meant);
            }
        }
    }
    refused(code, message, place, unique, details, retry)
}

/// A call's sources, as the host resolves them for it: its datasets ([`CallDatasets`] over
/// the session's [`DatasetStore`]) and the other documents it holds.
pub struct CallSources<'a> {
    /// The open documents and their formats' hooks.
    pub host: &'a dyn SourceHost,
    /// The session's datasets.
    pub store: &'a DatasetStore,
    /// What the call (or batch) defined and named so far.
    pub datasets: RefCell<&'a mut CallDatasets>,
    /// Each open document's version now.
    pub heads: &'a dyn Heads,
    /// The tool called.
    pub tool: &'a str,
    /// Its arguments, as given (the call a refusal offers is them with another source).
    pub arguments: &'a Map<String, Value>,
    /// The call's document.
    pub primary: Option<&'a str>,
    /// The documents the call holds for reading.
    pub held: &'a [String],
    /// The call as given, the way out of a refusal that names no option.
    pub retry: &'a TryCall,
    /// The call's document's own references (`{{n:7}}`): its text and fields
    /// ([`FactSources::own`]); `None`: it takes none.
    #[allow(clippy::type_complexity)]
    pub own: Option<&'a dyn Fn(&str, &str, &str) -> Result<(String, Map<String, Value>), ToolError>>,
}

impl CallSources<'_> {
    /// The refusal a source value that names no rows gets instead of `error`
    /// (`_source_refiner`): the call's own document's table said to be one, the sources
    /// there are offered, only options that resolve, the call's format's hint on a
    /// `not_found` the dispatcher found nothing to resolve in (`doc` `None`), and the call
    /// with the one source there is, when there is one and `place` is a top-level argument.
    #[must_use]
    pub fn refine(
        &self,
        error: ToolError,
        value: &str,
        place: Option<&str>,
        doc: Option<&str>,
        fact: bool,
    ) -> ToolError {
        let hint = match (error.code(), doc) {
            (ErrorCode::Refusal(Refusal::NotFound), None) => self.hint(value),
            _ => None,
        };
        let call = |address: &str| -> Option<Value> {
            let place = place.filter(|p| self.arguments.contains_key(*p))?;
            let mut meant: Map<String, Value> = self
                .arguments
                .iter()
                .filter(|(_, v)| !v.is_null() && v.as_str() != Some(""))
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
            meant.insert(place.to_owned(), Value::from(address));
            Some(serde_json::json!({"tool": self.tool, "arguments": meant}))
        };
        let pending = self.datasets.borrow().ids();
        let at = Refusing {
            field: place,
            primary: self.primary,
            pending: &pending,
            doc,
            call: if fact { None } else { Some(&call) },
            hint,
            fact,
        };
        refusal(error, value, self.host, &self.store.ids(), &at, self.retry)
    }

    /// What the call's document's format says of a source `value` that names nothing
    /// (`_source_hint`), trimmed; `None` for nothing.
    #[must_use]
    pub fn hint(&self, value: &str) -> Option<String> {
        let said = self.host.source_hint(self.primary?, value)?;
        let said = said.trim();
        (!said.is_empty()).then(|| said.to_owned())
    }

    /// A dataset a declared argument names, checked before the handler runs (the
    /// dispatcher's `_with_datasets`): an unknown one refused with the ids there are, and,
    /// in a declared source (`hinted`), the call's format's hint.
    ///
    /// # Errors
    /// No such dataset.
    pub fn name_dataset(&self, value: &str, place: &str, hinted: bool) -> Result<(), ToolError> {
        let named = self.datasets.borrow_mut().name(value, place, self.store, self.retry);
        match named {
            Ok(_) => Ok(()),
            Err(error) if hinted && error.code() == ErrorCode::Refusal(Refusal::NotFound) => {
                Err(with_hint(error, self.hint(value).as_deref()))
            }
            Err(error) => Err(error),
        }
    }

    /// A source value of a declared argument qualified with its document (`t:sales` ->
    /// `d2/t:sales`) and that document, before the call runs (the dispatcher's `_sources`);
    /// `None` for a dataset or no value (`""`, left for the handler).
    ///
    /// # Errors
    /// The refusal of a value that names no document's rows ([`CallSources::refine`]).
    pub fn qualify(&self, value: &str, place: &str, fact: bool) -> Result<Option<(String, String)>, ToolError> {
        if value.is_empty() || value.starts_with(DATASET_PREFIX) {
            return Ok(None);
        }
        let documents = kinds(&self.host.documents());
        match qualify(value, &documents, self.host.prefixes(), Some(place), self.retry) {
            Ok((doc, address)) => Ok(Some((doc.clone(), format!("{doc}/{address}")))),
            Err(error) => Err(self.refine(error, value, Some(place), None, fact)),
        }
    }

    /// The rows a source names: a whole dataset or a run of its rows
    /// ([`CallDatasets::source`]), or an address in a document this call holds (`d2/t:sales`,
    /// `t:sales`), read now by that document's format. `fact`: the source of a reference in
    /// a text, for a refusal's words.
    ///
    /// # Errors
    /// A source that names no rows; a document the call does not hold; `limit` over the
    /// cells a dataset may hold.
    pub fn source(&self, value: &str, place: &str, fact: bool) -> Result<SourceRows, ToolError> {
        if value.starts_with(DATASET_PREFIX) {
            return self
                .datasets
                .borrow_mut()
                .source(value, place, self.store, self.heads, self.retry);
        }
        let documents = self.host.documents();
        let (doc, address) = qualify(value, &kinds(&documents), self.host.prefixes(), Some(place), self.retry)
            .map_err(|error| self.refine(error, value, Some(place), None, fact))?;
        let Some(held) = documents
            .iter()
            .find(|d| d.handle == doc)
            .filter(|_| self.held.contains(&doc))
        else {
            return Err(ToolError::refusal(
                Refusal::Refused,
                format!("{doc} is not held by this call: name it in an argument the tool declares as a source"),
                WayOut::call(self.retry.clone()),
            )
            .at(place));
        };
        let dataset = self
            .host
            .resolve(&doc, &address)
            .map_err(|error| self.refine(error, value, Some(place), Some(&doc), fact))?;
        let max = self.store.limits.max_cells;
        if dataset.cells() > max {
            return Err(ToolError::failure(
                Failure::Limit,
                format!(
                    "{doc}/{address} has {} values; at most {max}: derive a smaller dataset",
                    dataset.cells()
                ),
            )
            .at(place));
        }
        let version = self.heads.head(&doc).and_then(|v| i64::try_from(v).ok());
        Ok(dataset.source_rows(
            value,
            &format!("{doc}/{address}"),
            Some(&doc),
            Some(&held.name),
            version,
            dataset.invalidated(),
        ))
    }
}

impl FactSources for CallSources<'_> {
    fn rows(&self, source: &str, field: &str) -> Result<SourceRows, ToolError> {
        self.source(source, field, true)
    }

    fn own(&self, prefix: &str, name: &str, field: &str) -> Result<(String, Map<String, Value>), ToolError> {
        match self.own {
            Some(own) => own(prefix, name, field),
            None => Err(crate::facts::not_own(
                &format!("{prefix}{name}"),
                prefix,
                None,
                &[],
                field,
                None,
                self.retry,
            )),
        }
    }

    fn merge(&self, rows: &SourceRows, sources: Vec<String>) -> Vec<String> {
        let kind = if rows.address.starts_with(DATASET_PREFIX) {
            self.datasets.borrow().kind_of(&rows.address, self.store)
        } else {
            let doc = rows.doc.as_deref();
            self.host
                .documents()
                .into_iter()
                .find(|d| Some(d.handle.as_str()) == doc)
                .map(|d| d.kind)
        };
        match kind {
            Some(kind) => self.host.merge_sources(kind, sources),
            None => sources,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::json_text;

    #[test]
    fn text_is_written_as_python_json_writes_it() {
        assert_eq!(json_text("t:sales"), r#""t:sales""#);
        assert_eq!(
            json_text("t:Caf\u{e9} \"x\"\n\u{1f600}"),
            "\"t:Caf\\u00e9 \\\"x\\\"\\n\\ud83d\\ude00\""
        );
    }
}
