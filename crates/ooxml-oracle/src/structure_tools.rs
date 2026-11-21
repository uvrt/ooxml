//! The tool cases of `word-edits` that take their arguments as the session does (`op`
//! `structure`, `objects`, `style`): a tool called with JSON arguments on a document as
//! opened -- its arguments taken as the tool's intent rules take them -- each reply as the
//! tool writes it, every part whose content changed, the archive's part order, `touched`,
//! the validation delta, the problems and the next ids, for `python/word_edits_diff.py
//! --structure`, `--objects` and `--style` to compare with docx-agent's toolbox.
//!
//! A tool the session registers runs as [`Registry::standard`] holds it
//! ([`ooxml_session::Tool::run_outside`]), so a newly registered tool needs nothing here;
//! [`unregistered`] runs the ported tools the session does not serve yet.
//!
//! Must not: be used by any library; it is a test instrument.

use crate::edits::{Monotonic, Unlimited};
use ooxml_base::{Cancel, Limits};
use ooxml_opc::Package;
use ooxml_session::Registry;
use ooxml_store::{Caller, StoreConfig};
use ooxml_tools::{ToolArgs, ToolResult};
use ooxml_tools_word::{
    word_headers_footers, word_links, word_move, word_notes, word_sections, Blob, BlobList, CallEditor, Editing,
    EnginePages, LastCheck, Others, Owned, Version, WordHeadersFooters, WordLinks, WordMove, WordNotes, WordSections,
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
    let tool = REGISTRY.get_or_init(Registry::standard).named(tool)?;
    let versions = Versions::new();
    let last_check = LastCheck::default();
    let version = opened_version(editing, &versions, &last_check);
    Some(tool.run_outside(arguments, editing, &version))
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
fn other_document(bytes: &Arc<[u8]>, case: &Value) -> Option<(Package, Option<WordDoc>)> {
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
        .with_others(Others::new(&others))
        .with_blobs(Some(&blobs))
        .with_locale(locale.as_ref());
    let arguments = case.get("arguments").cloned().unwrap_or(Value::Object(Map::new()));
    let tool = case.get("tool").and_then(Value::as_str).unwrap_or("");
    let reply = reply(tool, &arguments, &editing).ok_or_else(|| format!("no tool {tool}"))?;
    let head = editor.store().snapshot();
    let doc = head.doc();
    let parts = changed_parts(&package, doc);
    let found = touched(&opened, doc).map_err(|e| e.to_string())?;
    let now = problems(doc).map_err(|e| e.to_string())?;
    let delta = Delta::of(editor.baseline(), &now);
    Ok(json!({
        "reply": Value::Object(reply.to_json()),
        "unported": reply.unported(),
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
        parts.insert(name, json!(String::from_utf8_lossy(&bytes)));
    }
    parts
}

/// The other document a case names (`d2`), as a reading tool is handed it.
fn other_version<'a>(package: &'a Package, word: Option<&'a WordDoc>, versions: &'a Versions) -> Version<'a> {
    Version::new("copy.docx", package, 1, versions).with_word(word)
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
    EnginePages::new(Arc::new(std::sync::Mutex::new(page_engine(fonts))))
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
