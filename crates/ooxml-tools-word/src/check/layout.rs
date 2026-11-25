//! Owns: `check`'s `layout` and `range` (docx-agent's `_check` with `layout_facts` and the
//! ranges a check with `range` reports): the facts read from the pages
//! ([`super::laid_facts`]) and those read from the document alone ([`super::facts`]), on the
//! pages asked for and about the blocks a `range` names, capped and in docx-agent's order;
//! each range's extent, keeps and what its keep does ([`super::ranges`]); and where the
//! layout did not finish in time, the facts read from the document alone.
//!
//! Must not: lay out but through the document's engine, or find a fact (the model and
//! page crates do).

use super::laid_facts::{about, capped_values, page_facts_json, CAP};
use super::placed::Placed;
use super::{facts as worded, ranges_json, Trials, TRIALS, TRIAL_SECONDS};
use crate::version::Owned;
use crate::LastCheck;
use ooxml_base::Cancel;
use ooxml_tools::ToolError;
use ooxml_word_edit::facts::{self, LanguageScope};
use ooxml_word_edit::{EditError, WordDoc};
use ooxml_word_pages::layout_facts::sizes::Minimum;
use ooxml_word_pages::layout_facts::{Facts, PageReads, Scope};
use ooxml_word_pages::page_entry::PageEntry;
use ooxml_word_pages::{LaidVersion, PageEngine};
use serde_json::{Map, Value};
use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};

/// The layout facts' keys in the order docx-agent writes them (each `<key>_left_out` right
/// after its key).
const ORDER: [&str; 23] = [
    "error",
    "smallest_text",
    "small_text",
    "direct_formatting",
    "styles_not_allowed",
    "styles_unknown",
    "spacers",
    "empty_paragraphs",
    "keep_with_next_at_foot",
    "table_parted",
    "numbering_skips_level",
    "table_grid_mismatch",
    "accept_loses_text",
    "punctuation_spacing",
    "text_language",
    "settings",
    "split_rows",
    "header_not_repeated",
    "column_fit",
    "past_margins",
    "uncertain",
    "accept_loses_original",
    "row_text_format",
];

/// The pages' text, painted by the engine that laid the version out.
pub(super) struct Painted<'e> {
    pub(super) engine: &'e Mutex<PageEngine>,
    pub(super) version: Arc<LaidVersion>,
}

impl PageReads for Painted<'_> {
    fn page_read(&self, page: usize) -> Option<Arc<PageEntry>> {
        let mut engine = ooxml_word_pages::lock_engine(self.engine);
        engine.page_read(&self.version, page)
    }

    fn laid_entry(&self, page: usize) -> Option<Arc<ooxml_word_pages::laid_entry::LaidEntry>> {
        ooxml_word_pages::lock_engine(self.engine).laid_entry(&self.version, page)
    }

    fn resolved_table(
        &self,
        table: &ooxml_word_model::model::Table,
    ) -> Option<Arc<ooxml_word_model::resolve::ResolvedTable>> {
        ooxml_word_pages::lock_engine(self.engine).resolved_table(&self.version, table)
    }
}

/// What a check asks of the layout's facts.
pub(super) struct Asked<'a> {
    /// The minimum text size (`min_size`).
    pub(super) minimum: Option<&'a Minimum>,
    /// The styles allowed (`styles`).
    pub(super) styles: Option<&'a [String]>,
}

