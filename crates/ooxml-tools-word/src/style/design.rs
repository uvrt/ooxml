//! Owns: `word_design_facts` (docx-agent's `edit/design.py` `design_facts` and its tool in
//! `tools/style.py`): the facts of the kinds asked for, measured, with ids, no verdicts --
//! long lists capped (`omitted` counting the rest) and the caps halved until the reply
//! fits; `declared_styles` paged by what fits, `next_cursor` for the rest; a paragraph's
//! `keep_together` said as `word_format` says it (`keep_lines`).
//!
//! Each kind is its module's: [`super::design_styles`] (styles per role, declared styles),
//! [`super::design_tables`] (table styles by place), [`super::design_layout`] (table frames,
//! sections, theme, patterns), each look said by [`super::design_look`].
//!
//! Must not: change a document, or judge a design.

use super::design_args::{DesignKind, WordDesignFacts};
use super::design_layout::{patterns, section_facts, table_frames, theme, usual_width};
use super::design_look::Design;
use super::design_styles::{declared_styles, styles};
use super::design_tables::table_styles;
use crate::paging::page_list;
use crate::version::Version;
use ooxml_base::Cancel;
use ooxml_tools::{ArgEnum, Done, ToolError, TryCall};
use serde_json::{Map, Value};
use std::collections::HashMap;

/// Room left for the envelope when the facts are fitted to the reply's characters.
const MARGIN: i64 = 1_500;

/// How many entries of each list are reported; `omitted` counts the rest.
fn cap(kind: DesignKind) -> usize {
    match kind {
        DesignKind::Styles => 60,
        DesignKind::TableStyles => 12,
        DesignKind::TableFrames => 30,
        DesignKind::Sections => 20,
        DesignKind::Patterns => 12,
        DesignKind::DeclaredStyles => 300,
        DesignKind::Theme => 0,
    }
}

/// The facts of one version, each kind read once (a smaller cap only takes fewer).
struct Facts<'d> {
    design: Design<'d>,
    lists: HashMap<DesignKind, Vec<Value>>,
}

impl Facts<'_> {
    fn list(&mut self, kind: DesignKind, cancel: &dyn Cancel) -> Result<&Vec<Value>, ToolError> {
        if !self.lists.contains_key(&kind) {
            let made = match kind {
                DesignKind::Styles => styles(&self.design, cancel)?,
                DesignKind::TableStyles => table_styles(&self.design, cancel)?,
                DesignKind::TableFrames => table_frames(&self.design),
                DesignKind::Sections => section_facts(&self.design),
                DesignKind::Patterns => patterns(&self.design),
                DesignKind::DeclaredStyles => declared_styles(&self.design, cancel)?,
                DesignKind::Theme => Vec::new(),
            };
            self.lists.insert(kind, made);
        }
        Ok(self.lists.entry(kind).or_default())
    }

    /// The facts of `kinds` (in the order reported), each list at most its cap.
    fn facts(
        &mut self,
        kinds: &[DesignKind],
        caps: &HashMap<DesignKind, usize>,
        cancel: &dyn Cancel,
    ) -> Result<Map<String, Value>, ToolError> {
        let mut out = Map::new();
        let mut omitted = Map::new();
        for kind in DesignKind::ALL.into_iter().filter(|k| kinds.contains(k)) {
            if kind == DesignKind::Theme {
                out.insert("theme".into(), theme(&self.design));
                continue;
            }
            let limit = caps.get(&kind).copied().unwrap_or_else(|| cap(kind));
            let items = self.list(kind, cancel)?;
            if items.len() > limit {
                omitted.insert(kind.as_str().into(), Value::from(items.len() - limit));
            }
            out.insert(
                kind.as_str().into(),
                Value::from(items.iter().take(limit).cloned().collect::<Vec<_>>()),
            );
        }
        if out.contains_key("sections") {
            let even = self.design.formatting().context().settings.even_and_odd_headers;
            out.insert("even_and_odd_headers".into(), Value::Bool(even));
        }
        if let Some(Value::Array(frames)) = out.get("table_frames") {
            if let Some(usual) = usual_width(frames) {
                out.insert("usual_table_width".into(), usual);
            }
        }
        if !omitted.is_empty() {
            out.insert("omitted".into(), Value::Object(omitted));
        }
        Ok(out)
    }
}

/// The length of `json.dumps(value, ensure_ascii=False)`, in characters.
fn python_length(value: &Value) -> usize {
    fn separators(value: &Value) -> usize {
        match value {
            Value::Array(items) => items.len().saturating_sub(1) + items.iter().map(separators).sum::<usize>(),
            Value::Object(map) => map.len().saturating_sub(1) + map.len() + map.values().map(separators).sum::<usize>(),
            _ => 0,
        }
    }
    value.to_string().chars().count() + separators(value)
}

