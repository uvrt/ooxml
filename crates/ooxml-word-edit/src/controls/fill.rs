//! Owns: [`FillControl`], a content control filled as docx-agent's `fill_control` fills
//! one, as typing into it would: plain or rich text (a line per paragraph in a block-level
//! control), a drop-down list's item (its display text or value), a combo box's item or any
//! text, a date (shown in the control's own picture, `w:fullDate` set), a check box's state
//! (its glyph and `w14:checked`). The content keeps its first run's formatting; a
//! placeholder shown goes. Tracked, the old text is deleted and the new inserted inside the
//! control, as Word records typing there.
//!
//! A data-bound control's custom XML node is written too ([`super::bound`]).
//!
//! Must not: write but through the sink. A control around table rows or cells is not
//! filled here: docx-agent writes a run among its rows; the host fills it
//! (`EditError::Unsupported`).

use super::bound::Binding;
use super::{located, property_in, rewrite, ControlEdited, InParagraph, Rewrite};
use crate::delete::stretch_keys;
use crate::fields::{format_date, Day};
use crate::names::{W, W14};
use crate::policy::RevisionPolicy;
use crate::sink::{EditSink, TextWrite};
use crate::stamping::stamp_document;
use crate::text::write::{check_text, make_run};
use crate::work::{Id, Work};
use crate::{EditError, Result, WordDoc};
use ooxml_base::pyfmt::repr_str;
use ooxml_store::{Command, Txn};
use ooxml_word_model::docx::controls::{control_kind, items, lock, property, Control, W14 as W14_MODEL};
use ooxml_word_model::docx::{at_path, Docx};

/// The kinds a fill writes text into.
const FILLED: [&str; 6] = ["text", "rich-text", "drop-down", "combo-box", "date", "checkbox"];
/// The placeholder styles a filled content's formatting drops (English and Dutch Word's).
const PLACEHOLDER_STYLES: [&str; 2] = ["PlaceholderText", "Tekstvantijdelijkeaanduiding"];

/// What a control is filled with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Filling {
    /// Text: a choice, a date (ISO 8601), any text.
    Text(String),
    /// A check box's state.
    Checked(bool),
}

/// A content control filled as typing into it would.
#[derive(Debug, Clone)]
pub struct FillControl {
    /// The control (`cc:101`).
    pub control: String,
    /// What it is filled with.
    pub value: Filling,
    /// Untracked or tracked.
    pub policy: RevisionPolicy,
}

/// What a fill writes: the text, and a date's `w:fullDate`.
struct Planned {
    text: String,
    full_date: Option<String>,
    checked: Option<bool>,
    /// What a bound control's node holds once filled.
    node: String,
}

impl FillControl {
    fn plan(&self, doc: &WordDoc, docx: &Docx) -> Result<Planned> {
        let found = located(docx, &self.control)?;
        let id = &self.control;
        let kind = control_kind(&found.element);
        if !FILLED.contains(&kind) {
            return Err(EditError::Refused(format!(
                "{id} is a {kind} control: its content is not filled as text"
            )));
        }
        if matches!(lock(&found.element), Some("contentLocked" | "sdtContentLocked")) {
            return Err(EditError::Refused(format!("{id}'s content is locked")));
        }
        let planned = self.text_of(&found, kind)?;
        let lines: Vec<&str> = planned.text.split('\n').collect();
        for line in &lines {
            check_text(line)?;
        }
        let part = docx
            .part(&found.part)
            .ok_or_else(|| EditError::NoPart(found.part.clone()))?;
        let level = found.level(part);
        if lines.len() > 1 && level != "block" {
            return Err(EditError::Refused(format!("{id} sits in a line: one line of text")));
        }
        if let Some(binding) = Binding::of(&found.element) {
            binding.node(doc, docx)?; // refused before anything changes
        }
        if matches!(level, "row" | "cell") {
            return Err(EditError::Unsupported(format!(
                "word_controls fill of a control around a table's {level}s"
            )));
        }
        Ok(planned)
    }