/// `layout` of `facts` (laid out, scoped and on the pages asked for), with each range's
/// report (`range`) where scoped; `trials` the keep trials' engine and stop.
pub(super) fn laid(
    facts: &Facts<'_>,
    owned: Owned<'_>,
    asked: &Asked<'_>,
    coverage: &ooxml_word_pages::coverage::Coverage,
    trials: (&Mutex<PageEngine>, &dyn Cancel),
) -> Result<(Value, Option<Value>), ToolError> {
    let doc = owned.doc;
    let docx = facts.docx();
    let mut parts = Map::new();
    let mut settings = Map::new();
    page_facts_json(facts, asked.minimum, coverage, &mut parts, &mut settings);
    parts.insert("settings".into(), Value::Object(settings));
    let placed = Placed::of(facts);
    paragraph_facts(facts, &placed, (doc, owned.last_check), &mut parts)?;
    if let Some(allowed) = asked.styles {
        style_list(facts, &placed, doc, allowed, &mut parts)?;
    }
    table_facts(facts, doc, owned.made_by, &mut parts)?;
    session_facts(owned, facts.scope(), &mut parts)?;
    let range = facts.scope().map(|_| {
        let trials = Trials {
            engine: trials.0,
            trials: TRIALS,
            seconds: TRIAL_SECONDS,
            cancel: trials.1,
        };
        Value::from(ranges_json(facts, doc, docx, Some(&trials)))
    });
    Ok((ordered(parts), range))
}

/// The facts about paragraphs read from the document alone, on the pages asked for:
/// `direct_formatting`, `empty_paragraphs`, `numbering_skips_level`,
/// `punctuation_spacing` and `text_language`.
fn paragraph_facts(
    facts: &Facts<'_>,
    placed: &Placed<'_, '_>,
    (doc, last_check): (&WordDoc, &LastCheck),
    parts: &mut Map<String, Value>,
) -> Result<(), ToolError> {
    let pages_of = |id: &str| placed.pages(id);
    let entries = facts::direct_formatting(doc).map_err(internal)?;
    let mut direct = Vec::new();
    for entry in &entries {
        let pages = pages_of(&entry.para);
        if facts.shown(pages.iter().copied()) {
            let mut json = worded::direct_formatting_json(entry);
            set(&mut json, "pages", Value::from(pages));
            direct.push(json);
        }
    }
    capped_values(facts.scope(), parts, "direct_formatting", direct);
    let shown = |json: &Value, key: &str| -> bool {
        let ids: Vec<&str> = match json.get(key) {
            Some(Value::Array(items)) => items.iter().filter_map(Value::as_str).collect(),
            Some(Value::String(id)) => vec![id.as_str()],
            _ => Vec::new(),
        };
        facts.shown(ids.into_iter().flat_map(&pages_of))
    };
    let empty = facts::empty_paragraphs(doc).map_err(internal)?;
    let empty = empty.iter().map(worded::empty_run_json).filter(|e| shown(e, "ids"));
    capped_values(facts.scope(), parts, "empty_paragraphs", empty.collect());
    let skipped = facts::numbering_skips_level(doc).map_err(internal)?;
    let skipped = skipped
        .iter()
        .map(worded::skipped_level_json)
        .filter(|e| shown(e, "para"));
    capped_values(facts.scope(), parts, "numbering_skips_level", skipped.collect());
    let spacing = facts::punctuation_spacing(doc, None).map_err(internal)?;
    let spacing = spacing
        .iter()
        .map(worded::spacing_problem_json)
        .filter(|e| shown(e, "para"));
    capped_values(facts.scope(), parts, "punctuation_spacing", spacing.collect());
    let keep = |id: &str| -> bool {
        facts.scope().is_none_or(|s| s.paragraphs.contains(id)) && (facts.whole() || facts.shown(pages_of(id)))
    };
    // Unscoped on every page, every paragraph is kept: none is named to ask.
    let every = facts.scope().is_none() && facts.whole();
    let language = LanguageScope::new(every, true).with_detections(last_check.languages());
    let language = if every { language } else { language.keeping(&keep) };
    if let Some(found) = facts::text_language(doc, language).map_err(internal)? {
        parts.insert("text_language".into(), worded::text_language_json(&found, CAP));
    }
    Ok(())
}

