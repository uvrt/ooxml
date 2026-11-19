//! Owns: a new style as docx-agent's `Styles.add` writes it (`edit/styles.py`): a
//! paragraph or character style (`w:customStyle`, its name, `w:basedOn`, `w:next`,
//! `w:qFormat`, its formatting, shading and tab stops) or a table style (`_add_table`: based
//! on Normal Table unless said, its whole-table formatting and regions); and the formatting
//! writer both `add` and `modify` give a table style (`_write_table_formatting`).
//!
//! Must not: change a style the document has (`super::modify` does).

use super::regions::{self, Checked};
use super::shading::attributes;
use super::sheet::{self, make, Sheet};
use super::tabs::{self, TabStop};
use super::{not_found, RegionValues, StyleFormatting, StyleStep};
use crate::builtin;
use crate::format::{check, write_paragraph, write_run};
use crate::review::space::Space;
use crate::sink::EditSink;
use crate::work::{Id, Work};
use crate::{EditError, Result};
use ooxml_base::pyfmt::repr_str;

/// What `add` makes.
pub(crate) struct NewStyle<'a> {
    pub(crate) name: &'a str,
    pub(crate) kind: &'a str,
    pub(crate) based_on: Option<&'a str>,
    pub(crate) next_style: Option<&'a str>,
    pub(crate) tabs: Option<&'a [TabStop]>,
    pub(crate) regions: &'a [RegionValues],
    pub(crate) formatting: &'a StyleFormatting,
}

fn refused(message: impl Into<String>) -> EditError {
    EditError::Refused(message.into())
}

/// A style named for an edit resolved, its refusal docx-agent's `StyleError`.
pub(crate) fn resolve(sink: &mut EditSink<'_, '_>, name: &str, kind: &str) -> Result<()> {
    let docx = sink.docx()?;
    builtin::resolve(&docx, name, kind).map(|_| ()).map_err(|e| match e {
        EditError::Refused(m) => not_found(m),
        other => other,
    })
}

/// The style sheet, made when the document has none (`_create_styles_part`).
fn open_sheet(sink: &mut EditSink<'_, '_>, space: &mut Space) -> Result<Sheet> {
    if let Some(sheet) = Sheet::open(sink, space)? {
        return Ok(sheet);
    }
    builtin::styles_part(sink)?;
    Sheet::open(sink, space)?.ok_or_else(|| EditError::NoPart("word/styles.xml".to_owned()))
}

fn check_name(sink: &mut EditSink<'_, '_>, name: &str) -> Result<()> {
    if name.trim().is_empty() || name.chars().count() > 253 {
        return Err(refused("a style name is 1 to 253 characters"));
    }
    let docx = sink.docx()?;
    let taken = sheet::find_in(&docx, name, None).is_some();
    if taken {
        return Err(refused(format!("the document already has a style {}", repr_str(name))));
    }
    Ok(())
}

/// The new `w:style` element appended to the sheet: type, `w:customStyle`, id, name and
/// base (`add`'s first writes).
fn append(work: &mut Work, sheet: &Sheet, kind: &str, id: &str, name: &str, base: Option<&str>) -> Id {
    let orders = sheet.orders();
    let node = make(work, "style", &[("type", kind), ("customStyle", "1"), ("styleId", id)]);
    let named = make(work, "name", &[("val", name)]);
    crate::marks::insert_in_order(work, node, named, orders);
    if let Some(base) = base {
        let based = make(work, "basedOn", &[("val", base)]);
        crate::marks::insert_in_order(work, node, based, orders);
    }
    work.append(sheet.root, node);
    node
}

