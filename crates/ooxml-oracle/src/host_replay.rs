//! Owns: the replay of the host goldens (`crates/ooxml-oracle/host/`, lane ph3-diff F1:
//! today's Python host recorded) against the Rust host (`ooxml-host`, Phase 3 D1): every
//! docx stream's cases opened in a [`HostSession`] of Word's format laid out with the
//! repository's open faces (as the recording pinned them), each event made again, and each
//! call's reply normalised as `host_goldens.normalised_result` normalises it and compared with
//! the recorded one leaf by leaf; a difference an entry of `host/known-differences.toml`
//! admits is known ([`Known`]: rule 1, or a user decision's class); a call that needs what
//! the Rust host does not run yet is counted as waiting, with the PR it waits for
//! ([`Waits`]), and so is every later event of its case that its state depends on.
//!
//! Must not: write the goldens (the Python recorder blesses them), or admit a difference no
//! entry names.

use ooxml_base::context::{FaceData, FaceRequest, Timestamp};
use ooxml_base::{BaseError, CacheBudget, Cancel, Clock, Context, FontSource, PartStore};
use ooxml_fonts::{Folder, Location, WordFontOptions};
use ooxml_host::{CallAs, Host, HostSession, HostSettings, HostTools, Opening};
use ooxml_session::{DocId, DocKind, MemoryBudget, SessionConfig};
use ooxml_session_word::{SessionPages, WordFormat};
use ooxml_store::StoreConfig;
use ooxml_tools::roles::RoleSet;
use ooxml_tools_word::PageSetup;
use ooxml_word_pages::png::PageRaster;
use ooxml_word_pages::{DiskFonts, EngineOptions, FontSetup};
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

/// What a call or event waits for before the Rust host can answer it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Waits {
    /// The session tools (open, new, close, read_blob, save_document's output, the
    /// application's undo, save and reopen with state): Phase 3 D3.
    D3,
    /// `batch` and turns of several calls: Phase 3 D2.
    D2,
    /// A host setting the replay does not make yet (limits, a toolbox's own settings).
    Settings,
}

impl Waits {
    /// Its name in the report.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Waits::D3 => "D3 (session tools)",
            Waits::D2 => "D2 (batch)",
            Waits::Settings => "settings",
        }
    }
}

/// How one event replayed.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    /// Its answer is the recorded one.
    Equal,
    /// It differs only where known differences admit it (the entries' indices).
    Known(Vec<usize>),
    /// It differs at these paths (want, have of the first few).
    Different(Vec<String>, Value, Value),
    /// It waits for this; `true` where it is waiting only because an earlier event of its
    /// case did.
    Waiting(Waits, bool),
}

/// One known difference (`host/known-differences.toml`): which events it matches and the
/// fields it admits.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Known {
    /// A glob on the stream's name.
    pub stream: Option<String>,
    /// A glob on the case's id.
    pub case: Option<String>,
    /// The call's tool.
    pub tool: Option<String>,
    /// The paths it admits (and everything below them).
    pub fields: Vec<String>,
    /// The tool design rule that makes the new answer right (rule 1), where it is one.
    pub rule: Option<i64>,
    /// The user decision it is, where it is one (`structured-problems-2026-11-25`,
    /// `optimistic-batches-2026-11-25`, `limits-in-details-2026-11-25`,
    /// `stamp-on-open-2026-11-26`).
    pub decision: Option<String>,
    /// Why.
    pub cause: String,
}

/// The user decisions an entry may give instead of a rule.
pub const DECISIONS: [&str; 4] = [
    "structured-problems-2026-11-25",
    "optimistic-batches-2026-11-25",
    "limits-in-details-2026-11-25",
    "stamp-on-open-2026-11-26",
];

/// The known differences in `text` (the TOML subset the file uses: `[[difference]]` tables
/// of strings, integers and lists of strings).
///
/// # Errors
/// A line it cannot read, or an entry without a cause, fields, or an allowed rule or
/// decision.
pub fn known_entries(text: &str) -> Result<Vec<Known>, String> {
    let mut out: Vec<Known> = Vec::new();
    let lines: Vec<&str> = text.lines().collect();
    let mut number = 0;
    while number < lines.len() {
        let mut line = lines.get(number).map_or("", |l| l.trim()).to_owned();
        number += 1;
        // a multi-line basic string (`"""..."""`, a `\` at a line's end joining the next)
        if let Some(at) = line.find("\"\"\"") {
            let mut body = line.get(at + 3..).unwrap_or("").to_owned();
            while !body.contains("\"\"\"") && number < lines.len() {
                body = match body.strip_suffix('\\') {
                    Some(joined) => format!("{joined}{}", lines.get(number).map_or("", |l| l.trim_start())),
                    None => format!("{body}\n{}", lines.get(number).unwrap_or(&"")),
                };
                number += 1;
            }
            let body = body.split("\"\"\"").next().unwrap_or("");
            line = format!("{}{}", line.get(..at).unwrap_or(""), Value::from(body));
        }
        let line = line.as_str();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line == "[[difference]]" {
            out.push(Known::default());
            continue;
        }
        let (key, value) = line
            .split_once('=')
            .ok_or_else(|| format!("line {number}: not key = value"))?;
        let entry = out
            .last_mut()
            .ok_or_else(|| format!("line {number}: outside [[difference]]"))?;
        let value: Value = serde_json::from_str(value.trim()).map_err(|e| format!("line {number}: {e}"))?;
        let text = || value.as_str().map(str::to_owned);
        match key.trim() {
            "stream" => entry.stream = text(),
            "case" => entry.case = text(),
            "tool" => entry.tool = text(),
            "cause" => entry.cause = text().unwrap_or_default(),
            "rule" => entry.rule = value.as_i64(),
            "decision" => entry.decision = text(),
            "fields" => {
                entry.fields = value
                    .as_array()
                    .map(|a| a.iter().filter_map(Value::as_str).map(str::to_owned).collect())
                    .unwrap_or_default();
            }
            other => return Err(format!("line {number}: no key {other}")),
        }
    }
    for entry in &out {
        let allowed = entry.rule == Some(1) || entry.decision.as_deref().is_some_and(|d| DECISIONS.contains(&d));
        if entry.cause.is_empty() || entry.fields.is_empty() || !allowed {
            return Err(format!(
                "a known difference without a cause, fields, or rule 1 / a decision: {entry:?}"
            ));
        }
    }
    Ok(out)
}

