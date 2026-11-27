//! The tool cases of `word-edits` that take their arguments as the session does (`op`
//! `structure`, `objects`, `style`): a tool called with JSON arguments on a document as
//! opened -- its arguments taken as the tool's intent rules take them -- each reply as the
//! tool writes it, every part whose content changed, the archive's part order, `touched`,
//! the validation delta, the problems and the next ids, for `python/word_edits_diff.py
//! --structure`, `--objects` and `--style` to compare with docx-agent's toolbox.
//!
//! A tool the session registers runs as [`Registry::standard`] holds it
//! (its [`ooxml_tools::Tool`] run in a context of the document as opened), so a newly
//! registered tool needs nothing here;
//! [`unregistered`] runs the ported tools the session does not serve yet.
//!
//! Must not: be used by any library; it is a test instrument.

use crate::edits::{Monotonic, Unlimited};
use ooxml_base::{Cancel, Limits};
use ooxml_opc::Package;
use ooxml_session::Registry;
use ooxml_store::{Caller, StoreConfig};
use ooxml_tools::{Accepted, CallEnv, FormatCall, Run, ToolArgs, ToolResult};
use ooxml_tools_word::{
    word_headers_footers, word_links, word_move, word_notes, word_sections, Blob, BlobList, CallEditor, Editing,
    EnginePages, LastCheck, Others, Owned, Version, Word, WordHeadersFooters, WordLinks, WordMove, WordNotes,
    WordSections,
};
use ooxml_word_edit::touched::{touched, Touched};
use ooxml_word_edit::validate::{problems, Delta};
use ooxml_word_edit::{Editor, Tracking, WordDoc};
use ooxml_word_model::docx::Versions;
use serde_json::{json, Map, Value};
use std::sync::{Arc, OnceLock};

struct Never;

impl Cancel for Never {
    fn is_cancelled(&self) -> bool {
        false
    }
}

/// One tool case on a freshly opened document.
///
/// # Errors
/// A message when the document cannot be opened, or no tool is named so.
pub fn case(bytes: &Arc<[u8]>, case: &Value) -> Result<Value, String> {
    tool_case(bytes, case, |tool, arguments, editing| {
        registered(tool, arguments, editing).or_else(|| unregistered(tool, arguments, editing))
    })
}

/// The tool the session registers as `tool`, run on the document as opened (`None`: no
/// such tool): a reading tool reads it as `case.docx`, version 1, as a session owning it
/// hands it over.
fn registered(tool: &str, arguments: &Value, editing: &Editing<'_>) -> Option<ToolResult<Value>> {
    static REGISTRY: OnceLock<Registry> = OnceLock::new();
    let tool = REGISTRY.get_or_init(ooxml_session_word::registry).named(tool)?;
    let versions = Versions::new();
    let last_check = LastCheck::default();
    let version = opened_version(editing, &versions, &last_check);
    let spec = tool.spec();
    let Accepted {
        mut arguments,
        mut notes,
    } = match spec.accept(arguments.clone()) {
        Ok(accepted) => accepted,
        Err(error) => return Some(ToolResult::failed(error)),
    };
    notes.extend(spec.ignored_for(Some("docx"), &mut arguments));
    let outside = Outside {
        editing,
        version: &version,
    };
    let mut effects = ooxml_tools::OpEffects::default();
    let mut env = CallEnv {
        cancel: editing.cancel,
        format: &outside,
        effects: &mut effects,
    };
    let mut reply = tool.run(&arguments, &mut env);
    effects.give_to(&mut reply);
    // the notes on what was read as what first among the reply's warnings
    notes.append(&mut reply.warnings);
    reply.warnings = notes;
    Some(reply)
}

/// A Word document a test instrument opened outside a session, as a tool's call runs on
/// it: a reading tool reads `version`, an editing tool edits through `editing` (a tracking
/// mode it sets is not kept).
struct Outside<'a> {
    editing: &'a Editing<'a>,
    version: &'a Version<'a>,
}