/// The facts about tables read from the document alone, on the pages asked for (each with
/// its table's pages): `table_grid_mismatch` and `accept_loses_text`.
fn table_facts(
    facts: &Facts<'_>,
    doc: &WordDoc,
    made_by: Option<&crate::MadeBy>,
    parts: &mut Map<String, Value>,
) -> Result<(), ToolError> {
    let wanted = |table: &str| -> bool {
        facts.scope().is_none_or(|s| s.holds(table)) && facts.shown(facts.table_pages_of(table))
    };
    let grid = facts::table_grid_mismatch(doc, Some(&wanted)).map_err(internal)?;
    let mut mismatched = Vec::new();
    for entry in &grid {
        let mut json = worded::grid_mismatch_json(entry);
        let pages = json
            .get("table")
            .and_then(Value::as_str)
            .map(|t| facts.table_pages_of(t))
            .unwrap_or_default();
        if facts.shown(pages.iter().copied()) {
            set(&mut json, "pages", Value::from(pages));
            mismatched.push(json);
        }
    }
    if let Some(made_by) = made_by {
        made_by.add_to(&mut mismatched);
    }
    capped_values(facts.scope(), parts, "table_grid_mismatch", mismatched);
    let mut lost = Vec::new();
    for entry in &facts::accept_loses_text(doc).map_err(internal)? {
        let table = cell_table(&entry.cell);
        let pages = facts.table_pages_of(table);
        if facts.shown(pages.iter().copied()) && facts.scope().is_none_or(|s| s.about(&[table])) {
            let mut json = worded::lost_cell_json(entry);
            set(&mut json, "pages", Value::from(pages));
            lost.push(json);
        }
    }
    capped_values(None, parts, "accept_loses_text", lost);
    Ok(())
}

/// `layout` where laying the version out did not finish (`error`, the reflow's): the facts
/// read from the document alone, about `scope`, the first [`CAP`] of each.
pub(super) fn unlaid(owned: Owned<'_>, scope: Option<&Scope>, error: Value) -> Result<Value, ToolError> {
    let doc = owned.doc;
    let mut parts = Map::new();
    parts.insert("error".into(), error);
    let kept = |list: Vec<Value>| -> Value {
        let about = list.into_iter().filter(|e| scope.is_none_or(|s| about(s, e)));
        Value::from(about.take(CAP).collect::<Vec<_>>())
    };
    let empty = facts::empty_paragraphs(doc).map_err(internal)?;
    parts.insert(
        "empty_paragraphs".into(),
        kept(empty.iter().map(worded::empty_run_json).collect()),
    );
    let skipped = facts::numbering_skips_level(doc).map_err(internal)?;
    parts.insert(
        "numbering_skips_level".into(),
        kept(skipped.iter().map(worded::skipped_level_json).collect()),
    );
    let spacing = facts::punctuation_spacing(doc, None).map_err(internal)?;
    parts.insert(
        "punctuation_spacing".into(),
        kept(spacing.iter().map(worded::spacing_problem_json).collect()),
    );
    let wanted = |table: &str| scope.is_none_or(|s| s.holds(table));
    let grid = facts::table_grid_mismatch(doc, Some(&wanted)).map_err(internal)?;
    let mut grid: Vec<Value> = grid.iter().map(worded::grid_mismatch_json).collect();
    if let Some(made_by) = owned.made_by {
        made_by.add_to(&mut grid);
    }
    parts.insert("table_grid_mismatch".into(), kept(grid));
    let lost: Vec<Value> = facts::accept_loses_text(doc)
        .map_err(internal)?
        .iter()
        .filter(|e| scope.is_none_or(|s| s.about(&[cell_table(&e.cell)])))
        .map(worded::lost_cell_json)
        .take(CAP)
        .collect();
    parts.insert("accept_loses_text".into(), Value::from(lost));
    let keep = |id: &str| scope.is_none_or(|s| s.paragraphs.contains(id));
    // Unscoped, every paragraph is kept: none is named to ask.
    let language = LanguageScope::new(scope.is_none(), false).with_detections(owned.last_check.languages());
    let language = if scope.is_none() {
        language
    } else {
        language.keeping(&keep)
    };
    if let Some(found) = facts::text_language(doc, language).map_err(internal)? {
        parts.insert("text_language".into(), worded::text_language_json(&found, CAP));
    }
    session_facts(owned, scope, &mut parts)?;
    Ok(ordered(parts))
}

