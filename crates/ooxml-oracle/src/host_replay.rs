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
use ooxml_host::{CallAs, Host, HostSession, HostSettings, NotYet};
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
    /// check's numbers and scales: Phase 3 G1.
    G1,
    /// A host setting the replay does not make yet (limits, a toolbox's own settings).
    Settings,
    /// What docx-agent's adapters do to a Word call's arguments before the session sees
    /// them (`docx_agent/rust/served`: refs inside a place, `after:$x`, a span placed by its
    /// end; a blob's text as the Markdown; a deck's parameters refused): left where it is in
    /// D1 (`blocks_at`), ported with the session tools.
    Adapters,
}

impl Waits {
    /// Its name in the report.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Waits::D3 => "D3 (session tools)",
            Waits::D2 => "D2 (batch)",
            Waits::G1 => "G1 (numbers, scales)",
            Waits::Settings => "settings",
            Waits::Adapters => "D3 (docx adapters)",
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
    /// The user decision it is, where it is one (`structured-problems-2026-11-25`).
    pub decision: Option<String>,
    /// Why.
    pub cause: String,
}

/// The user decisions an entry may give instead of a rule.
pub const DECISIONS: [&str; 1] = ["structured-problems-2026-11-25"];

/// The known differences in `text` (the TOML subset the file uses: `[[difference]]` tables
/// of strings, integers and lists of strings).
///
/// # Errors
/// A line it cannot read, or an entry without a cause, fields, or an allowed rule or
/// decision.
pub fn known_entries(text: &str) -> Result<Vec<Known>, String> {
    let mut out: Vec<Known> = Vec::new();
    for (number, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line == "[[difference]]" {
            out.push(Known::default());
            continue;
        }
        let (key, value) = line
            .split_once('=')
            .ok_or_else(|| format!("line {}: not key = value", number + 1))?;
        let entry = out
            .last_mut()
            .ok_or_else(|| format!("line {}: outside [[difference]]", number + 1))?;
        let value: Value = serde_json::from_str(value.trim()).map_err(|e| format!("line {}: {e}", number + 1))?;
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
            other => return Err(format!("line {}: no key {other}", number + 1)),
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

/// The tools the Rust host runs through the session-tool seam until D3.
const SESSION_TOOLS: [&str; 5] = [
    "open_document",
    "new_document",
    "close_document",
    "read_blob",
    "save_document",
];

/// What the Python host adds to a Rust tool's reply that the session tools' port (D3)
/// brings: `describe`'s problems the document opened with, `check`'s whole-document
/// validation (round 53) and its note in the summary.
const HOST_ADDED: [(&str, &str); 3] = [
    ("describe", "result.data.problems"),
    ("check", "result.data.validation"),
    ("check", "result.summary"),
];

/// A read_batch op's path split: its index and the rest (`result.data.ops[2].data.x`:
/// `(2, "data.x")`).
fn op_path(path: &str) -> Option<(usize, &str)> {
    let rest = path.strip_prefix("result.data.ops[")?;
    let (index, rest) = rest.split_once("].")?;
    Some((index.parse().ok()?, rest))
}

/// Whether docx-agent's adapters change the call before the session sees it: a `$ref`
/// inside a place (`after:$x`), `word_insert_markdown`'s blob read as its Markdown, a deck's
/// parameter on `find_text`, `replace_text` or `render`.
fn adapted(tool: &str, arguments: &Value) -> bool {
    let place_ref = |v: &Value| {
        v.as_str()
            .is_some_and(|t| ["after:$", "before:$", "replace:$"].iter().any(|s| t.starts_with(s)))
    };
    let decks = ["find_text", "replace_text", "render"].contains(&tool)
        && arguments.get("slides").is_some_and(|v| !v.is_null());
    let blob = tool == "word_insert_markdown" && arguments.get("blob").is_some_and(Value::is_string);
    decks || blob || arguments.get("at").is_some_and(place_ref)
}

/// What a call of `tool` with `arguments` waits for, and whether its case's later events
/// depend on it (it changes the session's state).
fn waits(tool: &str, arguments: &Value) -> Option<(Waits, bool)> {
    let bare = tool.rsplit('.').next().unwrap_or(tool);
    if bare == "batch" {
        return Some((Waits::D2, true));
    }
    if SESSION_TOOLS.contains(&bare) {
        return Some((Waits::D3, bare != "save_document" && bare != "read_blob"));
    }
    if adapted(bare, arguments) {
        return Some((Waits::Adapters, true));
    }
    if bare == "check" {
        let include = arguments.get("include");
        let numbers = match include {
            Some(Value::Array(items)) => items.iter().any(|i| i == "numbers"),
            Some(Value::String(one)) => one == "numbers",
            _ => false,
        };
        if numbers || arguments.get("scale").is_some_and(|s| !s.is_null()) {
            return Some((Waits::G1, false));
        }
    }
    None
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
        for kind in ["tests", "transcripts"] {
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
    /// (roles; `Settings` for any other), its clock.
    fn session(&self, head: &Value) -> Result<HostSession, Waits> {
        let mut settings = HostSettings::default();
        let given = |which: &str| head.get(which).and_then(Value::as_object).cloned().unwrap_or_default();
        let (toolbox, session) = (given("toolbox"), given("session"));
        for (key, value) in toolbox.iter().chain(session.iter()) {
            match key.as_str() {
                "clock" | "font_dirs" => {}
                "roles" => {
                    let names: Vec<&str> = value
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_str)
                        .collect();
                    settings.roles = RoleSet::parse(names).map_err(|_| Waits::Settings)?;
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
        let format = WordFormat { pages: Some(pages) };
        let host = Arc::new(Host::new(vec![Arc::new(format)], settings, Arc::new(NotYet)));
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
            locale: None,
        };
        Ok(host.session(config, context))
    }

    /// `events` (one case, its head first) made again.
    #[must_use]
    pub fn replay(&self, stream: &str, events: &[Value]) -> Replayed {
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

    /// One event made again: how it went, and whether the case's later events depend on it.
    fn event(&self, stream: &str, session: &HostSession, event: &Value) -> (Outcome, bool) {
        let kind = event.get("event").and_then(Value::as_str).unwrap_or_default();
        match kind {
            "open" => {
                let given = ["doc_id", "state"]
                    .iter()
                    .any(|k| event.get(*k).is_some_and(|v| !v.is_null()));
                let bytes = event.get("input").and_then(|i| self.input(i));
                let name = event.get("name").and_then(Value::as_str).unwrap_or("document.docx");
                match (given, bytes) {
                    (false, Some(bytes)) => match session.open(bytes, DocKind::Docx, name) {
                        Ok(id) => (
                            self.compare(stream, event, "returned", &Value::from(id.to_string())),
                            false,
                        ),
                        Err(_) => (Outcome::Waiting(Waits::D3, true), true),
                    },
                    _ => (Outcome::Waiting(Waits::D3, true), true),
                }
            }
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
            "call" => self.call(stream, session, event),
            "calls" => (Outcome::Waiting(Waits::D2, true), true),
            "settings" => (Outcome::Waiting(Waits::Settings, true), true),
            // the application's undo, redo and save: the session tools' (D3)
            _ => (Outcome::Waiting(Waits::D3, true), true),
        }
    }

    fn call(&self, stream: &str, session: &HostSession, event: &Value) -> (Outcome, bool) {
        let tool = event.get("tool").and_then(Value::as_str).unwrap_or_default();
        let arguments = event.get("arguments").cloned().unwrap_or(Value::Null);
        let waiting = waits(tool, &arguments);
        if let Some((waits, blocks)) =
            waiting.filter(|(w, _)| !matches!(w, Waits::G1) && !tool.ends_with("save_document"))
        {
            return (Outcome::Waiting(waits, false), blocks);
        }
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
        };
        match session.call(tool, arguments, &how) {
            // (save_document and a check of numbers run for what they leave: their layout,
            // the check seen; their answers wait)
            Ok(_) if waiting.is_some() => (Outcome::Waiting(waiting.map_or(Waits::D3, |w| w.0), false), false),
            Ok(result) => (self.compare(stream, event, "result", &normalised(&result)), false),
            Err(_) => (Outcome::Waiting(Waits::Settings, true), true),
        }
    }

    /// `have` against the event's recorded `field`, with the known differences admitted.
    fn compare(&self, stream: &str, event: &Value, field: &str, have: &Value) -> Outcome {
        let want = event.get(field).cloned().unwrap_or(Value::Null);
        let paths = differing(&want, have, field);
        if paths.is_empty() {
            return Outcome::Equal;
        }
        let case = event.get("case").and_then(Value::as_str).unwrap_or_default();
        let tool = event.get("tool").and_then(Value::as_str);
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
        let host_added = |p: &String| {
            // a read_batch op's path as its own call's: `result.data.ops[2].data.x` is op 2's
            // tool's `result.data.x`
            let (tool, p) = match op_path(p) {
                Some((k, rest)) => (
                    want.pointer(&format!("/data/ops/{k}/tool")).and_then(Value::as_str),
                    format!("result.{rest}"),
                ),
                None => (tool, p.clone()),
            };
            HOST_ADDED.iter().any(|(t, f)| {
                Some(*t) == tool && (p == *f || p.starts_with(&format!("{f}.")) || p.starts_with(&format!("{f}[")))
            })
        };
        if !left.is_empty() && left.iter().all(host_added) {
            return Outcome::Waiting(Waits::D3, false);
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