/// `add` (docx-agent's `Styles.add`).
pub(crate) fn add(sink: &mut EditSink<'_, '_>, space: &mut Space, new: &NewStyle<'_>) -> Result<StyleStep> {
    if new.kind == "table" {
        return add_table(sink, space, new);
    }
    if !new.regions.is_empty() {
        return Err(refused(format!(
            "regions are a table style's: a {} style has none (kind table)",
            new.kind
        )));
    }
    if !matches!(new.kind, "paragraph" | "character") {
        return Err(refused("add() makes paragraph, character and table styles"));
    }
    if new.tabs.is_some_and(|t| !t.is_empty()) && new.kind != "paragraph" {
        return Err(refused("a character style has no tab stops"));
    }
    check_name(sink, new.name)?;
    if let Some(based_on) = new.based_on {
        resolve(sink, based_on, new.kind)?;
    }
    if let Some(next) = new.next_style {
        resolve(sink, next, "paragraph")?;
    }
    let formatting = new.formatting;
    check(&formatting.run)?;
    if !formatting.paragraph.is_empty() && new.kind != "paragraph" {
        return Err(refused("a character style has no paragraph formatting"));
    }
    check(&formatting.paragraph)?;
    let base = new.based_on.map(|b| builtin::ensure(sink, b, new.kind)).transpose()?;
    let following = new
        .next_style
        .map(|n| builtin::ensure(sink, n, "paragraph"))
        .transpose()?;
    let sheet = open_sheet(sink, space)?;
    let shd = match &formatting.shading {
        Some(Some(value)) => Some(attributes(value, &sheet.colours)?),
        _ => None,
    };
    let needs_column = new.tabs.is_some_and(|t| t.iter().any(|s| s.position.is_none()));
    if needs_column {
        sink.read_everything();
    }
    let docx = sink.docx()?;
    let work = sheet.work(space)?;
    let id = sheet::new_id(work, sheet.root, new.name);
    let node = append(work, &sheet, new.kind, &id, new.name, base.as_deref());
    let orders = sheet.orders();
    if let Some(following) = &following {
        let made = make(work, "next", &[("val", following)]);
        crate::marks::insert_in_order(work, node, made, orders);
    }
    let q = make(work, "qFormat", &[]);
    crate::marks::insert_in_order(work, node, q, orders);
    write_paragraph(work, node, &formatting.paragraph, orders);
    let colour = |name: &str| sheet.colours.value(name);
    write_run(work, node, &formatting.run, &colour, orders);
    if shd.is_some() {
        super::shading::write_style_shading(work, node, new.kind, shd.as_ref(), &sheet);
    }
    let mut warnings = Vec::new();
    if let Some(given) = new.tabs.filter(|t| !t.is_empty()) {
        let inherited = tabs::inherited(work, sheet.root, &id);
        let (stops, notes) = tabs::stops(given, &inherited, &mut || tabs::column_end(&docx))?;
        tabs::write(work, node, &stops, orders);
        warnings = notes;
    }
    Ok(StyleStep {
        created: vec![format!("style:{id}")],
        warnings,
        ..StyleStep::of(Some(format!("style:{id}")))
    })
}

/// A table style (`_add_table`).
fn add_table(sink: &mut EditSink<'_, '_>, space: &mut Space, new: &NewStyle<'_>) -> Result<StyleStep> {
    if new.next_style.is_some() {
        return Err(refused("next_style is a paragraph style's: a table style has none"));
    }
    if new.tabs.is_some_and(|t| !t.is_empty()) {
        return Err(refused(
            "a table style has no tab stops: give its paragraphs a paragraph style with them",
        ));
    }
    check_name(sink, new.name)?;
    if let Some(based_on) = new.based_on {
        resolve(sink, based_on, "table")?;
    }
    let (items, notes) = regions::check(new.regions)?;
    let base = match new.based_on {
        Some(b) => Some(builtin::ensure(sink, b, "table")?),
        None => sink
            .docx()?
            .default_style("table")
            .map(|s| ooxml_word_model::docx::style_id(s).to_owned()),
    };
    let sheet = open_sheet(sink, space)?;
    let work = sheet.work(space)?;
    let id = sheet::new_id(work, sheet.root, new.name);
    let node = append(work, &sheet, "table", &id, new.name, base.as_deref());
    let warnings = table_formatting(work, &sheet, node, new.formatting, &items, notes)?;
    Ok(StyleStep {
        created: vec![format!("style:{id}")],
        warnings,
        ..StyleStep::of(Some(format!("style:{id}")))
    })
}

/// A table style's whole-table formatting (run, paragraph, shading) and regions written on
/// `node` (`_write_table_formatting`); returns the notes.
pub(crate) fn table_formatting(
    work: &mut Work,
    sheet: &Sheet,
    node: Id,
    formatting: &StyleFormatting,
    items: &[Checked],
    mut notes: Vec<String>,
) -> Result<Vec<String>> {
    let orders = sheet.orders();
    check(&formatting.run)?;
    check(&formatting.paragraph)?;
    write_paragraph(work, node, &formatting.paragraph, orders);
    let colour = |name: &str| sheet.colours.value(name);
    write_run(work, node, &formatting.run, &colour, orders);
    if let Some(shading) = &formatting.shading {
        let whole = Checked {
            region: "whole_table",
            values: RegionValues {
                region: "whole_table".to_owned(),
                shading: Some(shading.clone().unwrap_or_else(|| "none".to_owned())),
                ..RegionValues::default()
            },
            sides: Vec::new(),
        };
        notes.extend(regions::write_region(work, node, &whole, sheet)?);
    }
    for item in items {
        notes.extend(regions::write_region(work, node, item, sheet)?);
    }
    let set: Vec<&str> = items.iter().filter(|i| i.values.sets()).map(|i| i.region).collect();
    notes.extend(regions::ensure_band_sizes(work, sheet.root, node, &set, orders));
    for local in ["pPr", "rPr"] {
        if let Some(child) = work.find(node, crate::names::W, local) {
            if work.children(child).is_empty() {
                work.remove(child);
            }
        }
    }
    Ok(notes)
}