/// `text` matched against the glob `pattern` (`*` any run, `?` one character).
#[must_use]
pub fn glob(pattern: &str, text: &str) -> bool {
    fn at(p: &[char], t: &[char]) -> bool {
        match p.split_first() {
            None => t.is_empty(),
            Some(('*', rest)) => (0..=t.len()).any(|k| t.get(k..).is_some_and(|tail| at(rest, tail))),
            Some((c, rest)) => t
                .split_first()
                .is_some_and(|(d, tail)| (*c == '?' || c == d) && at(rest, tail)),
        }
    }
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = text.chars().collect();
    at(&p, &t)
}

/// `value` flattened to its leaves by path (`result.data.items[0].text`), as
/// `host_goldens._paths` flattens it.
#[must_use]
pub fn leaves(value: &Value, prefix: &str) -> BTreeMap<String, Value> {
    let mut out = BTreeMap::new();
    match value {
        Value::Object(map) if !map.is_empty() => {
            for (key, item) in map {
                let path = if prefix.is_empty() {
                    key.clone()
                } else {
                    format!("{prefix}.{key}")
                };
                out.extend(leaves(item, &path));
            }
        }
        Value::Array(items) if !items.is_empty() => {
            for (index, item) in items.iter().enumerate() {
                out.extend(leaves(item, &format!("{prefix}[{index}]")));
            }
        }
        other => {
            out.insert(prefix.to_owned(), other.clone());
        }
    }
    out
}

/// The paths where `have` differs from `want`.
#[must_use]
pub fn differing(want: &Value, have: &Value, prefix: &str) -> Vec<String> {
    let (a, b) = (leaves(want, prefix), leaves(have, prefix));
    let mut paths: Vec<String> = a
        .keys()
        .chain(b.keys())
        .filter(|p| a.get(*p) != b.get(*p))
        .cloned()
        .collect();
    paths.sort();
    paths.dedup();
    paths
}

/// `value` without any `timing` key, at any depth.
fn without_timing(value: &Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.iter()
                .filter(|(k, _)| k.as_str() != "timing")
                .map(|(k, v)| (k.clone(), without_timing(v)))
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.iter().map(without_timing).collect()),
        other => other.clone(),
    }
}

/// A reply as the streams hold it (`normalised_result`): the model's envelope, its data
/// without timing (and save_document's size), and the application's touched and changed
/// parts where there are some.
#[must_use]
pub fn normalised(result: &ooxml_tools::ToolResult<Value>) -> Value {
    let mut body = result.to_json();
    if let Some(data) = body.get("data").filter(|d| d.is_object() || d.is_array()) {
        let mut data = without_timing(data);
        if result.app.tool == "save_document" {
            if let Value::Object(map) = &mut data {
                map.shift_remove("size");
            }
        }
        body.insert("data".into(), data);
    }
    let mut app = Map::new();
    if !result.app.touched.is_empty() {
        app.insert("touched".into(), Value::Array(result.app.touched.clone()));
    }
    if !result.app.touched_by_op.is_empty() {
        let by: Map<String, Value> = result
            .app
            .touched_by_op
            .iter()
            .map(|(op, t)| (op.to_string(), Value::Array(t.clone())))
            .collect();
        app.insert("touched_by_op".into(), Value::Object(by));
    }
    if !result.app.changed_parts.is_empty() {
        app.insert("changed_parts".into(), Value::Array(result.app.changed_parts.clone()));
    }
    body.insert("app".into(), Value::Object(app));
    Value::Object(body)
}

/// A read_batch op's path split: its index and the rest (`result.data.ops[2].data.x`:
/// `(2, "data.x")`).
fn op_path(path: &str) -> Option<(usize, &str)> {
    let rest = path.strip_prefix("result.data.ops[")?;
    let (index, rest) = rest.split_once("].")?;
    Some((index.parse().ok()?, rest))
}

/// Two comparisons of one event as one: the first difference, else the known ones of both.
fn both(a: Outcome, b: Outcome) -> Outcome {
    match (a, b) {
        (Outcome::Equal, other) | (other, Outcome::Equal) => other,
        (Outcome::Known(mut x), Outcome::Known(y)) => {
            x.extend(y);
            x.sort_unstable();
            x.dedup();
            Outcome::Known(x)
        }
        (different @ Outcome::Different(..), _) | (_, different) => different,
    }
}

/// A file's parts' digests as an output's are recorded: a file that is no package, its
/// digest under `""`.
fn output_parts(data: &[u8]) -> Value {
    part_digests(data).map_or_else(
        || serde_json::json!({"": hex(&ooxml_base::sha256::sha256(data))}),
        Value::Object,
    )
}

/// An output as the streams hold it (`host_goldens.described_output`): its description less
/// its size, and its parts' digests.
fn described_output(output: &ooxml_session::outputs::Output) -> Value {
    serde_json::json!({"name": output.name, "format": output.format, "doc": output.doc,
        "validate": output.validate, "parts": output_parts(&output.data)})
}

/// A dataset as `freeze_values.py` froze it: its id, columns, rows (values as the adapters
/// hand them over), sources, keys, types and Markdown, of no document.
fn frozen_dataset(given: &Value) -> Option<ooxml_session::datasets::DatasetEntry> {
    let id = given.get("id")?.as_str()?;
    Some(ooxml_session::datasets::DatasetEntry::new(
        id,
        Arc::new(frozen_rows(given)?),
        None,
    ))
}

/// The rows of a frozen dataset or resolver answer (`freeze_values._dataset`).
pub(crate) fn frozen_rows(given: &Value) -> Option<ooxml_tools::datasets::Dataset> {
    use ooxml_tools::datasets::{Cache, Dataset, Markdown};
    use ooxml_tools::sources::Cell;
    let texts =
        |v: &Value| -> Option<Vec<String>> { v.as_array()?.iter().map(|c| Some(c.as_str()?.to_owned())).collect() };
    let columns = texts(given.get("columns")?)?;
    let rows: Option<Vec<Vec<Cell>>> = given
        .get("rows")?
        .as_array()?
        .iter()
        .map(|row| row.as_array()?.iter().map(Cell::from_json).collect())
        .collect();
    let mut dataset = Dataset::new(columns, rows?).ok()?;
    if let Some(sources) = given.get("sources").and_then(Value::as_array) {
        let sources: Option<Vec<Vec<Vec<String>>>> = sources
            .iter()
            .map(|row| row.as_array()?.iter().map(texts).collect())
            .collect();
        dataset = dataset.with_sources(sources?).ok()?;
    }
    let keys = given.get("keys").and_then(Value::as_u64).unwrap_or(0);
    dataset = dataset.with_keys(usize::try_from(keys).ok()?).ok()?;
    if let Some(types) = given.get("types").and_then(Value::as_array) {
        dataset = dataset
            .with_types(types.iter().map(|t| t.as_str().map(str::to_owned)).collect())
            .ok()?;
    }
    let markdown: std::collections::BTreeMap<(usize, usize), String> = given
        .get("markdown")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|m| {
            let at = |k: usize| m.get(k)?.as_u64().and_then(|n| usize::try_from(n).ok());
            Some(((at(0)?, at(1)?), m.get(2)?.as_str()?.to_owned()))
        })
        .collect();
    if !markdown.is_empty() {
        dataset = dataset.with_markdown(Markdown::Given(markdown)).ok()?;
    }
    if let Some(formats) = given.get("formats").and_then(Value::as_array) {
        dataset = dataset
            .with_formats(formats.iter().map(|f| f.as_str().map(str::to_owned)).collect())
            .ok()?;
    }
    if let Some(cache) = given.get("cache").and_then(Value::as_object) {
        let counts: Vec<(&str, i64)> = cache
            .iter()
            .filter_map(|(k, v)| Some((k.as_str(), v.as_i64()?)))
            .collect();
        dataset = dataset.with_cache(Cache::new(&counts).ok()?);
    }
    Some(dataset)
}