/// Each range where the layout did not finish: its address and blocks, no pages.
pub(super) fn unlaid_ranges(scope: &Scope) -> Value {
    let ranges: Vec<Value> = scope
        .ranges
        .iter()
        .map(|r| serde_json::json!({"range": r.address, "blocks": r.blocks.len(), "pages": null}))
        .collect();
    Value::from(ranges)
}

/// The facts of what this session did to the document: the file's text a pending change
/// of it takes out (`accept_loses_original`), and the rows its table edits wrote that look
/// unlike the rows around (`row_text_format`, of the rows the host's record holds).
fn session_facts(owned: Owned<'_>, scope: Option<&Scope>, parts: &mut Map<String, Value>) -> Result<(), ToolError> {
    let opened = owned.opened.docx().map_err(internal)?;
    let found = facts::accept_loses_original(owned.doc, opened).map_err(internal)?;
    capped_values(
        scope,
        parts,
        "accept_loses_original",
        found.iter().map(worded::lost_original_json).collect(),
    );
    let wanted = |table: &str| scope.is_none_or(|s| s.holds(table));
    let written = owned.rows_written.map(crate::RowsWritten::ids).unwrap_or_default();
    let rows = facts::row_text_format(owned.doc, &written, Some(&wanted)).map_err(internal)?;
    capped_values(
        None,
        parts,
        "row_text_format",
        rows.iter().map(worded::row_unlike_json).collect(),
    );
    Ok(())
}

/// `styles_not_allowed` (the styles in use the list does not allow, by block in the scope,
/// on the pages asked for) and `styles_unknown`.
fn style_list(
    facts: &Facts<'_>,
    placed: &Placed<'_, '_>,
    doc: &WordDoc,
    allowed: &[String],
    parts: &mut Map<String, Value>,
) -> Result<(), ToolError> {
    let found = facts::style_list(doc, allowed).map_err(internal)?;
    let mut entries = Vec::new();
    for mut entry in found.not_allowed {
        if let Some(scope) = facts.scope() {
            entry.blocks.retain(|b| scope.holds(b));
            if entry.blocks.is_empty() {
                continue;
            }
        }
        let pages: BTreeSet<usize> = entry
            .blocks
            .iter()
            .flat_map(|b| match entry.kind {
                "table" => facts.table_pages_of(b),
                _ => placed.pages(b),
            })
            .collect();
        if !pages.is_empty() && !facts.shown(pages.iter().copied()) {
            continue;
        }
        let mut json = worded::style_use_json(&entry);
        set(
            &mut json,
            "pages",
            Value::from(pages.into_iter().take(CAP).collect::<Vec<_>>()),
        );
        entries.push((entry.blocks.len(), json));
    }
    entries.sort_by_key(|(count, _)| std::cmp::Reverse(*count));
    capped_values(
        facts.scope(),
        parts,
        "styles_not_allowed",
        entries.into_iter().map(|(_, e)| e).collect(),
    );
    parts.insert("styles_unknown".into(), Value::from(found.unknown));
    Ok(())
}

/// The table a cell address (`<table id>/c<row>,<column>`) is in.
fn cell_table(cell: &str) -> &str {
    cell.rsplit_once("/c").map_or(cell, |(table, _)| table)
}

/// `parts` in [`ORDER`] (anything else after, as found).
fn ordered(mut parts: Map<String, Value>) -> Value {
    let mut out = Map::new();
    for key in ORDER {
        if let Some(value) = parts.shift_remove(key) {
            out.insert(key.into(), value);
        }
        let left = format!("{key}_left_out");
        if let Some(value) = parts.shift_remove(&left) {
            out.insert(left, value);
        }
    }
    out.extend(parts);
    Value::Object(out)
}

/// `value` (an object) with `key` set to `to`.
fn set(value: &mut Value, key: &str, to: Value) {
    if let Some(object) = value.as_object_mut() {
        object.insert(key.into(), to);
    }
}

fn internal(error: EditError) -> ToolError {
    ToolError::internal(error.to_string(), "check:layout")
}
