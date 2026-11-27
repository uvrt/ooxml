//! Owns: `check`'s layout facts read from the pages as docx-agent words them in `layout`
//! (`smallest_text`, `small_text`, `spacers`, `keep_with_next_at_foot`, `table_parted`,
//! `settings.keep_together`): each entry's JSON, with the call that fixes it.
//!
//! Also owns [`page_facts_json`], those facts and the table, margin and uncertainty ones
//! ([`super::laid_tables`]) of one version, kept to the facts' scope (`range`: the
//! entries about its blocks) and capped as `check` lists them, and [`minimum_of`], a
//! minimum text size read from its JSON.
//!
//! Must not: find the facts (`ooxml_word_pages::layout_facts` does) or filter them.

use super::laid_tables::{column_fit_json, past_margins_json, split_row_json, uncertain_json, unrepeated_json};
use ooxml_word_pages::coverage::Coverage;
use ooxml_word_pages::layout_facts::columns::column_fit;
use ooxml_word_pages::layout_facts::keeps::{
    keep_together, keep_with_next_at_foot, kept_tables, KeepAtFoot, KeptTogether,
};
use ooxml_word_pages::layout_facts::margins::past_margins;
use ooxml_word_pages::layout_facts::parted::{table_parted, AfterFix, Parted};
use ooxml_word_pages::layout_facts::sizes::{text_sizes, Minimum, SmallText, TextSizes, TEXT_KINDS};
use ooxml_word_pages::layout_facts::spacers::{spacers, Spacer};
use ooxml_word_pages::layout_facts::tables::{drawn_tables, tables};
use ooxml_word_pages::layout_facts::uncertain::uncertain;
use ooxml_word_pages::layout_facts::{Facts, Scope};
use serde_json::{json, Map, Value};

/// `smallest_text`: the smallest size per kind.
#[must_use]
pub fn smallest_text_json(sizes: &TextSizes) -> Value {
    Value::Object(
        sizes
            .smallest
            .iter()
            .map(|(kind, size)| (kind.clone(), Value::from(*size)))
            .collect(),
    )
}

/// A `small_text` entry, with the `word_format` call clearing a direct size where the
/// paragraph's styles give it at least the minimum.
#[must_use]
pub fn small_text_json(entry: &SmallText) -> Value {
    let mut out = Map::new();
    out.insert("para".into(), Value::from(entry.para.clone()));
    out.insert("kind".into(), Value::from(entry.kind.clone()));
    out.insert("size".into(), Value::from(entry.size));
    out.insert("chars".into(), Value::from(entry.chars));
    out.insert("pages".into(), Value::from(entry.pages.clone()));
    if let Some(min) = entry.min {
        out.insert("min".into(), Value::from(min));
    }
    if let Some(style_size) = entry.style_size {
        out.insert("style_size".into(), Value::from(style_size));
        if entry.fixable {
            let call = json!({"targets": [entry.para], "clear": ["size"]});
            out.insert("fix".into(), json!({"tool": "word_format", "calls": [call]}));
        }
    }
    Value::Object(out)
}

/// A `spacers` entry, with the `word_delete` call that removes the run and the
/// `word_format` call that keeps its space as the next paragraph's.
#[must_use]
pub fn spacer_json(entry: &Spacer) -> Value {
    let mut out = Map::new();
    out.insert("paras".into(), Value::from(entry.paras.clone()));
    out.insert("page".into(), Value::from(entry.page));
    out.insert(
        "fix".into(),
        json!({"tool": "word_delete", "calls": [{"target": entry.target}]}),
    );
    out.insert("height_pt".into(), Value::from(entry.height_pt));
    if let Some(next) = &entry.next {
        out.insert("next".into(), Value::from(next.clone()));
    }
    if let (Some(gap), Some(next)) = (entry.keep_space, &entry.next) {
        let call = json!({"targets": [next], "space_before": gap});
        out.insert("keep_space".into(), json!({"tool": "word_format", "calls": [call]}));
    }
    if entry.page_end {
        out.insert("page_end".into(), Value::Bool(true));
    }
    Value::Object(out)
}

/// A `keep_with_next_at_foot` entry.
#[must_use]
pub fn keep_at_foot_json(entry: &KeepAtFoot) -> Value {
    match entry {
        KeepAtFoot::Paragraph {
            para,
            style,
            page,
            next,
            next_page,
        } => json!({"para": para, "style": style, "page": page, "next": next, "next_page": next_page}),
        KeepAtFoot::TableEnd {
            table,
            row,
            page,
            next,
            next_page,
        } => json!({"table": table, "row": row, "page": page, "next": next, "next_page": next_page}),
        KeepAtFoot::Row {
            table,
            row,
            page,
            next_row,
            next_page,
            taller_than_page,
        } => {
            let mut out =
                json!({"table": table, "row": row, "page": page, "next_row": next_row, "next_page": next_page});
            if *taller_than_page {
                set(&mut out, "taller_than_page", Value::Bool(true));
            }
            out
        }
    }
}