/// `data` with `keep_together` said as `keep_lines` in every `look`, at any depth.
fn tool_names(value: Value) -> Value {
    match value {
        Value::Array(items) => Value::Array(items.into_iter().map(tool_names).collect()),
        Value::Object(map) => Value::Object(
            map.into_iter()
                .map(|(key, value)| {
                    let value = match (key.as_str(), value) {
                        ("look", Value::Object(look)) => Value::Object(
                            look.into_iter()
                                .map(|(k, v)| (if k == "keep_together" { "keep_lines".into() } else { k }, v))
                                .collect(),
                        ),
                        (_, value) => value,
                    };
                    (key, tool_names(value))
                })
                .collect(),
        ),
        other => other,
    }
}

fn spaced(kind: &str) -> String {
    kind.replace('_', " ")
}

/// `word_design_facts` on a Word document at one version.
///
/// # Errors
/// `invalid_arguments` for a cursor no page gave; `timeout` when cancelled.
pub fn word_design_facts(
    args: &WordDesignFacts,
    version: &Version<'_>,
    cancel: &dyn Cancel,
) -> Result<Done<Value>, ToolError> {
    let docx = version.docx()?;
    let mut facts = Facts {
        design: Design::new(&docx),
        lists: HashMap::new(),
    };
    let kinds: Vec<DesignKind> = match args.include.as_deref() {
        Some(asked) if !asked.is_empty() => asked.to_vec(),
        _ => DesignKind::DEFAULT.to_vec(),
    };
    let declared = kinds.contains(&DesignKind::DeclaredStyles) || args.cursor.is_some();
    let others: Vec<DesignKind> = if args.cursor.is_some() {
        Vec::new()
    } else {
        kinds.into_iter().filter(|k| *k != DesignKind::DeclaredStyles).collect()
    };
    let mut caps: HashMap<DesignKind, usize> = DesignKind::ALL.iter().map(|k| (*k, cap(*k))).collect();
    caps.remove(&DesignKind::Theme);
    let mut data = Map::new();
    // The reply's characters less the envelope's room, as docx-agent's `max_result_chars -
    // 1500` (below zero for a tiny limit: the caps then shrink to 1 each, as there).
    let limit = i64::try_from(version.limits.result_chars).unwrap_or(i64::MAX) - MARGIN;
    let fits = if declared { limit.div_euclid(2) } else { limit };
    while !others.is_empty() {
        data = facts.facts(&others, &caps, cancel)?;
        let length = i64::try_from(python_length(&Value::Object(data.clone()))).unwrap_or(i64::MAX);
        if length <= fits || caps.values().max().is_none_or(|m| *m <= 1) {
            break;
        }
        for value in caps.values_mut() {
            *value = (*value / 2).max(1);
        }
    }
    let mut counts: Vec<String> = ["styles", "table_styles", "table_frames", "sections", "patterns"]
        .iter()
        .filter_map(|k| Some(format!("{} {}", data.get(*k)?.as_array()?.len(), spaced(k))))
        .collect();
    let mut done = Done::new(String::new());
    if declared {
        let all = facts.list(DesignKind::DeclaredStyles, cancel)?.clone();
        let (page, total, _) = page_list(&all, args.cursor.as_deref(), all.len(), first_page(args))?;
        let start = all.len() - page.len();
        let mut room = limit - python_length(&Value::Object(data.clone())) as i64;
        let mut kept = Vec::new();
        for entry in page {
            room -= python_length(&entry) as i64 + 2;
            if room < 0 && !kept.is_empty() {
                break;
            }
            kept.push(entry);
        }
        let end = start + kept.len();
        counts.push(if kept.is_empty() {
            format!("no declared styles of {total}")
        } else {
            format!("declared styles {}-{end} of {total}", start + 1)
        });
        data.insert("declared_styles".into(), Value::Array(kept));
        done.total = Some(total as u64);
        done.next_cursor = (end < total).then(|| format!("c{end}"));
    }
    let left = match data.get("omitted") {
        Some(Value::Object(left)) if !left.is_empty() => {
            let listed: Vec<String> = left.iter().map(|(k, n)| format!("{n} more {}", spaced(k))).collect();
            format!("; {} left out (include fewer kinds)", listed.join(", "))
        }
        _ => String::new(),
    };
    done.summary = format!("Design facts: {}{left}", counts.join(", "));
    done.data = Some(tool_names(Value::Object(data)));
    Ok(done)
}

/// The call for the first page.
fn first_page(args: &WordDesignFacts) -> TryCall {
    let mut arguments = Map::new();
    arguments.insert("doc".into(), Value::from(args.doc.as_str()));
    if let Some(include) = &args.include {
        let kinds: Vec<Value> = include.iter().map(|k| Value::from(k.as_str())).collect();
        arguments.insert("include".into(), Value::Array(kinds));
    }
    TryCall {
        tool: "word_design_facts".into(),
        arguments,
        fill: Vec::new(),
    }
}

/// Registers `word_design_facts` (the crate's [`crate::register`]).
pub(crate) fn register(r: &mut impl ooxml_tools::Registrar<crate::Word>) {
    r.read::<WordDesignFacts>(word_design_facts);
}