impl FormatCall for Outside<'_> {
    fn run(&self, tool: &str, _: &Map<String, Value>, run: &dyn std::any::Any) -> ToolResult<Value> {
        match run.downcast_ref::<Run<Word>>() {
            Some(Run::Read(read)) => match read(self.version, self.editing.cancel) {
                Ok(done) => ToolResult::done(done),
                Err(error) => ToolResult::failed(error),
            },
            Some(Run::Edit(edit)) => edit(self.editing),
            Some(Run::Tracking(edit)) => edit(self.editing).0,
            None => ToolResult::failed(ooxml_tools::ToolError::internal(
                format!("{tool} is not a Word tool"),
                "oracle:outside",
            )),
        }
    }
}

/// The document as opened, version 1, as a reading tool is handed it by a session that
/// owns it: its Word document, the problems it opened with, the other documents the call
/// names; `last_check` the document's (no layout is handed: a check that needs one leaves
/// the call to the host).
pub(crate) fn opened_version<'a>(
    editing: &Editing<'a>,
    versions: &'a Versions,
    last_check: &'a LastCheck,
) -> Version<'a> {
    let owned = Owned {
        doc: editing.opened,
        baseline: editing.editor.baseline(),
        last_check,
        layout: None,
        layout_limit: None,
        opened: editing.opened,
        drawing: None,
        rows_written: editing.rows_written,
        made_by: editing.made_by,
        tracking: editing.tracking.map(|t| t.author.as_str()),
    };
    Version::new("case.docx", editing.opened.package(), 1, versions)
        .with_others(editing.others)
        .with_word(Some(editing.opened))
        .with_owned(Some(owned))
}

/// The ported tools the session does not register yet (each leaves this list when it is
/// registered).
fn unregistered(tool: &str, arguments: &Value, editing: &Editing<'_>) -> Option<ToolResult<Value>> {
    Some(match tool {
        "word_links" => run(arguments, |args: &WordLinks| word_links(args, editing)),
        "word_notes" => run(arguments, |args: &WordNotes| word_notes(args, editing)),
        "word_move" => run(arguments, |args: &WordMove| word_move(args, editing)),
        "word_sections" => run(arguments, |args: &WordSections| word_sections(args, editing)),
        "word_headers_footers" => run(arguments, |args: &WordHeadersFooters| {
            word_headers_footers(args, editing)
        }),
        _ => return None,
    })
}

/// `d2`, which a call may name: the document opened again (`reference_copy`) or the file
/// `reference_file`; its package, and the document as a session opens one (its ids
/// stamped, their renames kept) where it opens.
fn other_document(bytes: &Arc<[u8]>, case: &Value) -> Option<(ooxml_word_model::WordPackage, Option<WordDoc>)> {
    let other: Option<Arc<[u8]>> = match case.get("reference_file").and_then(Value::as_str) {
        Some(path) => std::fs::read(path).ok().map(Arc::from),
        None => case
            .get("reference_copy")
            .and_then(Value::as_bool)
            .unwrap_or(false)
            .then(|| Arc::clone(bytes)),
    };
    let package = other.and_then(|b| Package::open(b, &Limits::default()).ok())?;
    let word = WordDoc::open(package.clone(), Arc::new(Unlimited)).ok();
    // read as a session reads it: as its editor holds it, else stamped as an open stamps it
    let package = word.as_ref().map_or_else(
        || ooxml_word_model::WordPackage::open(package).0,
        |w| w.package().clone(),
    );
    Some((package, word))
}

/// The tracking mode a case gives (`tracked`: its author and date), if any.
fn case_tracking(case: &Value) -> Option<Tracking> {
    case.get("tracked").and_then(Value::as_object).map(|t| Tracking {
        author: t.get("author").and_then(Value::as_str).unwrap_or("").to_owned(),
        date: t.get("date").and_then(Value::as_str).unwrap_or("").to_owned(),
    })
}