    /// The text the control shows once filled (and a date's `w:fullDate`).
    fn text_of(&self, found: &Control, kind: &str) -> Result<Planned> {
        let id = &self.control;
        let text = match &self.value {
            Filling::Checked(value) => {
                if kind != "checkbox" {
                    if *value { "True" } else { "False" }.to_owned()
                } else {
                    let local = if *value { "checkedState" } else { "uncheckedState" };
                    let state = property(&found.element, W14_MODEL, "checkbox")
                        .and_then(|b| b.find(Some(W14_MODEL), local))
                        .and_then(|s| s.attribute(Some(W14_MODEL), "val").map(str::to_owned));
                    let code = state.unwrap_or_else(|| if *value { "2612" } else { "2610" }.to_owned());
                    let glyph = u32::from_str_radix(&code, 16).ok().and_then(char::from_u32);
                    return Ok(Planned {
                        text: glyph.map(String::from).unwrap_or_default(),
                        full_date: None,
                        checked: Some(*value),
                        node: if *value { "true" } else { "false" }.to_owned(),
                    });
                }
            }
            Filling::Text(text) => text.clone(),
        };
        let mut planned = Planned {
            text: text.clone(),
            full_date: None,
            checked: None,
            node: text.clone(),
        };
        match kind {
            "checkbox" => {
                return Err(EditError::Refused(
                    "a check box is filled with True or False".to_owned(),
                ));
            }
            "date" => {
                let day = Day::parse(&text)
                    .filter(|_| text.get(..10).is_some_and(iso_date))
                    .ok_or_else(|| EditError::Refused(format!("{} is not a date", repr_str(&text))))?;
                let picture = property(&found.element, W, "date")
                    .and_then(|d| d.find(Some(W), "dateFormat"))
                    .and_then(|f| f.attribute(Some(W), "val"))
                    .unwrap_or("d-M-yyyy");
                planned.text = format_date(picture, day);
                planned.node = format!("{:04}-{:02}-{:02}T00:00:00", day.year, day.month, day.day);
                planned.full_date = Some(format!("{}Z", planned.node));
            }
            "drop-down" | "combo-box" => {
                let choices = items(&found.element);
                let choice = choices.iter().find(|(d, v)| *d == text || *v == text);
                match choice {
                    Some((display, value)) => {
                        planned.text.clone_from(display);
                        planned.node.clone_from(value);
                    }
                    None if kind == "drop-down" => {
                        let shown: Vec<String> = choices.iter().map(|(d, _)| repr_str(d)).collect();
                        return Err(EditError::Refused(format!(
                            "{} is not one of {id}'s items [{}]",
                            repr_str(&text),
                            shown.join(", ")
                        )));
                    }
                    None => {}
                }
            }
            _ => {}
        }
        Ok(planned)
    }
}

/// Whether `text` is a date as `date.fromisoformat` reads ten characters: `YYYY-MM-DD`,
/// a day the month has.
fn iso_date(text: &str) -> bool {
    let b = text.as_bytes();
    let shaped = b.len() == 10
        && b.iter().enumerate().all(|(k, c)| {
            if k == 4 || k == 7 {
                *c == b'-'
            } else {
                c.is_ascii_digit()
            }
        });
    let number = |range: std::ops::Range<usize>| text.get(range).and_then(|t| t.parse::<u32>().ok()).unwrap_or(0);
    let (year, month, day) = (number(0..4), number(5..7), number(8..10));
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let days = match month {
        2 if leap => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        1..=12 => 31,
        _ => 0,
    };
    shaped && year >= 1 && (1..=days).contains(&day)
}

impl Command<WordDoc> for FillControl {
    type Out = ControlEdited;
    type Error = EditError;

    fn validate(&self, doc: &WordDoc) -> Result<()> {
        self.plan(doc, &*doc.docx()?).map(|_| ())
    }

    fn prepare(&self, tx: &mut Txn<'_, WordDoc>) -> Result<ControlEdited> {
        let mut sink = EditSink::new(tx, self.policy.clone());
        let out = self.edit(&mut sink)?;
        crate::after::after_edit(&mut sink)?;
        Ok(out)
    }
}