/// A `settings.keep_together` entry.
#[must_use]
pub fn kept_together_json(entry: &KeptTogether) -> Value {
    let mut out = json!({"table": entry.table, "pages": entry.pages, "direct": entry.direct});
    if entry.with_next {
        set(&mut out, "with_next", Value::Bool(true));
    }
    if entry.taller_than_page {
        set(&mut out, "taller_than_page", Value::Bool(true));
    }
    out
}

/// A `table_parted` entry, with the keep that moves the paragraph and the table together.
#[must_use]
pub fn parted_json(entry: &Parted) -> Value {
    match entry {
        Parted::LeadIn {
            table,
            para,
            kind,
            para_page,
            table_page,
        } => json!({
            "table": table, "para": para, "side": "lead_in", "kind": kind.as_str(),
            "para_page": para_page, "table_page": table_page,
            "fix": {"tool": "word_format", "calls": [{"targets": [para], "keep_with_next": true}]},
        }),
        Parted::LineAfter {
            table,
            para,
            kind,
            row,
            row_page,
            para_page,
            fix,
        } => {
            let mut out = json!({
                "table": table, "para": para, "side": "line_after", "kind": kind.as_str(),
                "row": row, "row_page": row_page, "para_page": para_page,
            });
            match fix {
                Some(AfterFix::Table { keep_together }) => {
                    let mut call = json!({"table": table, "scope": "table", "keep_with_next": true});
                    if *keep_together {
                        set(&mut call, "keep_together", Value::Bool(true));
                    }
                    set(&mut out, "fix", json!({"tool": "word_format_table", "calls": [call]}));
                }
                Some(AfterFix::Row(lead)) => {
                    set(
                        &mut out,
                        "fix",
                        json!({"tool": "word_format", "calls": [{"targets": [lead], "keep_with_next": true}]}),
                    );
                }
                None => {}
            }
            out
        }
    }
}

/// `value` (an object) with `key` set to `to`.
fn set(value: &mut Value, key: &str, to: Value) {
    if let Some(object) = value.as_object_mut() {
        object.insert(key.into(), to);
    }
}

/// Entries per fact, most telling first (docx-agent's `CAP`).
pub(super) const CAP: usize = 20;

/// The keys of an entry that name what it is about (docx-agent's `SUBJECT_KEYS`).
const SUBJECT_KEYS: [&str; 6] = ["para", "paras", "ids", "table", "drawing", "blocks"];

/// Whether an entry is about `scope` (docx-agent's `Scope.about`): one of its subjects is
/// held, or it names none.
pub(super) fn about(scope: &Scope, entry: &Value) -> bool {
    let mut named = false;
    for key in SUBJECT_KEYS {
        let Some(value) = entry.get(key).filter(|v| !v.is_null()) else {
            continue;
        };
        named = true;
        let values = value.as_array().map_or_else(|| vec![value], |a| a.iter().collect());
        if values.iter().any(|v| v.as_str().is_some_and(|s| scope.holds(s))) {
            return true;
        }
    }
    !named
}

/// `entries` as `name`'s list in `out`, those about the facts' scope, the first [`CAP`]
/// of them, and how many are left out (`<name>_left_out`) where any are (docx-agent's
/// `capped`).
fn capped<T>(facts: &Facts<'_>, out: &mut Map<String, Value>, name: &str, entries: &[T], each: impl Fn(&T) -> Value) {
    capped_values(facts.scope(), out, name, entries.iter().map(each).collect());
}

/// [`capped`] of entries already worded, those about `scope` (`None`: all).
pub(super) fn capped_values(scope: Option<&Scope>, out: &mut Map<String, Value>, name: &str, entries: Vec<Value>) {
    let entries: Vec<Value> = entries
        .into_iter()
        .filter(|e| scope.is_none_or(|s| about(s, e)))
        .collect();
    let left = entries.len().saturating_sub(CAP);
    out.insert(name.into(), entries.into_iter().take(CAP).collect());
    if left > 0 {
        out.insert(format!("{name}_left_out"), Value::from(left));
    }
}