/// The host's document locale a case gives (`locale`: ooxml-common's `Locale` fields, each
/// left out its default, as `Locale(**fields)` reads them), if any.
fn case_locale(case: &Value) -> Result<Option<ooxml_numfmt::Locale>, String> {
    let Some(fields) = case.get("locale").and_then(Value::as_object) else {
        return Ok(None);
    };
    let en = ooxml_numfmt::EN_US;
    let char_of = |key: &str, default: char| {
        fields
            .get(key)
            .and_then(Value::as_str)
            .and_then(|s| s.chars().next())
            .unwrap_or(default)
    };
    let text_of = |key: &str, default: &str| fields.get(key).and_then(Value::as_str).unwrap_or(default).to_owned();
    ooxml_numfmt::Locale::new(
        char_of("decimal_separator", en.decimal_separator),
        char_of("thousands_separator", en.thousands_separator),
        &text_of("date_order", &en.date_order),
        &text_of("language", &en.language),
        char_of("list_separator", en.list_separator),
    )
    .map(Some)
    .map_err(|e| e.to_string())
}

/// One tool case on a freshly opened document, the tool's reply given by `reply` (`None`:
/// no such tool): the reply and what the document became. `d2`, which a call may name, is
/// the document opened again (`reference_copy`) or the file `reference_file`.
///
/// # Errors
/// A message when the document cannot be opened, or the tool is not one `reply` knows.
pub(crate) fn tool_case(
    bytes: &Arc<[u8]>,
    case: &Value,
    reply: impl FnOnce(&str, &Value, &Editing<'_>) -> Option<ToolResult<Value>>,
) -> Result<Value, String> {
    let package = Package::open(Arc::clone(bytes), &Limits::default()).map_err(|e| e.to_string())?;
    let editor = Editor::open(
        package.clone(),
        StoreConfig::default(),
        Arc::new(Monotonic::new()),
        Arc::new(Unlimited),
    )
    .map_err(|e| e.to_string())?;
    let opened = editor.store().snapshot().doc().clone();
    let caller = Caller::anonymous();
    let tracking = case_tracking(case);
    let layout = case.get("fonts").map(engine_pages);
    let date = case
        .get("date")
        .and_then(Value::as_str)
        .unwrap_or("2026-11-17T10:00:00Z");
    let blobs = case_blobs(case);
    let locale = case_locale(case)?;
    // the host's references: written into the arguments, or its refusal of the call
    let given = case.get("arguments").cloned().unwrap_or(Value::Object(Map::new()));
    let (arguments, written, refused, refusal) = match crate::case_values::case_values(case, given)? {
        Ok(values) => (values.arguments, values.written, values.refused, None),
        Err(refusal) => (Value::Null, Vec::new(), Vec::new(), Some(refusal)),
    };
    let sources = case_sources(case, refused);
    let copy = other_document(bytes, case);
    let copy_versions = Versions::new();
    let others: Vec<(&str, Version<'_>)> = copy
        .iter()
        .map(|(package, word)| ("d2", other_version(package, word.as_ref(), &copy_versions)))
        .collect();
    let call_editor = CallEditor::new(&editor);
    let editing = Editing::new(&call_editor, &caller, &opened, "d1", &Never, date)
        .with_tracking(tracking.as_ref())
        .with_layout(layout.as_ref().map(|l| l as &dyn ooxml_word_edit::FieldLayout))
        .with_others(Others::new(&others).with_sources(&sources))
        .with_blobs(Some(&blobs))
        .with_written(&written)
        .with_locale(locale.as_ref());
    let tool = case.get("tool").and_then(Value::as_str).unwrap_or("");
    let reply = match refusal {
        Some(refusal) => ToolResult::failed(refusal),
        None => reply(tool, &arguments, &editing).ok_or_else(|| format!("no tool {tool}"))?,
    };
    let head = editor.store().snapshot();
    let doc = head.doc();
    let parts = changed_parts(&package, doc);
    let found = touched(&opened, doc).map_err(|e| e.to_string())?;
    let now = problems(doc).map_err(|e| e.to_string())?;
    let delta = Delta::of(editor.baseline(), &now);
    Ok(json!({
        "reply": Value::Object(reply.to_json()),
        "not_supported": reply.not_supported_form(),
        "parts": parts,
        "order": doc.package().part_names(),
        "touched": found.iter().map(touched_json).collect::<Vec<_>>(),
        "validation": {"new": delta.new, "fixed": delta.fixed, "baseline": delta.baseline},
        "problems": now.iter().map(ToString::to_string).collect::<Vec<_>>(),
        "next": next_ids(doc),
    }))
}

/// The parts of `doc` that differ from `package`'s, by name, as text.
fn changed_parts(package: &Package, doc: &ooxml_word_edit::WordDoc) -> Map<String, Value> {
    let mut parts = Map::new();
    for name in doc.package().part_names() {
        let same = matches!((package.part(&name), doc.package().part(&name)), (Some(a), Some(b)) if a.same(b));
        if same {
            continue;
        }
        let bytes = doc
            .package()
            .read(&name)
            .ok()
            .flatten()
            .unwrap_or_else(|| Arc::from(Vec::new()));
        crate::part_texts::insert_part(&mut parts, name, &bytes);
    }
    parts
}

/// The other document a case names (`d2`), as a reading tool is handed it.
fn other_version<'a>(
    package: &'a ooxml_word_model::WordPackage,
    word: Option<&'a WordDoc>,
    versions: &'a Versions,
) -> Version<'a> {
    Version::new("copy.docx", package, 1, versions).with_word(word)
}