impl FillControl {
    fn edit(&self, sink: &mut EditSink<'_, '_>) -> Result<ControlEdited> {
        let docx = sink.docx()?;
        let planned = self.plan(sink.doc(), &docx)?;
        let renamed = stamp_document(sink, &docx)?;
        let docx = sink.docx()?;
        let found = located(&docx, &self.control)?;
        let part = docx
            .part(&found.part)
            .ok_or_else(|| EditError::NoPart(found.part.clone()))?;
        if found.level(part) == "block" {
            self.fill_block(sink, &found, &planned)?;
        } else {
            let at = InParagraph::above(&docx, &found.part, &found.path)
                .ok_or_else(|| EditError::Path(found.part.clone()))?;
            let inside = found.path.get(at.path.len()..).unwrap_or_default().to_vec();
            rewrite(sink, &at, Rewrite::default(), |sink, work| {
                let sdt = super::at_place(work, &inside).ok_or_else(|| EditError::Path(at.part.clone()))?;
                let wrote = fill_inline(sink, work, sdt, &planned.text)?;
                set_properties(work, sdt, &planned);
                Ok(((), wrote))
            })?;
        }
        if let Some(binding) = Binding::of(&found.element) {
            binding.write(sink, &planned.node)?;
        }
        Ok(ControlEdited {
            id: Some(self.control.clone()),
            renamed,
            ..ControlEdited::default()
        })
    }

    /// A block-level control's paragraphs given a line each (keeping their properties),
    /// the lines left over following the last, the paragraphs left over going from the
    /// end (`_fill_block`); then its properties.
    fn fill_block(&self, sink: &mut EditSink<'_, '_>, found: &Control, planned: &Planned) -> Result<()> {
        let docx = sink.docx()?;
        let part = docx
            .part(&found.part)
            .ok_or_else(|| EditError::NoPart(found.part.clone()))?;
        let content = content_path(found).ok_or_else(|| EditError::Path(found.part.clone()))?;
        let ids: Vec<String> = part
            .ids
            .paragraphs()
            .iter()
            .filter(|p| p.path.len() == content.len() + 1 && p.path.starts_with(&content))
            .map(|p| p.id.clone())
            .collect();
        if ids.is_empty() {
            return Err(EditError::Unsupported(
                "word_controls fill of a block-level control without a paragraph".to_owned(),
            ));
        }
        let lines: Vec<&str> = planned.text.split('\n').collect();
        for (id, line) in ids.iter().zip(&lines) {
            crate::set_text::write_text(sink, id, line)?;
        }
        let mut last = ids
            .get(ids.len().min(lines.len()).saturating_sub(1))
            .cloned()
            .unwrap_or_default();
        for line in lines.iter().skip(ids.len()) {
            last = crate::paragraphs::insert_paragraph(sink, &last, (true, line, None))?.id;
        }
        for id in ids.iter().skip(lines.len()).rev() {
            crate::delete_write::delete(sink, id)?;
        }
        // the control's properties: its place found again, after its content changed
        let docx = sink.docx()?;
        let found = located(&docx, &self.control)?;
        let tree = sink
            .doc()
            .tree(&found.part)?
            .ok_or_else(|| EditError::NoPart(found.part.clone()))?;
        let at = |p: &[u32]| at_path(tree.root(), p).cloned();
        let k = sdt_properties(&found.element).ok_or_else(|| EditError::Path(found.part.clone()))?;
        let mut path = found.path.to_vec();
        path.push(k);
        let element = at(&path).ok_or_else(|| EditError::Path(found.part.clone()))?;
        let mut work = Work::of(&element, crate::work::scope_above(tree.root(), &path));
        let root = work.root();
        properties_of(&mut work, root, planned);
        if work.changed() {
            let new = work.build().ok_or_else(|| EditError::Path(found.part.clone()))?;
            let (&index, parent) = found
                .path
                .split_last()
                .ok_or_else(|| EditError::Path(found.part.clone()))?;
            let keys = stretch_keys(&docx, &found.part, parent, (index, index));
            sink.replace(&found.part, &path, new, keys)?;
        }
        Ok(())
    }
}

/// The place of a control's `w:sdtContent` in its part.
fn content_path(found: &Control) -> Option<Vec<u32>> {
    let k = found
        .element
        .children()
        .iter()
        .position(|n| matches!(n, ooxml_xml::Node::Element(e) if e.name().is(Some(W), "sdtContent")))?;
    let mut path = found.path.to_vec();
    path.push(u32::try_from(k).ok()?);
    Some(path)
}

