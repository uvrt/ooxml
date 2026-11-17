//! Owns: the fields of `describe` the Word model's reading gives (`ooxml_word_model::docx`):
//! headings, sections, styles in use, revision records, fields, tables, drawings and named
//! content controls, each as docx-agent's `tools/read.py` writes it.
//!
//! Must not: read parts beyond what [`Docx`] holds, or lay out.

use ooxml_word_model::docx::controls::{controls, Control};
use ooxml_word_model::docx::drawings::{drawing_kind, member_kind};
use ooxml_word_model::docx::drawings::{graphics, pictures, Drawing};
use ooxml_word_model::docx::numbering::{membership, Lists};
use ooxml_word_model::docx::outline::{body_blocks, headings};
use ooxml_word_model::docx::sections::{section_story, sections, Section, KINDS};
use ooxml_word_model::docx::tables::TableGrid;
use ooxml_word_model::docx::text::{paragraph_text, View};
use ooxml_word_model::docx::{fields::scan, revisions::records};
use ooxml_word_model::docx::{find_path, style_name, Docx};
use ooxml_word_model::ids::W;
use serde_json::{json, Map, Value};

/// One version's reading for `describe`.
pub(super) struct Reading<'a> {
    docx: &'a Docx,
    controls: Vec<Control>,
    sections: Vec<Section>,
    lists: Lists,
}

/// Python's `str.isspace` for one character.
fn py_space(c: char) -> bool {
    c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c)
}

/// docx-agent's `short`: whitespace runs as one space, at most `limit` characters (the
/// last an ellipsis where cut).
pub(crate) fn short(text: &str, limit: usize) -> String {
    let joined = text
        .split(py_space)
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    if joined.chars().count() <= limit {
        return joined;
    }
    let mut out: String = joined.chars().take(limit.saturating_sub(1)).collect();
    out.push('\u{2026}');
    out
}

/// A heading's number typed into its text (`1.2 `, `IV. `, `B. `), as docx-agent's
/// `_TYPED_NUMBER` matches it at the start.
pub(crate) fn typed_number(text: &str) -> bool {
    let rest = text.trim_start_matches(py_space);
    let after = typed_digits(rest)
        .or_else(|| {
            let roman = rest.find(|c: char| !"IVXLC".contains(c)).unwrap_or(rest.len());
            (roman > 0)
                .then(|| rest.get(roman..))
                .flatten()
                .and_then(|r| r.strip_prefix('.'))
        })
        .or_else(|| {
            let mut chars = rest.chars();
            chars.next().filter(char::is_ascii_uppercase)?;
            chars.as_str().strip_prefix('.')
        });
    after.and_then(|r| r.chars().next()).is_some_and(py_space)
}

/// `\d+(\.\d+)*\.?` at the start of `text` (Unicode digits, as Python's `\d`): the rest.
fn typed_digits(text: &str) -> Option<&str> {
    let digits = |s: &str| s.find(|c: char| !is_decimal(c)).unwrap_or(s.len());
    let first = digits(text);
    if first == 0 {
        return None;
    }
    let mut rest = text.get(first..)?;
    while let Some(after) = rest.strip_prefix('.') {
        let n = digits(after);
        if n == 0 {
            break;
        }
        rest = after.get(n..)?;
    }
    Some(rest.strip_prefix('.').unwrap_or(rest))
}

/// A decimal digit as Python's `\d` matches one: ASCII, and the other decimal digits
/// Word documents carry (Arabic-Indic, extended Arabic-Indic, Devanagari, full width).
fn is_decimal(c: char) -> bool {
    c.is_ascii_digit()
        || matches!(c, '\u{660}'..='\u{669}' | '\u{6f0}'..='\u{6f9}' | '\u{966}'..='\u{96f}' | '\u{ff10}'..='\u{ff19}')
}

