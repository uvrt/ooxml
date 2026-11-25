//! Owns: [`InsertControl`], a content control in the line as docx-agent's `insert_control`
//! writes one (as Word 365 does): at a position of a range's first paragraph -- a run
//! there in the formatting new text takes, put outside a control it falls in (`hoist`) --
//! its properties (a check box's MS Gothic, the title, the tag, its number unlike every
//! control's, the kind's element), an empty `w:sdtEndPr`, its content (a drop-down's first
//! item, a check box's glyph) in that run's formatting. Tracked, the content is an
//! insertion; the control is no revision.
//!
//! Must not: write but through the sink.

use super::number::{next_number, NumbersControl};
use super::{rewrite, ControlEdited, InParagraph, Rewrite, CONTENT_TRACKED};
use crate::address_keys::names_by_id;
use crate::names::{W, W14};
use crate::policy::RevisionPolicy;
use crate::ranges::renamed_id;
use crate::review::xml::make;
use crate::sink::{EditSink, TextWrite};
use crate::stamping::stamp_document;
use crate::text::write::{check_text, make_run};
use crate::text_range::{range, Span};
use crate::work::{Id, Work};
use crate::{EditError, Result, WordDoc};
use ooxml_store::{Command, Txn};
use std::sync::Arc;

/// Word's check box glyphs (MS Gothic) as it writes a new one.
const CHECKED: &str = "2612";
const UNCHECKED: &str = "2610";
const CHECK_FONT: &str = "MS Gothic";

/// A content control's kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlKind {
    /// Plain text (`w:text`).
    Text,
    /// Rich text (`w:richText`).
    RichText,
    /// A drop-down list.
    DropDown,
    /// A combo box.
    ComboBox,
    /// A date.
    Date,
    /// A check box (`w14:checkbox`).
    Checkbox,
}

/// A content control inserted in the line.
#[derive(Debug, Clone)]
pub struct InsertControl {
    /// Where: a position, or a range (at its start).
    pub at: ooxml_word_model::address::Address,
    /// Its kind.
    pub kind: ControlKind,
    /// A list's items (each its display text and value).
    pub items: Vec<String>,
    /// Its tag (`w:tag`).
    pub tag: Option<String>,
    /// Its title (`w:alias`).
    pub alias: Option<String>,
    /// Untracked or tracked.
    pub policy: RevisionPolicy,
}

impl InsertControl {
    /// The text its content shows when made.
    fn content_text(&self) -> String {
        match self.kind {
            ControlKind::DropDown => self.items.first().cloned().unwrap_or_default(),
            ControlKind::Checkbox => glyph(UNCHECKED),
            _ => String::new(),
        }
    }

    fn check(&self, doc: &WordDoc) -> Result<(String, usize)> {
        if matches!(self.kind, ControlKind::DropDown | ControlKind::ComboBox) && self.items.is_empty() {
            return Err(EditError::Refused("a list control needs items".to_owned()));
        }
        check_text(&self.content_text())?;
        let docx = doc.docx()?;
        let found = range(doc, &docx, &self.at)?;
        let span = Span::of(doc, &docx, &found)?;
        let first = span.first().ok_or_else(|| EditError::no_element(span.part.clone()))?;
        Ok((first.id.clone(), span.start))
    }
}

impl Command<WordDoc> for InsertControl {
    type Out = ControlEdited;
    type Error = EditError;

    fn validate(&self, doc: &WordDoc) -> Result<()> {
        self.check(doc).map(|_| ())
    }

    fn prepare(&self, tx: &mut Txn<'_, WordDoc>) -> Result<ControlEdited> {
        let mut sink = EditSink::new(tx, self.policy.clone());
        let out = self.edit(&mut sink)?;
        crate::after::after_edit(&mut sink)?;
        Ok(out)
    }
}

impl InsertControl {
    fn edit(&self, sink: &mut EditSink<'_, '_>) -> Result<ControlEdited> {
        let (paragraph, offset) = self.check(sink.doc())?;
        let docx = sink.docx()?;
        // a place named by condition, or a range's end, depends on the document around it
        let by_id = names_by_id(self.at.as_str()) && !self.at.has(ooxml_word_model::address::Shape::Dots);
        if !by_id {
            sink.read_everything();
        }
        let renamed = stamp_document(sink, &docx)?;
        let docx = sink.docx()?;
        // the number is unlike every control's: read by condition, a control given it
        // meanwhile
        let number = next_number(&docx);
        if by_id {
            sink.read_where(Arc::new(NumbersControl { number }));
        }
        let found = crate::set_text::find(sink.doc(), &docx, &renamed_id(&renamed, &paragraph))?;
        let at = InParagraph {
            part: found.part,
            path: found.path,
            id: found.id,
        };
        let how = Rewrite {
            renew: true,
            ensure_w14: true,
        };
        rewrite(sink, &at, how, |sink, work| {
            let content = self.place(work, offset, number)?;
            let wrote = sink.track(work, |work, stamp, _| {
                let runs: Vec<Id> = work
                    .children(content)
                    .iter()
                    .copied()
                    .filter(|c| work.is(*c, W, "r"))
                    .collect();
                if !runs.is_empty() {
                    for run in &runs {
                        work.remove(*run);
                    }
                    crate::track::insert_nodes(work, content, 0, &runs, stamp);
                }
                Vec::new()
            })?;
            Ok(((), TextWrite { changed: true, ..wrote }))
        })?;
        let warnings = match &self.policy {
            RevisionPolicy::Tracked(_) => vec![CONTENT_TRACKED.to_owned()],
            RevisionPolicy::Untracked => Vec::new(),
        };
        let id = format!("cc:{number}");
        Ok(ControlEdited {
            id: Some(id.clone()),
            created: vec![id],
            removed: Vec::new(),
            renamed,
            warnings,
        })
    }