/// The layout facts of `facts` this module words, capped as `check` lists them, into
/// `out` (`settings`' `keep_together` into `settings`); `coverage` the version's (its
/// placeholders and approximations: `uncertain`).
pub fn page_facts_json(
    facts: &Facts<'_>,
    minimum: Option<&Minimum>,
    coverage: &Coverage,
    out: &mut Map<String, Value>,
    settings: &mut Map<String, Value>,
) {
    let sizes = text_sizes(facts, minimum);
    out.insert("smallest_text".into(), smallest_text_json(&sizes));
    if let Some(small) = &sizes.small {
        capped(facts, out, "small_text", small, small_text_json);
    }
    capped(facts, out, "spacers", &spacers(facts), spacer_json);
    let kept = kept_tables(facts);
    capped(
        facts,
        out,
        "keep_with_next_at_foot",
        &keep_with_next_at_foot(facts, &kept),
        keep_at_foot_json,
    );
    capped(facts, out, "table_parted", &table_parted(facts), parted_json);
    capped(
        facts,
        settings,
        "keep_together",
        &keep_together(facts, &kept),
        kept_together_json,
    );
    let (split, unrepeated) = tables(facts, &drawn_tables(facts));
    capped(facts, out, "split_rows", &split, split_row_json);
    capped(facts, out, "header_not_repeated", &unrepeated, unrepeated_json);
    capped(facts, out, "column_fit", &column_fit(facts), column_fit_json);
    capped(facts, out, "past_margins", &past_margins(facts), past_margins_json);
    capped(facts, out, "uncertain", &uncertain(facts, coverage), uncertain_json);
}

/// A minimum text size as `check`'s `min_size` (or the application's `min_text_size`) gives
/// it: points for every kind, or points by kind (`None` values left out); `None` for none.
///
/// # Errors
/// docx-agent's words for a setting that is not one (`MinimumError`).
pub fn minimum_of(value: &Value) -> Result<Option<Minimum>, String> {
    let points = |kind: &str, given: &Value| -> Result<f64, String> {
        match given
            .as_f64()
            .filter(|p| !given.is_boolean() && (1.0..=400.0).contains(p))
        {
            Some(p) => Ok(p),
            None => {
                let named = if kind.is_empty() {
                    String::new()
                } else {
                    format!("['{kind}']")
                };
                Err(format!(
                    "min_text_size{named} {given}: points between 1 and 400, or a mapping of {} to points",
                    TEXT_KINDS.join(", ")
                ))
            }
        }
    };
    match value {
        Value::Null => Ok(None),
        Value::Object(kinds) => {
            if let Some(unknown) = kinds.keys().find(|k| !TEXT_KINDS.contains(&k.as_str())) {
                return Err(format!(
                    "min_text_size has no kind '{unknown}'; the kinds are {}",
                    TEXT_KINDS.join(", ")
                ));
            }
            let mut found = std::collections::BTreeMap::new();
            for (kind, given) in kinds.iter().filter(|(_, v)| !v.is_null()) {
                found.insert(kind.clone(), points(kind, given)?);
            }
            if found.is_empty() {
                return Err(format!(
                    "min_text_size is an empty mapping; name {}, or give one number",
                    TEXT_KINDS.join(", ")
                ));
            }
            Ok(Some(Minimum::PerKind(found)))
        }
        other => Ok(Some(Minimum::One(points("", other)?))),
    }
}

#[cfg(test)]
mod tests {
    use super::{minimum_of, Minimum};
    use serde_json::json;
    use std::collections::BTreeMap;

    #[test]
    fn a_minimum_is_one_number_or_points_by_kind_with_nones_left_out() {
        assert_eq!(minimum_of(&json!(null)), Ok(None));
        assert_eq!(minimum_of(&json!(9)), Ok(Some(Minimum::One(9.0))));
        let kinds = BTreeMap::from([("table".to_owned(), 8.5)]);
        assert_eq!(
            minimum_of(&json!({"table": 8.5, "caption": null})),
            Ok(Some(Minimum::PerKind(kinds)))
        );
    }

    #[test]
    fn what_is_not_a_minimum_is_refused_in_docx_agents_words() {
        let kinds = "body, table, footnote, endnote, header, footer, caption";
        assert_eq!(
            minimum_of(&json!({"chart": 9})),
            Err(format!("min_text_size has no kind 'chart'; the kinds are {kinds}"))
        );
        assert_eq!(
            minimum_of(&json!({})),
            Err(format!(
                "min_text_size is an empty mapping; name {kinds}, or give one number"
            ))
        );
        assert_eq!(
            minimum_of(&json!({"body": 0})),
            Err(format!(
                "min_text_size['body'] 0: points between 1 and 400, or a mapping of {kinds} to points"
            ))
        );
        for bad in [json!(0), json!("9"), json!(true), json!(401)] {
            assert!(minimum_of(&bad).is_err(), "{bad}");
        }
    }
}