impl<'a> Reading<'a> {
    pub(super) fn new(docx: &'a Docx) -> Reading<'a> {
        Reading {
            docx,
            controls: controls(docx),
            sections: sections(docx),
            lists: Lists::read(docx),
        }
    }

    /// The body's headings (docx-agent's `heading_tree`), every one.
    pub(super) fn headings(&self) -> Vec<Value> {
        let blocks = body_blocks(self.docx, &self.controls);
        let several = self.sections.len() > 1;
        headings(&blocks)
            .into_iter()
            .map(|h| {
                let text = paragraph_text(&h.element, View::Current);
                let mut item = Map::new();
                item.insert("id".into(), json!(h.id));
                item.insert("level".into(), json!(h.level));
                item.insert("text".into(), json!(short(&text, 100)));
                item.insert("blocks".into(), json!(h.blocks));
                if membership(self.docx, &self.lists, &h.element).is_some() {
                    item.insert("numbered".into(), json!("list"));
                } else if typed_number(&text) {
                    item.insert("numbered".into(), json!("text"));
                }
                if several {
                    item.insert("section".into(), json!(format!("s:{}", h.section)));
                }
                Value::Object(item)
            })
            .collect()
    }

    pub(super) fn sections(&self) -> Value {
        let out: Vec<Value> = self
            .sections
            .iter()
            .enumerate()
            .map(|(k, s)| {
                let mut item = Map::new();
                item.insert("id".into(), json!(s.id));
                item.insert("start".into(), json!(s.start()));
                item.insert("orientation".into(), json!(s.orientation()));
                let (width, height) = s
                    .page()
                    .map_or((Value::Null, Value::Null), |(w, h)| (json!(w), json!(h)));
                item.insert("page".into(), json!([width, height]));
                let mut margins = Map::new();
                for side in ["top", "right", "bottom", "left"] {
                    margins.insert(side.into(), s.margin(side).map_or(Value::Null, |m| json!(m)));
                }
                item.insert("margins".into(), Value::Object(margins));
                item.insert("columns".into(), json!(s.columns()));
                for which in ["header", "footer"] {
                    let mut shown = Map::new();
                    for kind in KINDS {
                        if let Some(story) = section_story(self.docx, &self.sections, k, which, kind) {
                            shown.insert(kind.into(), json!(story));
                        }
                    }
                    if !shown.is_empty() {
                        item.insert(format!("{which}s"), Value::Object(shown));
                    }
                }
                Value::Object(item)
            })
            .collect();
        Value::Array(out)
    }

    /// The paragraph styles applied, by name, most used first (at most 40).
    pub(super) fn styles_in_use(&self) -> Value {
        let mut declared: Vec<(Option<String>, u64)> = Vec::new();
        for part in &self.docx.parts {
            for p in &part.ids.paragraphs {
                let style = find_path(&p.element, &["pPr", "pStyle"])
                    .map(|s| s.attribute(Some(W), "val").unwrap_or("").to_owned());
                count(&mut declared, style);
            }
        }
        let default = self.docx.default_style("paragraph");
        let mut counts: Vec<(Option<String>, u64)> = Vec::new();
        for (style_id, n) in declared {
            let found = style_id
                .as_deref()
                .filter(|s| !s.is_empty())
                .and_then(|s| self.docx.find_style(s, "paragraph"))
                .or(default);
            let name = found.and_then(|s| style_name(s)).map(str::to_owned);
            match counts.iter_mut().find(|(k, _)| *k == name) {
                Some(slot) => slot.1 += n,
                None => counts.push((name, n)),
            }
        }
        // Counter.most_common: by count, ties in first-seen order.
        counts.sort_by(|a, b| b.1.cmp(&a.1));
        Value::Array(
            counts
                .into_iter()
                .take(40)
                .filter_map(|(name, n)| {
                    name.filter(|s| !s.is_empty())
                        .map(|s| json!({"name": s, "paragraphs": n}))
                })
                .collect(),
        )
    }

    /// Revision records and their authors (docx-agent also groups them into changes: not yet).
    pub(super) fn revisions(&self) -> Value {
        let found = records(self.docx);
        let mut authors: Vec<(Option<String>, u64)> = Vec::new();
        for r in &found {
            count(&mut authors, r.author().map(str::to_owned));
        }
        json!({"records": found.len(), "by_author": counter_object(authors)})
    }

    /// Every field's keyword, counted.
    pub(super) fn fields(&self) -> Value {
        let mut counts: Vec<(Option<String>, u64)> = Vec::new();
        for part in &self.docx.parts {
            for f in scan(part.root()) {
                count(&mut counts, Some(f.keyword()));
            }
        }
        counter_object(counts)
    }

    /// The body's tables (nested ones too), at most 30.
    pub(super) fn tables(&self) -> Value {
        let Some(body) = self.docx.body() else { return json!([]) };
        let out: Vec<Value> = body
            .ids
            .tables
            .iter()
            .take(30)
            .map(|t| {
                let grid = TableGrid::new(body, t);
                let mut item = Map::new();
                item.insert("id".into(), json!(t.id));
                item.insert("rows".into(), json!(grid.accepted_rows()));
                item.insert("columns".into(), json!(grid.column_count()));
                let first: Vec<String> = (0..grid.column_count())
                    .map(|k| grid.cell_text(0, k).unwrap_or_default())
                    .collect();
                item.insert("first_row".into(), json!(short(&first.join(" | "), 80)));
                for (label, local) in [("alt", "tblCaption"), ("description", "tblDescription")] {
                    let value = find_path(&t.element, &["tblPr", local]).and_then(|n| n.attribute(Some(W), "val"));
                    if let Some(value) = value.filter(|v| !v.is_empty()) {
                        item.insert(label.into(), json!(short(value, 80)));
                    }
                }
                Value::Object(item)
            })
            .collect();
        Value::Array(out)
    }

    /// Pictures, then charts not among them, at most 30.
    pub(super) fn drawings(&self) -> Value {
        let mut seen: Vec<Drawing> = pictures(self.docx);
        for chart in graphics(self.docx).into_iter().filter(|d| d.kind == "chart") {
            if !seen.iter().any(|d| d.id == chart.id) {
                seen.push(chart);
            }
        }
        Value::Array(
            seen.into_iter()
                .take(30)
                .map(|d| {
                    // A drawing is described by what its frame holds; a group's member by its own kind.
                    let kind = if d.member {
                        member_kind(&d.element)
                    } else {
                        drawing_kind(&d.element)
                    };
                    let mut item = Map::new();
                    item.insert("id".into(), json!(d.id));
                    item.insert("kind".into(), json!(kind));
                    if let Some(alt) = d.alt_text().filter(|a| !a.is_empty()) {
                        item.insert("alt_text".into(), json!(short(alt, 60)));
                    }
                    Value::Object(item)
                })
                .collect(),
        )
    }

    /// Content controls a tag or title names, at most 30; `None` where none is.
    pub(super) fn tagged(&self) -> Option<Value> {
        let named: Vec<Value> = self
            .controls
            .iter()
            .filter(|c| c.tag().is_some_and(|t| !t.is_empty()) || c.alias().is_some_and(|a| !a.is_empty()))
            .take(30)
            .map(|c| {
                let mut item = Map::new();
                item.insert("id".into(), json!(c.id));
                item.insert("kind".into(), json!(c.kind()));
                if let Some(tag) = c.tag().filter(|t| !t.is_empty()) {
                    item.insert("tag".into(), json!(short(tag, 80)));
                }
                if let Some(alias) = c.alias().filter(|a| !a.is_empty()) {
                    item.insert("title".into(), json!(short(alias, 80)));
                }
                Value::Object(item)
            })
            .collect();
        (!named.is_empty()).then_some(Value::Array(named))
    }
}

fn count(counts: &mut Vec<(Option<String>, u64)>, key: Option<String>) {
    match counts.iter_mut().find(|(k, _)| *k == key) {
        Some(slot) => slot.1 += 1,
        None => counts.push((key, 1)),
    }
}

/// A counter as JSON keys it (`None` as `"null"`), in first-seen order.
fn counter_object(counts: Vec<(Option<String>, u64)>) -> Value {
    let mut out = Map::new();
    for (key, n) in counts {
        out.insert(key.unwrap_or_else(|| "null".to_owned()), json!(n));
    }
    Value::Object(out)
}