    /// The control made and put at `offset` of `work`'s paragraph: its content element.
    fn place(&self, work: &mut Work, offset: usize, number: i64) -> Result<Id> {
        let root = work.root();
        let run = crate::inline::new_run(work, root, offset, &[])?;
        hoist(work, run);
        let mut properties = work.find(run, W, "rPr");
        let gothic = (self.kind == ControlKind::Checkbox).then(|| gothic(work));
        if let Some(g) = gothic {
            if let Some(p) = properties {
                work.remove(p);
            }
            properties = Some(work.deep_copy(g));
        }
        let sdt = make(work, "sdt", &[]);
        let sdt_properties = make(work, "sdtPr", &[]);
        work.append(sdt, sdt_properties);
        if let Some(g) = gothic {
            work.append(sdt_properties, g);
        }
        for (local, value) in [("alias", &self.alias), ("tag", &self.tag)] {
            if let Some(value) = value.as_deref().filter(|v| !v.is_empty()) {
                let element = make(work, local, &[("val", value)]);
                work.append(sdt_properties, element);
            }
        }
        let id = make(work, "id", &[("val", &number.to_string())]);
        work.append(sdt_properties, id);
        let kind = self.kind_element(work);
        work.append(sdt_properties, kind);
        let end = make(work, "sdtEndPr", &[]);
        work.append(sdt, end);
        let content = make(work, "sdtContent", &[]);
        work.append(sdt, content);
        let text = self.content_text();
        if !text.is_empty() {
            let copied = properties.map(|p| work.deep_copy(p));
            let made = make_run(work, &text, copied);
            work.append(content, made);
        }
        work.add_previous(run, sdt);
        work.remove(run);
        Ok(content)
    }

    /// The element naming the control's kind in its properties (`_kind_element`).
    fn kind_element(&self, work: &mut Work) -> Id {
        match self.kind {
            ControlKind::Text => make(work, "text", &[]),
            ControlKind::RichText => make(work, "richText", &[]),
            ControlKind::DropDown | ControlKind::ComboBox => {
                let local = if self.kind == ControlKind::DropDown {
                    "dropDownList"
                } else {
                    "comboBox"
                };
                let list = make(work, local, &[]);
                for item in &self.items {
                    let element = make(work, "listItem", &[("displayText", item), ("value", item)]);
                    work.append(list, element);
                }
                list
            }
            ControlKind::Date => {
                let date = make(work, "date", &[]);
                for (local, value) in [
                    ("dateFormat", "d-M-yyyy"),
                    ("lid", "en-GB"),
                    ("storeMappedDataAs", "dateTime"),
                    ("calendar", "gregorian"),
                ] {
                    let element = make(work, local, &[("val", value)]);
                    work.append(date, element);
                }
                date
            }
            ControlKind::Checkbox => check_box(work),
        }
    }
}

/// A `w14:checkbox`, unchecked, with Word's glyphs.
fn check_box(work: &mut Work) -> Id {
    let make14 = |work: &mut Work, local: &str, attributes: &[(&str, &str)]| {
        let element = work.new_element(work.qname(W14, local, "w14"));
        for (name, value) in attributes {
            let name = work.qname(W14, name, "w14");
            work.set(element, name, value);
        }
        element
    };
    let checkbox = make14(work, "checkbox", &[]);
    for (local, attributes) in [
        ("checked", vec![("val", "0")]),
        ("checkedState", vec![("val", CHECKED), ("font", CHECK_FONT)]),
        ("uncheckedState", vec![("val", UNCHECKED), ("font", CHECK_FONT)]),
    ] {
        let child = make14(work, local, &attributes);
        work.append(checkbox, child);
    }
    checkbox
}

/// A check box's run properties: MS Gothic.
fn gothic(work: &mut Work) -> Id {
    let properties = make(work, "rPr", &[]);
    let fonts = make(
        work,
        "rFonts",
        &[
            ("ascii", CHECK_FONT),
            ("eastAsia", CHECK_FONT),
            ("hAnsi", CHECK_FONT),
            ("hint", "eastAsia"),
        ],
    );
    work.append(properties, fonts);
    properties
}

/// The character a glyph's hexadecimal code names.
fn glyph(code: &str) -> String {
    u32::from_str_radix(code, 16)
        .ok()
        .and_then(char::from_u32)
        .map(String::from)
        .unwrap_or_default()
}

/// A run that a position put inside a content control's content, put just outside the
/// control (before it at the content's start, else after it): what is made there is not
/// the control's content, which a fill replaces (`hoist`).
pub(crate) fn hoist(work: &mut Work, run: Id) {
    while let Some(content) = work.parent(run).filter(|p| work.is(*p, W, "sdtContent")) {
        let Some(sdt) = work.parent(content) else { return };
        if work.index(run) == Some(0) {
            work.add_previous(sdt, run);
        } else {
            work.add_next(sdt, run);
        }
    }
}