/// The rows the case's `from` names of the datasets it defines (`datasets`: `{id, columns,
/// types, rows}` each), as a host hands them over: a whole dataset (`d:sales`) or a run of
/// its rows (`d:sales/#2:#3`, `/#2:`, `/#2`); none for anything else (one value is a
/// cell's: [`crate::case_values`]). With them the host's refusals of cells' values,
/// `refused`.
fn case_sources(case: &Value, refused: Vec<Value>) -> ooxml_tools_word::Sources {
    let mut listed = refused;
    listed.extend(case_rows(case));
    ooxml_tools_word::Sources::from_json(&Value::Array(listed)).unwrap_or_default()
}

/// The rows of [`case_sources`], as `CallDocuments(sources=...)` takes them.
fn case_rows(case: &Value) -> Option<Value> {
    let given = case.pointer("/arguments/from").and_then(Value::as_str)?;
    let rest = given.strip_prefix("d:")?;
    let (id, run) = rest.split_once('/').map_or((rest, None), |(id, run)| (id, Some(run)));
    if run.is_some_and(|r| r.contains('/')) {
        return None;
    }
    let datasets = case.get("datasets").and_then(Value::as_array).into_iter().flatten();
    let found = datasets
        .into_iter()
        .find(|d| d.get("id").and_then(Value::as_str) == Some(id))?;
    let rows = found.get("rows").and_then(Value::as_array).cloned().unwrap_or_default();
    let (first, last) = match run.and_then(|r| r.strip_prefix('#')) {
        None => (1, rows.len()),
        Some(run) => {
            let number = |t: &str| t.trim_start_matches('#').parse::<usize>().ok();
            match run.split_once(':') {
                Some((first, "")) => (number(first).unwrap_or(1), rows.len()),
                Some((first, last)) => (number(first).unwrap_or(1), number(last).unwrap_or(rows.len())),
                None => (number(run).unwrap_or(1), number(run).unwrap_or(1)),
            }
        }
    };
    let taken: Vec<Value> = rows.into_iter().skip(first - 1).take(last + 1 - first).collect();
    let mut rows_json = found.clone();
    if let Value::Object(map) = &mut rows_json {
        map.insert("given".into(), json!(given));
        map.insert("address".into(), json!(given));
        map.insert("rows".into(), Value::Array(taken));
    }
    Some(rows_json)
}