/// The formats a frozen answer's toolbox held beside Word's, each as what the call asked
/// of it (its prefixes, the runs of addresses it merged); one per kind the session has.
fn frozen_formats(case: &Value) -> Vec<Arc<dyn ooxml_session::Format>> {
    let mut formats: Vec<Arc<dyn ooxml_session::Format>> = Vec::new();
    let mut kinds: Vec<DocKind> = Vec::new();
    // the formats the toolbox held, a document of theirs open or not
    let registered = case["formats"].as_array().cloned().unwrap_or_default();
    for other in &registered {
        let named = other["kind"].as_str().unwrap_or_default();
        let kind = frozen_kind(named);
        if kind == DocKind::Docx || kinds.contains(&kind) {
            continue;
        }
        kinds.push(kind);
        let merges = case["merges"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|m| m[0] == named)
            .filter_map(|m| {
                let list = |v: &Value| -> Option<Vec<String>> {
                    v.as_array()?.iter().map(|a| Some(a.as_str()?.to_owned())).collect()
                };
                Some((list(&m[1])?, list(&m[2])?))
            })
            .collect();
        let prefixes = other["prefixes"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|p| p.as_str().map(str::to_owned))
            .collect();
        formats.push(Arc::new(crate::frozen::FrozenFormat { kind, prefixes, merges }));
    }
    formats
}

/// The kind a frozen answer's made-up format stands for: its own name where the session
/// has the kind, a grid's as a workbook's, anything else's as a deck's.
fn frozen_kind(kind: &str) -> DocKind {
    match kind {
        "docx" => DocKind::Docx,
        "xlsx" | "grid" => DocKind::Xlsx,
        _ => DocKind::Pptx,
    }
}

/// The parts `now` has that `then` has otherwise or not at all (each with its bytes), and
/// those it lacks (`None`).
fn changed_parts(then: &[u8], now: &[u8]) -> Vec<(String, Option<Arc<[u8]>>)> {
    let read = |data: &[u8]| -> BTreeMap<String, Vec<u8>> {
        ooxml_zip::Archive::read(Arc::from(data), &ooxml_zip::Limits::default())
            .map(|archive| {
                archive
                    .entries()
                    .iter()
                    .filter_map(|e| Some((e.meta().name.clone(), archive.read_entry(e).ok()?)))
                    .collect()
            })
            .unwrap_or_default()
    };
    let (a, b) = (read(then), read(now));
    let mut out: Vec<(String, Option<Arc<[u8]>>)> = b
        .iter()
        .filter(|(name, bytes)| a.get(*name) != Some(*bytes))
        .map(|(name, bytes)| (name.clone(), Some(Arc::from(bytes.as_slice()))))
        .collect();
    out.extend(
        a.keys()
            .filter(|name| !b.contains_key(*name))
            .map(|name| (name.clone(), None)),
    );
    out
}

/// An empty package's bytes.
fn empty_package() -> Arc<[u8]> {
    let mut writer = ooxml_zip::Writer::new();
    let entry = ooxml_zip::NewEntry {
        name: "[Content_Types].xml".into(),
        method: ooxml_zip::Method::Deflated,
        dos_time: 0,
        dos_date: 33,
        external_attr: 0,
        create_system: 3,
    };
    let _ = writer.add(&entry, b"<Types/>");
    writer
        .finish(b"")
        .map(Arc::from)
        .unwrap_or_else(|_| Arc::from(&b""[..]))
}

/// The roles a settings value names.
fn roles_of(value: &Value) -> Option<RoleSet> {
    let names: Vec<&str> = value.as_array()?.iter().filter_map(Value::as_str).collect();
    RoleSet::parse(names).ok()
}

/// A locale as the recorder writes one (ooxml-common's `Locale`, its fields as given, the
/// rest its defaults: en-US's).
fn locale_of(value: &Value) -> Option<ooxml_numfmt::Locale> {
    let given = value.as_object()?;
    let mut locale = ooxml_numfmt::EN_US;
    let one = |key: &str| -> Option<Option<char>> {
        match given.get(key) {
            None => Some(None),
            Some(v) => v.as_str()?.chars().next().map(Some),
        }
    };
    if let Some(c) = one("decimal_separator")? {
        locale.decimal_separator = c;
    }
    if let Some(c) = one("thousands_separator")? {
        locale.thousands_separator = c;
    }
    if let Some(c) = one("list_separator")? {
        locale.list_separator = c;
    }
    if let Some(order) = given.get("date_order") {
        locale.date_order = order.as_str()?.to_owned().into();
    }
    if let Some(language) = given.get("language") {
        locale.language = language.as_str()?.to_owned().into();
    }
    Some(locale)
}

/// The host's settings of a recorded `limits` (ooxml-edit's `Limits`, the fields that differ
/// from its defaults); a batch's own limits are lane D2's.
fn limits(settings: &mut HostSettings, value: &Value) -> Result<(), Waits> {
    let given = value.as_object().ok_or(Waits::Settings)?;
    let count = |v: &Value| v.as_u64().and_then(|n| usize::try_from(n).ok());
    for (key, v) in given {
        match key.as_str() {
            "max_result_chars" => settings.limits.result_chars = count(v).ok_or(Waits::Settings)?,
            "max_list_items" => settings.limits.list_items = count(v).ok_or(Waits::Settings)?,
            "max_images_per_call" => settings.limits.images_per_call = count(v).ok_or(Waits::Settings)?,
            "max_read_batch_chars" => settings.read_batch_chars = count(v).ok_or(Waits::Settings)?,
            "image_budget" => {
                settings.image_budget = if v.is_null() {
                    None
                } else {
                    Some(count(v).ok_or(Waits::Settings)?)
                }
            }
            "image_budget_per_round" => {
                settings.image_budget_per_round = if v.is_null() {
                    None
                } else {
                    Some(count(v).ok_or(Waits::Settings)?)
                };
            }
            "layout_timeout" => {
                let seconds = v.as_f64().filter(|s| *s >= 0.0).ok_or(Waits::Settings)?;
                settings.layout_timeout = Some(Duration::from_secs_f64(seconds));
            }
            "max_batch_ops" => settings.max_batch_ops = count(v).ok_or(Waits::Settings)?,
            "batch_timeout" => {
                let seconds = v.as_f64().filter(|s| *s >= 0.0).ok_or(Waits::Settings)?;
                settings.batch_timeout = Duration::from_secs_f64(seconds);
            }
            _ => return Err(Waits::Settings),
        }
    }
    Ok(())
}