/// The place of a `w:sdt`'s `w:sdtPr` among its children.
fn sdt_properties(sdt: &ooxml_xml::Element) -> Option<u32> {
    sdt.children()
        .iter()
        .position(|n| matches!(n, ooxml_xml::Node::Element(e) if e.name().is(Some(W), "sdtPr")))
        .and_then(|k| u32::try_from(k).ok())
}

/// A control in the line given `text` (`_fill_inline`): in its first text run's
/// formatting (else the control's), a placeholder's style dropped; untracked its content
/// replaced, tracked its shown runs deleted and the new run inserted after them.
fn fill_inline(sink: &mut EditSink<'_, '_>, work: &mut Work, sdt: Id, text: &str) -> Result<TextWrite> {
    let content = match work.find(sdt, W, "sdtContent") {
        Some(c) => c,
        None => {
            let made = crate::review::xml::make(work, "sdtContent", &[]);
            work.append(sdt, made);
            made
        }
    };
    let runs = work.iter(content, W, &["r"]);
    let template = runs
        .iter()
        .copied()
        .find(|r| work.find(*r, W, "t").is_some() || work.find(*r, W, "delText").is_some())
        .or_else(|| runs.first().copied());
    let source = template
        .and_then(|r| work.find(r, W, "rPr"))
        .or_else(|| property_in(work, sdt, W, "rPr"));
    let properties = source.map(|p| work.deep_copy(p));
    if let Some(p) = properties {
        for child in work.children(p).to_vec() {
            let placeholder = work.is(child, W, "rStyle")
                && work
                    .get(child, Some(W), "val")
                    .is_some_and(|v| PLACEHOLDER_STYLES.contains(&v));
            if placeholder || work.is(child, W, "rPrChange") {
                work.remove(child);
            }
        }
    }
    let properties = properties.filter(|p| !work.children(*p).is_empty());
    let new_run = (!text.is_empty()).then(|| make_run(work, text, properties));
    if !matches!(sink.policy(), RevisionPolicy::Tracked(_)) {
        for child in work.children(content).to_vec() {
            work.remove(child);
        }
        if let Some(run) = new_run {
            work.append(content, run);
        }
        return Ok(TextWrite {
            changed: true,
            ..TextWrite::default()
        });
    }
    let shown: Vec<Id> = runs
        .into_iter()
        .filter(|r| crate::track::revision_owner(work, *r).is_none_or(|o| work.is_any(o, W, &["ins", "moveTo"])))
        .collect();
    sink.track(work, |work, stamp, _| {
        let released = if shown.is_empty() {
            Vec::new()
        } else {
            crate::track::delete_runs(work, &shown, stamp)
        };
        if let Some(run) = new_run {
            let at = work.children(content).len();
            crate::track::insert_nodes(work, content, at, &[run], stamp);
        }
        released
    })
}

/// A filled control's properties (`sdt` in `work`): no placeholder shown, a check box's
/// state, a date's `w:fullDate`.
fn set_properties(work: &mut Work, sdt: Id, planned: &Planned) {
    if let Some(properties) = work.find(sdt, W, "sdtPr") {
        properties_of(work, properties, planned);
    }
}

/// [`set_properties`] on the `w:sdtPr` `properties`.
fn properties_of(work: &mut Work, properties: Id, planned: &Planned) {
    for node in work.children(properties).to_vec() {
        if work.is(node, W, "showingPlcHdr") {
            work.remove(node);
        }
    }
    if let (Some(value), Some(checkbox)) = (planned.checked, work.find(properties, W14, "checkbox")) {
        let state = match work.find(checkbox, W14, "checked") {
            Some(s) => s,
            None => {
                let made = work.new_element(work.qname(W14, "checked", "w14"));
                work.insert(checkbox, 0, made);
                made
            }
        };
        let name = work.qname(W14, "val", "w14");
        work.set(state, name, if value { "1" } else { "0" });
    }
    if let (Some(full), Some(date)) = (&planned.full_date, work.find(properties, W, "date")) {
        crate::review::xml::set_attr(work, date, "fullDate", full);
    }
}