/// The blobs a case registers (`blobs`: `{name, mime, hex}` each), as the session's
/// `add_blob` numbers them: `b1`, `b2`, ...
fn case_blobs(case: &Value) -> BlobList {
    let hex = |text: &str| -> Vec<u8> {
        text.as_bytes()
            .chunks(2)
            .filter_map(|pair| {
                std::str::from_utf8(pair)
                    .ok()
                    .and_then(|p| u8::from_str_radix(p, 16).ok())
            })
            .collect()
    };
    let listed = case.get("blobs").and_then(Value::as_array).into_iter().flatten();
    BlobList(
        listed
            .enumerate()
            .map(|(k, blob)| {
                let text = |key: &str| blob.get(key).and_then(Value::as_str).unwrap_or("").to_owned();
                Blob {
                    handle: format!("b{}", k + 1),
                    name: text("name"),
                    mime: text("mime"),
                    data: Arc::from(hex(&text("hex"))),
                }
            })
            .collect(),
    )
}

/// A page engine over the fonts a case names, as the layout port the field tools read.
pub(crate) fn engine_pages(fonts: &Value) -> EnginePages {
    EnginePages::of(page_engine(fonts))
}

/// A page engine over the fonts a case names (`folders`: `[location, path]` pairs as
/// docx2svg hands them to `ooxml_core.WordFonts`, `substitutes`, `substitute`).
pub(crate) fn page_engine(fonts: &Value) -> ooxml_word_pages::PageEngine {
    use ooxml_fonts::{Folder, Location, WordFontOptions};
    use ooxml_word_pages::{DiskFonts, EngineBudgets, EngineOptions, FontSetup, PageEngine};
    let pairs = |key: &str| -> Vec<(String, String)> {
        fonts
            .get(key)
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|pair| {
                let text = |k: usize| pair.get(k).and_then(Value::as_str).map(str::to_owned);
                Some((text(0)?, text(1)?))
            })
            .collect()
    };
    let folders: Vec<Folder> = pairs("folders")
        .into_iter()
        .map(|(location, path)| Folder {
            location: if location == "bundle" {
                Location::WordBundle
            } else {
                Location::Folder
            },
            path,
        })
        .collect();
    let substitute = fonts.get("substitute").and_then(Value::as_bool).unwrap_or(true);
    let options = WordFontOptions {
        substitutes: pairs("substitutes").into_iter().collect(),
        recorded_faces: substitute,
        fallbacks: substitute,
    };
    let setup = Arc::new(FontSetup::new(Arc::new(DiskFonts), &folders, options));
    let budgets = EngineBudgets {
        held: Arc::new(Unlimited),
        paint: Arc::new(Unlimited),
    };
    PageEngine::new(setup, EngineOptions::default(), budgets)
}

/// The tool's reply to `arguments` taken as its intent rules take them (a refusal of the
/// arguments is its reply), the notes on what was read as what first among its warnings.
pub(crate) fn run<A: ToolArgs>(arguments: &Value, handler: impl FnOnce(&A) -> ToolResult<Value>) -> ToolResult<Value> {
    match A::accept(arguments.clone(), Some("docx")) {
        Ok((args, notes)) => {
            let mut reply = handler(&args);
            if !notes.is_empty() {
                let mut warnings = notes;
                warnings.append(&mut reply.warnings);
                reply.warnings = warnings;
            }
            reply
        }
        Err(error) => ToolResult::failed(error),
    }
}

fn touched_json(touched: &Touched) -> Value {
    match &touched.section {
        None => json!({"part": touched.part}),
        Some(s) => json!({"part": touched.part, "heading": s.heading, "level": s.level, "blocks": s.blocks}),
    }
}

/// The value the main part's sequence and its text-id sequence give next.
fn next_ids(doc: &WordDoc) -> Map<String, Value> {
    let main = doc.docx().map(|d| d.main.clone()).unwrap_or_default();
    [main.clone(), format!("{main}\0text")]
        .into_iter()
        .map(|seed| {
            let values = doc.next_ids(&seed, 1);
            (seed, json!(values))
        })
        .collect()
}