struct Fixed(i64);

impl Clock for Fixed {
    fn now(&self) -> Timestamp {
        Timestamp {
            seconds: self.0,
            nanos: 0,
        }
    }
    fn monotonic(&self) -> Duration {
        Duration::ZERO
    }
}

struct NoFonts;

impl FontSource for NoFonts {
    fn face(&self, _: &FaceRequest) -> Result<Option<FaceData>, BaseError> {
        Ok(None)
    }
}

struct NoParts;

impl PartStore for NoParts {
    fn load(&self, _: &str) -> Result<Option<Arc<[u8]>>, BaseError> {
        Ok(None)
    }
    fn store(&self, _: &str, _: Arc<[u8]>) -> Result<(), BaseError> {
        Ok(())
    }
}

struct Unlimited;

impl CacheBudget for Unlimited {
    fn reserve(&self, _: usize) -> bool {
        true
    }
    fn release(&self, _: usize) {}
    fn used(&self) -> usize {
        0
    }
}

struct Never;

impl Cancel for Never {
    fn is_cancelled(&self) -> bool {
        false
    }
}

/// Seconds since the epoch of an ISO 8601 moment in UTC (`2026-01-05T09:00:00+00:00`).
fn epoch(text: &str) -> Option<i64> {
    let digits = |from: usize, to: usize| text.get(from..to)?.parse::<i64>().ok();
    let (year, month, day) = (digits(0, 4)?, digits(5, 7)?, digits(8, 10)?);
    let (hour, minute, second) = (digits(11, 13)?, digits(14, 16)?, digits(17, 19)?);
    // days from civil (H. Hinnant)
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(days * 86_400 + hour * 3_600 + minute * 60 + second)
}

/// The replayer: the repository, the goldens' folder, and the faces every layout finds.
pub struct Replayer {
    root: PathBuf,
    host: PathBuf,
    fonts: Arc<FontSetup>,
    raster: Arc<PageRaster>,
    known: Vec<Known>,
}

/// One case's events and how each replayed.
pub type Replayed = Vec<(Value, Outcome)>;

impl Replayer {
    /// A replayer of the goldens in `root`'s `crates/ooxml-oracle/host`.
    ///
    /// # Errors
    /// The known differences' file unreadable.
    pub fn new(root: &Path) -> Result<Replayer, String> {
        let host = root.join("crates/ooxml-oracle/host");
        let faces = root.join("python/pptx2svg/packages/pptx2svg-fonts/src/pptx2svg_fonts/files");
        let folder = Folder {
            location: Location::Folder,
            path: faces.to_string_lossy().into_owned(),
        };
        let fonts = Arc::new(FontSetup::new(
            Arc::new(DiskFonts),
            std::slice::from_ref(&folder),
            WordFontOptions::default(),
        ));
        let raster = Arc::new(PageRaster::new(&DiskFonts, None, (&[], &[folder.path]), None));
        let text = std::fs::read_to_string(host.join("known-differences.toml")).map_err(|e| e.to_string())?;
        Ok(Replayer {
            root: root.to_owned(),
            known: known_entries(&text)?,
            host,
            fonts,
            raster,
        })
    }

    /// The known differences.
    #[must_use]
    pub fn known(&self) -> &[Known] {
        &self.known
    }

    /// The docx streams, by name (`docx/tests/test_x`), in order.
    #[must_use]
    pub fn streams(&self) -> Vec<(String, PathBuf)> {
        let mut out = Vec::new();
        for kind in ["tests", "transcripts", "features"] {
            let dir = self.host.join("docx").join(kind);
            let Ok(found) = std::fs::read_dir(&dir) else { continue };
            for entry in found.flatten() {
                let path = entry.path();
                if path.extension().is_some_and(|e| e == "jsonl") {
                    let stem = path
                        .file_stem()
                        .map(|s| s.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    out.push((format!("docx/{kind}/{stem}"), path));
                }
            }
        }
        out.sort();
        out
    }

    /// `path`'s cases, each its events in order.
    ///
    /// # Errors
    /// A line that is not JSON.
    pub fn cases(path: &Path) -> Result<Vec<(String, Vec<Value>)>, String> {
        let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
        let mut out: Vec<(String, Vec<Value>)> = Vec::new();
        for line in text.lines().filter(|l| !l.trim().is_empty()) {
            let event: Value = serde_json::from_str(line).map_err(|e| e.to_string())?;
            let case = event.get("case").and_then(Value::as_str).unwrap_or_default().to_owned();
            match out.iter_mut().find(|(c, _)| *c == case) {
                Some((_, events)) => events.push(event),
                None => out.push((case, vec![event])),
            }
        }
        Ok(out)
    }

    fn input(&self, spec: &Value) -> Option<Arc<[u8]>> {
        let path = spec.get("path")?.as_str()?;
        let real = if let Some(rest) = path.strip_prefix("$ROOT/") {
            self.root.join(rest)
        } else if let Some(rest) = path.strip_prefix("$HOST/") {
            self.host.join(rest)
        } else {
            PathBuf::from(path)
        };
        std::fs::read(real).ok().map(Arc::from)
    }

    /// A session for a case whose head is `head`: the toolbox and session settings it gives
    /// (roles, locale, limits, Word's `min_text_size`; `Settings` for any other), its clock;
    /// Word's format laid out with the repository's open faces (a bench's host too).
    ///
    /// # Errors
    /// What it waits for: a setting the replay does not make yet.
    pub fn session(&self, head: &Value) -> Result<HostSession, Waits> {
        self.session_with(head, Vec::new())
    }

    /// [`Replayer::session`] with `formats` beside Word's.
    fn session_with(&self, head: &Value, formats: Vec<Arc<dyn ooxml_session::Format>>) -> Result<HostSession, Waits> {
        let mut settings = HostSettings::default();
        let given = |which: &str| head.get(which).and_then(Value::as_object).cloned().unwrap_or_default();
        let (toolbox, session) = (given("toolbox"), given("session"));
        let mut locale = None;
        let mut min_text_size = None;
        for (key, value) in toolbox.iter().chain(session.iter()) {
            match key.as_str() {
                // (block_edits: when the Python host drafts an edit beside another; the Rust
                // host's store runs every edit side by side)
                "clock" | "font_dirs" | "block_edits" => {}
                "roles" => settings.roles = roles_of(value).ok_or(Waits::Settings)?,
                "locale" => locale = Some(locale_of(value).ok_or(Waits::Settings)?),
                "limits" => limits(&mut settings, value)?,
                // docx_format(min_text_size=...): Word's format's setting
                "min_text_size" => {
                    min_text_size = ooxml_tools_word::minimum_of(value).map_err(|_| Waits::Settings)?;
                }
                _ => return Err(Waits::Settings),
            }
        }
        let clock = session
            .get("clock")
            .and_then(Value::as_str)
            .and_then(epoch)
            .unwrap_or(1_767_603_600);
        let pages = SessionPages {
            setup: PageSetup::new(Arc::clone(&self.fonts), EngineOptions::default()),
            memory: MemoryBudget::new(256 << 20),
            raster: Some(Arc::clone(&self.raster)),
        };
        let format = WordFormat {
            pages: Some(pages),
            min_text_size,
        };
        let mut all: Vec<Arc<dyn ooxml_session::Format>> = vec![Arc::new(format)];
        all.extend(formats);
        let host = Arc::new(Host::new(all, settings, Arc::new(HostTools)));
        let context = Context {
            fonts: Arc::new(NoFonts),
            parts: Arc::new(NoParts),
            clock: Arc::new(Fixed(clock)),
            budget: Arc::new(Unlimited),
            cancel: Arc::new(Never),
            limits: ooxml_base::Limits::default(),
            locale: ooxml_base::Locale::default(),
        };
        let config = SessionConfig {
            store: StoreConfig::default(),
            locale,
        };
        Ok(host.session(config, context))
    }

    /// `events` (one case, its head first) made again.
    #[must_use]
    pub fn replay(&self, stream: &str, events: &[Value]) -> Replayed {
        let events = expected_of(events);
        let Some((head, rest)) = events.split_first() else {
            return Vec::new();
        };
        let session = match self.session(head) {
            Ok(session) => session,
            Err(waits) => {
                return rest
                    .iter()
                    .map(|e| (e.clone(), Outcome::Waiting(waits, false)))
                    .collect()
            }
        };
        let mut blocked: Option<Waits> = None;
        let mut out = Vec::new();
        for event in rest {
            let outcome = match blocked {
                Some(waits) => Outcome::Waiting(waits, true),
                None => {
                    let (outcome, blocks) = self.event(stream, &session, event);
                    if blocks {
                        if let Outcome::Waiting(waits, _) = outcome {
                            blocked = Some(waits);
                        }
                    }
                    outcome
                }
            };
            out.push((event.clone(), outcome));
        }
        out
    }

    /// A frozen answer of the Python host (`frozen/values.jsonl`, `freeze_values.py`)
    /// made again: the document opened as it was before the call, the session's datasets
    /// kept as they were, the call made; its reply and the document's parts after it compared
    /// with the frozen ones. A case the freezer skipped waits, with why.
    #[must_use]
    pub fn frozen(&self, case: &Value) -> Outcome {
        self.frozen_in("frozen/values", case)
    }

    /// [`Replayer::frozen`] of a case of the frozen answers `stream` (the known differences'
    /// stream it is matched as), the session's rules for numbers made where it froze them.
    pub(crate) fn frozen_in(&self, stream: &str, case: &Value) -> Outcome {
        if case.get("skipped").is_some() {
            return Outcome::Waiting(Waits::Settings, false);
        }
        let mut given = Map::new();
        for key in ["clock", "locale"] {
            if let Some(value) = case.get(key).filter(|v| !v.is_null()) {
                given.insert(key.to_owned(), value.clone());
            }
        }
        let head = serde_json::json!({"session": given});
        let others = case["others"].as_array().cloned().unwrap_or_default();
        let formats = frozen_formats(case);
        let Ok(session) = self.session_with(&head, formats) else {
            return Outcome::Waiting(Waits::Settings, false);
        };
        if let Some(rules) = case
            .get("numbers")
            .filter(|r| r.as_object().is_some_and(|r| !r.is_empty()))
        {
            match ooxml_session::numbers::NumberRules::from_json(rules) {
                Ok(rules) => session.set_number_rules(rules),
                Err(_) => return Outcome::Waiting(Waits::Settings, false),
            }
        }
        if let Err(why) = self.open_others(&session, &others) {
            return Outcome::Different(vec!["others".into()], Value::Array(others), Value::from(why));
        }
        let document = &case["document"];
        let arguments = case.get("arguments").cloned().unwrap_or(Value::Null);
        let doc = arguments.get("doc").and_then(Value::as_str).unwrap_or("d1");
        let Some(bytes) = document.get("input").and_then(|i| self.input(i)) else {
            return Outcome::Different(vec!["document.input".into()], document.clone(), Value::Null);
        };
        if let Err(why) = self.open_frozen(&session, case, doc, bytes) {
            return Outcome::Different(vec!["document".into()], document.clone(), Value::from(why));
        }
        let entries: Option<Vec<_>> = case["datasets"]
            .as_array()
            .into_iter()
            .flatten()
            .map(frozen_dataset)
            .collect();
        let Some(entries) = entries else {
            return Outcome::Different(vec!["datasets".into()], case["datasets"].clone(), Value::Null);
        };
        if let Err(error) = session.keep_datasets(entries) {
            return Outcome::Different(vec!["datasets".into()], Value::Null, error.to_json());
        }
        let tool = case.get("tool").and_then(Value::as_str).unwrap_or_default();
        let Ok(result) = session.call(tool, arguments.clone(), &CallAs::default()) else {
            return Outcome::Waiting(Waits::Settings, false);
        };
        // (its test as the case a known difference names)
        let mut case = case.clone();
        let test = case.get("test").cloned();
        if let (Value::Object(map), Some(test)) = (&mut case, test) {
            map.insert("case".into(), test);
        }
        let case = &case;
        let replied = self.compare(stream, case, "result", &normalised(&result));
        let after = DocId::parse(doc)
            .and_then(|id| session.save(&id).ok())
            .and_then(|saved| part_digests(&saved.data))
            .map_or(Value::Null, Value::Object);
        both(replied, self.compare(stream, case, "after", &after))
    }

    /// A frozen answer's document opened as it was before the call: its bytes and id state
    /// then; for a call that removes rows -- which says what it removes of what the document
    /// was when first opened (`original_removed`) -- opened as it was first opened and brought to
    /// the version the call read as one step, numbered as the Python host numbered it (the
    /// ids drawn in between are not drawn again); its tracking set as the test set it.
    fn open_frozen(&self, session: &HostSession, case: &Value, doc: &str, now: Arc<[u8]>) -> Result<(), String> {
        let document = &case["document"];
        let name = document.get("name").and_then(Value::as_str).unwrap_or("document.docx");
        let reads_first = case.pointer("/arguments/remove") == Some(&Value::Bool(true));
        let opened = case.get("opened").filter(|o| !o.is_null() && reads_first);
        let first = match opened {
            Some(o) => o
                .get("input")
                .and_then(|i| self.input(i))
                .ok_or("the first version's input not found")?,
            None => Arc::clone(&now),
        };
        let state = match opened {
            Some(o) => o.get("state").and_then(Value::as_str),
            None => document.get("state").and_then(Value::as_str),
        };
        let how = Opening {
            kind: Some(DocKind::Docx),
            source: Some("app"),
            doc_id: Some(doc),
            state,
        };
        let id = session
            .open_with(Arc::clone(&first), name, &how)
            .map_err(|e| e.to_string())?;
        let version = document.get("version").and_then(Value::as_u64).unwrap_or(0);
        let parts = changed_parts(&first, &now);
        if !parts.is_empty() && version > 0 {
            session
                .session()
                .continue_numbers(&id, version - 1, version - 1)
                .map_err(|e| e.to_string())?;
            session
                .session()
                .update(&id, parts, &ooxml_session::CallContext::default())
                .map_err(|e| e.to_string())?;
        }
        if let Some(tracking) = case.get("tracking").filter(|t| !t.is_null()) {
            let reply = session
                .call("word_set_tracking", tracking.clone(), &CallAs::default())
                .map_err(|_| "tracking not set".to_owned())?;
            if reply.is_error() {
                return Err(format!("tracking not set: {:?}", reply.to_json()));
            }
        }
        Ok(())
    }

    /// A frozen answer's other open documents opened under their handles: a Word document
    /// from its bytes, another as what its format answered.
    fn open_others(&self, session: &HostSession, others: &[Value]) -> Result<(), String> {
        for other in others {
            let handle = other["doc"].as_str().unwrap_or_default();
            let name = other["name"].as_str().unwrap_or("document");
            let kind = frozen_kind(other["kind"].as_str().unwrap_or_default());
            if kind == DocKind::Docx {
                let bytes = other
                    .get("input")
                    .and_then(|i| self.input(i))
                    .ok_or("an input not found")?;
                let how = Opening {
                    kind: Some(DocKind::Docx),
                    source: Some("app"),
                    doc_id: Some(handle),
                    state: other.get("state").and_then(Value::as_str),
                };
                session.open_with(bytes, name, &how).map_err(|e| e.to_string())?;
                continue;
            }
            let id = DocId::parse(handle).ok_or("a handle that is no id")?;
            let package =
                ooxml_opc::Package::open(empty_package(), &ooxml_base::Limits::default()).map_err(|e| e.to_string())?;
            let env = session.session().open_env();
            let inner = ooxml_session::Format::open(&ooxml_session::PackageFormat(kind), package, &env)
                .map_err(|e| e.to_string())?;
            let answers = other["answers"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|a| {
                    Some((
                        a[0].as_str()?.to_owned(),
                        crate::frozen::FrozenDoc::answer(&a[1], frozen_rows),
                    ))
                })
                .collect();
            let value_sources = other["value_sources"]
                .as_array()
                .map(|v| v.iter().filter_map(|s| s.as_str().map(str::to_owned)).collect());
            let doc = crate::frozen::FrozenDoc {
                inner,
                answers,
                value_sources,
            };
            session
                .session()
                .insert_as(kind, name, Arc::new(doc), Some(id))
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    /// One event made again: how it went, and whether the case's later events depend on it.
    fn event(&self, stream: &str, session: &HostSession, event: &Value) -> (Outcome, bool) {
        let kind = event.get("event").and_then(Value::as_str).unwrap_or_default();
        match kind {
            "open" => (self.open(stream, session, event), false),
            "blob" => {
                let bytes = event.get("input").and_then(|i| self.input(i));
                let name = event.get("name").and_then(Value::as_str).unwrap_or("blob");
                let mime = event.get("mime").and_then(Value::as_str);
                match bytes.map(|b| session.add_blob(b, name, mime)) {
                    Some(Ok(handle)) => (self.compare(stream, event, "returned", &Value::from(handle)), false),
                    _ => (Outcome::Waiting(Waits::D3, true), true),
                }
            }
            "close" => {
                let doc = event.get("doc").and_then(Value::as_str).and_then(DocId::parse);
                match doc.map(|d| session.close(&d)) {
                    Some(Ok(kept)) => (self.compare(stream, event, "returned", &Value::from(kept)), false),
                    _ => (Outcome::Waiting(Waits::D3, true), true),
                }
            }
            "new_round" => {
                session.new_round();
                (Outcome::Equal, false)
            }
            "save" | "undo" | "redo" => self.history(stream, session, event, kind),
            "call" | "change" => self.call(stream, session, event),
            "final" => {
                let mut documents = Map::new();
                for id in session.session().documents() {
                    let parts = session.session().save(&id).ok().and_then(|bytes| part_digests(&bytes));
                    documents.insert(id.to_string(), parts.map_or(Value::Null, Value::Object));
                }
                (
                    self.compare(stream, event, "documents", &Value::Object(documents)),
                    false,
                )
            }
            "calls" => (Outcome::Waiting(Waits::D2, true), true),
            "settings" => self.settings(session, event),
            _ => (Outcome::Waiting(Waits::Settings, true), true),
        }
    }

    /// The application's settings changed between calls: the session's roles (the rest
    /// the Rust host takes per host, not per session).
    fn settings(&self, session: &HostSession, event: &Value) -> (Outcome, bool) {
        let given = event.get("session").and_then(Value::as_object);
        let toolbox = event.get("toolbox").and_then(Value::as_object);
        if toolbox.is_some_and(|t| !t.is_empty()) {
            return (Outcome::Waiting(Waits::Settings, true), true);
        }
        for (key, value) in given.into_iter().flatten() {
            match (key.as_str(), roles_of(value)) {
                ("roles", Some(roles)) => session.set_roles(roles),
                _ => return (Outcome::Waiting(Waits::Settings, true), true),
            }
        }
        (Outcome::Equal, false)
    }

    /// The application's open, made again.
    fn open(&self, stream: &str, session: &HostSession, event: &Value) -> Outcome {
        let Some(bytes) = event.get("input").and_then(|i| self.input(i)) else {
            return Outcome::Waiting(Waits::Settings, true);
        };
        let text = |key: &str| event.get(key).and_then(Value::as_str);
        let name = text("name").unwrap_or("document.docx");
        let how = Opening {
            kind: text("kind").and_then(DocKind::parse),
            source: text("source"),
            doc_id: text("doc_id"),
            state: text("state"),
        };
        let have = match session.open_with(bytes, name, &how) {
            Ok(id) => (Some(Value::from(id.to_string())), None),
            Err(refused) => (None, Some(refused.to_string())),
        };
        self.compare_app(stream, event, have)
    }

    /// The application's save, undo or redo (`kind`), made again: how it went, and whether
    /// the case's later events depend on it.
    fn history(&self, stream: &str, session: &HostSession, event: &Value, kind: &str) -> (Outcome, bool) {
        let Some(doc) = event.get("doc").and_then(Value::as_str).and_then(DocId::parse) else {
            return (Outcome::Waiting(Waits::Settings, true), true);
        };
        if kind == "save" {
            let outcome = match session.save(&doc) {
                Ok(saved) => {
                    let have = serde_json::json!({"doc": saved.doc, "name": saved.name,
                        "kind": saved.kind.as_str(), "version": saved.version,
                        "parts": part_digests(&saved.data), "state": saved.state});
                    self.compare(stream, event, "saved", &have)
                }
                Err(error) => self.compare_app(stream, event, (None, Some(error.to_string()))),
            };
            return (outcome, false);
        }
        if event.get("scope").is_some_and(|s| !s.is_null()) {
            return (Outcome::Waiting(Waits::Settings, true), true);
        }
        let steps = event
            .get("steps")
            .and_then(Value::as_u64)
            .and_then(|n| usize::try_from(n).ok())
            .unwrap_or(1);
        let caller = event
            .get("view")
            .and_then(|v| v.get("context"))
            .and_then(Value::as_object);
        let have = match session.history(&doc, steps, kind == "redo", caller) {
            Ok(count) => (Some(Value::from(count)), None),
            Err(error) => (None, Some(error.to_string())),
        };
        (self.compare_app(stream, event, have), false)
    }

    fn call(&self, stream: &str, session: &HostSession, event: &Value) -> (Outcome, bool) {
        let tool = event.get("tool").and_then(Value::as_str).unwrap_or_default();
        let arguments = event.get("arguments").cloned().unwrap_or(Value::Null);
        let roles = event
            .get("roles")
            .or_else(|| event.get("view").and_then(|v| v.get("roles")))
            .and_then(Value::as_array)
            .map(|names| RoleSet::parse(names.iter().filter_map(Value::as_str)));
        let roles = match roles {
            Some(Ok(set)) => Some(set),
            Some(Err(_)) => return (Outcome::Waiting(Waits::Settings, true), true),
            None => None,
        };
        let mut context: Map<String, Value> = Map::new();
        for found in [event.get("view").and_then(|v| v.get("context")), event.get("context")]
            .into_iter()
            .flatten()
        {
            if let Some(map) = found.as_object() {
                context.extend(map.clone());
            }
        }
        let how = CallAs {
            roles,
            context: (!context.is_empty()).then_some(context),
            ..CallAs::default()
        };
        match session.call(tool, arguments, &how) {
            Ok(result) if event.get("event").and_then(Value::as_str) == Some("change") => {
                let have = serde_json::json!({"ok": !result.is_error(), "changed_parts": result.app.changed_parts});
                (self.compare(stream, event, "result", &have), false)
            }
            Ok(result) => {
                let replied = self.compare(stream, event, "result", &normalised(&result));
                let outputs: Vec<Value> = session.take_outputs().iter().map(described_output).collect();
                let saved = if event.get("outputs").is_some() {
                    self.compare(stream, event, "outputs", &Value::Array(outputs))
                } else {
                    Outcome::Equal
                };
                (both(replied, saved), false)
            }
            Err(_) => (Outcome::Waiting(Waits::Settings, true), true),
        }
    }

    /// An application call's answer (`returned`, else what it raised: its message) against
    /// the recorded one.
    fn compare_app(&self, stream: &str, event: &Value, have: (Option<Value>, Option<String>)) -> Outcome {
        match (event.get("raised").and_then(Value::as_str), have) {
            (Some(raised), (_, Some(message))) => {
                let want = raised.split_once(": ").map_or(raised, |(_, m)| m);
                let mut event = event.clone();
                if let Value::Object(map) = &mut event {
                    map.insert("raised".into(), Value::from(want));
                }
                self.compare(stream, &event, "raised", &Value::from(message))
            }
            (Some(raised), (returned, None)) => Outcome::Different(
                vec!["raised".into()],
                Value::from(raised),
                returned.unwrap_or(Value::Null),
            ),
            (None, (_, Some(message))) => Outcome::Different(
                vec!["returned".into()],
                event.get("returned").cloned().unwrap_or(Value::Null),
                Value::from(message),
            ),
            (None, (returned, None)) => self.compare(stream, event, "returned", &returned.unwrap_or(Value::Null)),
        }
    }

    /// `have` against the event's recorded `field`, with the known differences admitted.
    pub(crate) fn compare(&self, stream: &str, event: &Value, field: &str, have: &Value) -> Outcome {
        let want = event.get(field).cloned().unwrap_or(Value::Null);
        let paths = differing(&want, have, field);
        if paths.is_empty() {
            return Outcome::Equal;
        }
        let case = event.get("case").and_then(Value::as_str).unwrap_or_default();
        let tool = event
            .get("tool")
            .and_then(Value::as_str)
            .map(|t| t.rsplit('.').next().unwrap_or(t));
        let mut used = Vec::new();
        let mut left: Vec<String> = Vec::new();
        for path in paths {
            // a read_batch op's path is admitted as its own call's would be
            let own = op_path(&path).map_or_else(|| path.clone(), |(_, rest)| format!("result.{rest}"));
            let under = |f: &String| {
                [&path, &own]
                    .iter()
                    .any(|p| **p == *f || p.starts_with(&format!("{f}.")) || p.starts_with(&format!("{f}[")))
            };
            let admitted = self.known.iter().position(|entry| {
                entry.stream.as_deref().is_none_or(|g| glob(g, stream))
                    && entry.case.as_deref().is_none_or(|g| glob(g, case))
                    && entry.tool.as_deref().is_none_or(|t| Some(t) == tool)
                    && entry.fields.iter().any(under)
            });
            match admitted {
                Some(index) => used.push(index),
                None => left.push(path),
            }
        }
        if left.is_empty() {
            used.sort_unstable();
            used.dedup();
            return Outcome::Known(used);
        }
        let (a, b) = (leaves(&want, field), leaves(have, field));
        let pick = |m: &BTreeMap<String, Value>| -> Value {
            Value::Object(
                left.iter()
                    .take(5)
                    .map(|p| (p.clone(), m.get(p).cloned().unwrap_or(Value::Null)))
                    .collect(),
            )
        };
        let (want, have) = (pick(&a), pick(&b));
        Outcome::Different(left, want, have)
    }
}

/// A concurrent case's changes in the order they were committed (`_commit_order`): grouped
/// by overlap of their spans, a group that ended before the next began first, within a group
/// by the version each left.
/// A change's place among a case's changes: its span's start and end, the version it left
/// and its position, and the change itself.
type Placed = ((i64, i64, i64, usize), Value);

fn commit_order(mut changes: Vec<Placed>) -> Vec<Value> {
    changes.sort_by_key(|(k, _)| (k.0, k.3));
    let mut groups: Vec<Vec<Placed>> = Vec::new();
    let mut end: Option<i64> = None;
    for item in changes {
        match end {
            Some(e) if item.0 .0 <= e => {
                end = Some(e.max(item.0 .1));
                if let Some(group) = groups.last_mut() {
                    group.push(item);
                }
            }
            _ => {
                end = Some(item.0 .1);
                groups.push(vec![item]);
            }
        }
    }
    let mut out = Vec::new();
    for mut group in groups {
        group.sort_by_key(|(k, _)| (k.2, k.3));
        out.extend(group.into_iter().map(|(_, change)| change));
    }
    out
}

/// What a replay of a case must give (`host_goldens.expected_of`): a case of calls on one
/// thread, its lines; a concurrent one (calls from several threads at once), its calls that
/// changed something as `change` events in commit order (each answer its `ok` and
/// `changed_parts`), in the slots the calls took among the application's own events, and its
/// `final` state -- what a serial replay in commit order must give.
#[must_use]
pub fn expected_of(lines: &[Value]) -> Vec<Value> {
    let Some((head, rest)) = lines.split_first() else {
        return Vec::new();
    };
    if head.get("concurrent") != Some(&Value::Bool(true)) {
        return lines.to_vec();
    }
    let changing = |result: Option<&Value>| {
        result.is_some_and(|r| {
            r.get("ok") == Some(&Value::Bool(true))
                && r.pointer("/app/changed_parts")
                    .and_then(Value::as_array)
                    .is_some_and(|a| !a.is_empty())
        })
    };
    let mut changes = Vec::new();
    let mut shape: Vec<Option<Value>> = Vec::new();
    for event in rest {
        let items: Vec<(Value, Value, Option<Value>)> = match event.get("event").and_then(Value::as_str) {
            Some("call") => vec![(
                event["tool"].clone(),
                event["arguments"].clone(),
                event.get("result").cloned(),
            )],
            Some("calls") => {
                let calls = event
                    .get("calls")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                let results = event
                    .get("results")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                calls
                    .into_iter()
                    .zip(results)
                    .map(|(c, r)| {
                        let part = |k: usize| c.get(k).cloned().unwrap_or(Value::Null);
                        (part(0), part(1), Some(r))
                    })
                    .collect()
            }
            _ => {
                shape.push(Some(event.clone()));
                continue;
            }
        };
        for (tool, arguments, result) in items {
            if changing(result.as_ref()) {
                changes.push(change_of(event, tool, arguments, result.as_ref(), changes.len()));
                shape.push(None);
            }
        }
    }
    let mut ordered = commit_order(changes).into_iter();
    let case = head.get("case").cloned().unwrap_or(Value::Null);
    let mut out = vec![head.clone()];
    for (index, event) in shape.into_iter().enumerate() {
        let mut body: Map<String, Value> = match event {
            None => ordered.next().and_then(|v| v.as_object().cloned()).unwrap_or_default(),
            Some(event) => event
                .as_object()
                .map(|m| {
                    m.iter()
                        .filter(|(k, _)| *k != "case" && *k != "at")
                        .map(|(k, v)| (k.clone(), v.clone()))
                        .collect()
                })
                .unwrap_or_default(),
        };
        let mut line = Map::new();
        line.insert("case".into(), case.clone());
        line.insert("at".into(), Value::from(index + 1));
        line.append(&mut body);
        out.push(Value::Object(line));
    }
    out
}

/// A package's parts by name, each the sha256 of its bytes, or, for an embedded package,
/// `package:` and the digest of its own parts' digests (`host_goldens.part_digests`).
#[must_use]
pub fn part_digests(data: &[u8]) -> Option<Map<String, Value>> {
    let archive = ooxml_zip::Archive::read(Arc::from(data), &ooxml_zip::Limits::default()).ok()?;
    let mut out = BTreeMap::new();
    for entry in archive.entries() {
        let body = archive.read_entry(entry).ok()?;
        out.insert(entry.meta().name.clone(), Value::from(part_digest(&body)));
    }
    Some(out.into_iter().collect())
}

fn part_digest(body: &[u8]) -> String {
    if body.starts_with(b"PK\x03\x04") {
        if let Some(inner) = part_digests(body).filter(|d| d.contains_key("[Content_Types].xml")) {
            // canonical: keys sorted, compact
            let text = Value::Object(inner).to_string();
            return format!("package:{}", hex(&ooxml_base::sha256::sha256(text.as_bytes())));
        }
    }
    hex(&ooxml_base::sha256::sha256(body))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The `change` event of a concurrent case's changing call, placed for [`commit_order`]
/// (`n`: the changes before it).
fn change_of(event: &Value, tool: Value, arguments: Value, result: Option<&Value>, n: usize) -> Placed {
    let mut change = Map::new();
    change.insert("event".into(), Value::from("change"));
    change.insert("tool".into(), tool);
    change.insert("arguments".into(), arguments);
    let parts = result.and_then(|r| r.pointer("/app/changed_parts")).cloned();
    change.insert("result".into(), serde_json::json!({"ok": true, "changed_parts": parts}));
    for key in ["view", "roles", "context"] {
        if let Some(v) = event.get(key) {
            change.insert(key.into(), v.clone());
        }
    }
    let k = i64::try_from(n).unwrap_or(i64::MAX);
    let span = event.get("span").and_then(Value::as_array);
    let at = |i: usize| span.and_then(|s| s.get(i)).and_then(Value::as_i64).unwrap_or(k);
    let version = result
        .and_then(|r| r.get("version"))
        .and_then(Value::as_i64)
        .unwrap_or(0);
    ((at(0), at(1), version, n), Value::Object(change))
}
