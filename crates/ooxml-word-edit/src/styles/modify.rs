//! Owns: a style's formatting changed as docx-agent's `Styles.modify` changes it
//! (`edit/styles.py`): run and paragraph formatting and shading (`None` removes a value),
//! its own tab stops replaced, a paragraph style linked to a list level or unlinked (the
//! styles that held the level, and those based on it, said), and a table style's
//! whole-table formatting and regions (`_modify_table`, no change said as none); and the
//! list level a call names checked (`_list_level`).
//!
//! Must not: add or remove a style.

use super::add::table_formatting;
use super::link::{self, Numbers, LEVELS};
use super::shading::{attributes, Shd};
use super::sheet::{self, quoted_name, Sheet};
use super::tabs::{self, TabStop};
use super::{invalid, not_found, ListLevel, RegionValues, StyleFormatting, StyleStep};
use crate::doc::Key;
use crate::format::{check, write_paragraph, write_run};
use crate::names::W;
use crate::review::space::Space;
use crate::sink::EditSink;
use crate::{EditError, Result};
use ooxml_base::pyfmt::repr_str;
use std::sync::Arc;

/// What `modify` changes.
pub(crate) struct Change<'a> {
    pub(crate) name: &'a str,
    pub(crate) kind: Option<&'a str>,
    pub(crate) numbering: Option<&'a ListLevel>,
    pub(crate) tabs: Option<&'a [TabStop]>,
    pub(crate) regions: &'a [RegionValues],
    pub(crate) formatting: &'a StyleFormatting,
}

fn refused(message: impl Into<String>) -> EditError {
    EditError::Refused(message.into())
}

const REL_NUMBERING: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/numbering";

/// The numbering part, read and copied into `space` (`None`: the document has none).
///
/// # Errors
/// When a part is not XML.
pub(crate) fn open_numbers(sink: &mut EditSink<'_, '_>, space: &mut Space) -> Result<Option<Numbers>> {
    let docx = sink.docx()?;
    let Some(part) = ooxml_word_model::parts::related(sink.doc().package(), &docx.main, REL_NUMBERING)? else {
        return Ok(None);
    };
    sink.read(Key::Part(Arc::from(part.as_str())));
    let held = sink.doc().clone();
    let Some(work) = space.work(&held, &part)? else {
        return Ok(None);
    };
    let root = work.root();
    Ok(Some(Numbers { part, root }))
}

/// The list level a call names, checked (`_list_level`): `(numId, level)`.
///
/// # Errors
/// [`EditError::Style`] naming `numbering.list` or `numbering.level`.
pub(crate) fn list_level(space: &Space, sheet: &Sheet, numbers: Option<&Numbers>, list: &str, level: i64) -> Result<(i64, i64)> {
    let at = |message: String, field: &str| invalid(message, Some(field), Vec::new());
    if !(0..LEVELS).contains(&level) {
        return Err(at(
            format!("numbering level is 0 to {} (the first level is 0), not {level}", LEVELS - 1),
            "numbering.level",
        ));
    }
    let known = link::list_numbers(space, numbers);
    let given = list.trim();
    // a number in digits is a list's number (the tool reads it so); anything else a style
    let digits = !given.is_empty() && given.chars().all(|c| c.is_ascii_digit());
    let num_id = match given.parse::<i64>() {
        Ok(n) if digits => n,
        _ => {
            let work = sheet.view(space)?;
            let Some(other) = sheet::find(work, sheet.root, given, Some("paragraph")) else {
                let mut error = not_found(format!(
                    "the document has no paragraph style {} (numbering list: a list's number or a style in it)",
                    repr_str(given)
                ));
                if let EditError::Style(r) = &mut error {
                    r.field = Some("numbering.list".to_owned());
                }
                return Err(error);
            };
            link::list_style(space, sheet, numbers, &other.id).ok_or_else(|| {
                at(
                    format!(
                        "{} is in no list: give a list's number, or a style in one",
                        other.name.as_deref().map_or_else(|| "None".to_owned(), repr_str)
                    ),
                    "numbering.list",
                )
            })?
        }
    };
    if !known.contains(&num_id) {
        let mut sorted = known.clone();
        sorted.sort_unstable();
        let listed: Vec<String> = sorted.iter().map(i64::to_string).collect();
        let listed = if listed.is_empty() { "none".to_owned() } else { listed.join(", ") };
        return Err(at(
            format!("the document has no list {num_id}: its lists are {listed}"),
            "numbering.list",
        ));
    }
    let levels = link::defined_levels(space, sheet, numbers, num_id);
    if !levels.contains(&level) {
        let listed: Vec<String> = levels.iter().map(i64::to_string).collect();
        let listed = if listed.is_empty() { "none".to_owned() } else { listed.join(", ") };
        return Err(at(
            format!("list {num_id} has no level {level}: its levels are {listed}"),
            "numbering.level",
        ));
    }
    Ok((num_id, level))
}

/// `modify` (docx-agent's `Styles.modify`).
pub(crate) fn modify(sink: &mut EditSink<'_, '_>, space: &mut Space, change: &Change<'_>) -> Result<StyleStep> {
    let missing = || refused(format!("the document has no style {}", repr_str(change.name)));
    let Some(sheet) = Sheet::open(sink, space)? else {
        return Err(missing());
    };
    let style = sheet::find(sheet.view(space)?, sheet.root, change.name, change.kind).ok_or_else(missing)?;
    let quoted = repr_str(change.name);
    if change.numbering.is_some() && style.kind != "paragraph" {
        return Err(refused(format!(
            "{quoted} is a {} style: only a paragraph style is linked to a list level",
            style.kind
        )));
    }
    if change.tabs.is_some() && style.kind != "paragraph" {
        return Err(refused(format!(
            "{quoted} is a {} style: only a paragraph style has tab stops",
            style.kind
        )));
    }
    if style.kind == "table" {
        return modify_table(space, &sheet, &style, change);
    }
    if !change.regions.is_empty() {
        return Err(refused(format!(
            "{quoted} is a {} style: regions are a table style's",
            style.kind
        )));
    }
    let Prepared {
        stops,
        numbers,
        link,
        shd,
    } = prepared(sink, space, &sheet, &style, change)?;
    let formatting = change.formatting;
    let orders = sheet.orders();
    {
        let work = sheet.work(space)?;
        write_paragraph(work, style.node, &formatting.paragraph, orders);
        let colour = |name: &str| sheet.colours.value(name);
        write_run(work, style.node, &formatting.run, &colour, orders);
        if let Some(shd) = &shd {
            super::shading::write_style_shading(work, style.node, &style.kind, shd.as_ref(), &sheet);
        }
        if let Some((stops, _)) = &stops {
            tabs::write(work, style.node, stops, orders);
        }
    }
    let mut warnings: Vec<String> = stops.map(|(_, w)| w).unwrap_or_default();
    let mut changed = true;
    match (change.numbering, link) {
        (Some(_), Some(level)) => {
            let numbers = numbers.as_ref().ok_or_else(|| refused(format!("the document has no list {}", level.0)))?;
            warnings.extend(linked(sink, space, &sheet, numbers, &style, level)?);
        }
        (Some(ListLevel::Unlink), _) if !link::unlink(space, &sheet, numbers.as_ref(), &style.id)? => {
            warnings.push(format!(
                "{} numbers nothing, neither its own nor through based_on: left as it was",
                repr_str(style.name.as_deref().unwrap_or(""))
            ));
            changed = !formatting.is_empty() || change.tabs.is_some();
        }
        _ => {}
    }
    let work = sheet.work(space)?;
    for local in ["pPr", "rPr"] {
        if let Some(child) = work.find(style.node, W, local) {
            if work.children(child).is_empty() {
                work.remove(child);
            }
        }
    }
    space.mark_dirty(&sheet.part);
    Ok(StyleStep {
        changed,
        warnings,
        ..StyleStep::of(Some(format!("style:{}", style.id)))
    })
}

/// What `modify` writes, checked before anything is: the tab stops (and their warnings),
/// the numbering part, the list level, the shading (`Some(None)`: removed).
struct Prepared {
    stops: Option<(Vec<tabs::Stop>, Vec<String>)>,
    numbers: Option<Numbers>,
    link: Option<(i64, i64)>,
    shd: Option<Option<Shd>>,
}

/// `modify`'s values checked (`Prepared`).
fn prepared(
    sink: &mut EditSink<'_, '_>,
    space: &mut Space,
    sheet: &Sheet,
    style: &sheet::Entry,
    change: &Change<'_>,
) -> Result<Prepared> {
    let quoted = repr_str(change.name);
    let formatting = change.formatting;
    if change.tabs.is_some_and(|t| t.iter().any(|s| s.position.is_none())) {
        sink.read_everything();
    }
    let docx = sink.docx()?;
    let stops = match change.tabs {
        Some(given) if !given.is_empty() => {
            let inherited = tabs::inherited(sheet.view(space)?, sheet.root, &style.id);
            Some(tabs::stops(given, &inherited, &mut || tabs::column_end(&docx))?)
        }
        Some(_) => Some((Vec::new(), Vec::new())),
        None => None,
    };
    let numbers = if change.numbering.is_some() {
        // the uses said in the warnings are the whole document's
        sink.read_everything();
        open_numbers(sink, space)?
    } else {
        None
    };
    let link = match change.numbering {
        Some(ListLevel::Link { list, level }) => Some(list_level(space, sheet, numbers.as_ref(), list, *level)?),
        _ => None,
    };
    check(&formatting.run)?;
    check(&formatting.paragraph)?;
    let shd = match &formatting.shading {
        Some(Some(value)) => Some(Some(attributes(value, &sheet.colours)?)),
        Some(None) => Some(None),
        None => None,
    };
    if !formatting.paragraph.is_empty() && !matches!(style.kind.as_str(), "paragraph" | "table") {
        return Err(refused(format!(
            "{quoted} is a {} style: it has no paragraph formatting",
            style.kind
        )));
    }
    if shd.is_some() && !matches!(style.kind.as_str(), "paragraph" | "character") {
        return Err(refused(format!(
            "{quoted} is a {} style: shade its tables with word_format_table, or a paragraph or character style",
            style.kind
        )));
    }
    Ok(Prepared {
        stops,
        numbers,
        link,
        shd,
    })
}

/// Style `style` linked to `(num_id, level)`, and the warnings: the styles that held the
/// level, those based on it numbering at it too (with their paragraphs).
fn linked(
    sink: &mut EditSink<'_, '_>,
    space: &mut Space,
    sheet: &Sheet,
    numbers: &Numbers,
    style: &sheet::Entry,
    (num_id, level): (i64, i64),
) -> Result<Vec<String>> {
    let mut warnings = Vec::new();
    for holder in link::link(space, sheet, numbers, &style.id, (num_id, level))? {
        let uses = super::remove::uses(sink, space, &holder)?;
        warnings.push(format!(
            "{} held level {level} of list {num_id} before: unlinked, its {uses} paragraph(s) are no longer numbered",
            quoted_name(sheet.view(space)?, sheet.root, &holder)
        ));
    }
    let named = repr_str(style.name.as_deref().unwrap_or(""));
    for heir in link::inheriting(space, sheet, Some(numbers), &style.id) {
        let uses = super::remove::uses(sink, space, &heir)?;
        warnings.push(format!(
            "{} is based on {named}: it numbers at level {level} of list {num_id} too, its {uses} paragraph(s) (numbering none on it stops that)",
            quoted_name(sheet.view(space)?, sheet.root, &heir)
        ));
    }
    Ok(warnings)
}

/// A table style's formatting and regions (`_modify_table`): no change said as none.
fn modify_table(space: &mut Space, sheet: &Sheet, style: &sheet::Entry, change: &Change<'_>) -> Result<StyleStep> {
    let (items, notes) = super::regions::check(change.regions)?;
    let work = sheet.work(space)?;
    let before = work.build_element(style.node);
    let mut warnings = table_formatting(work, sheet, style.node, change.formatting, &items, notes)?;
    let changed = work.build_element(style.node) != before;
    space.mark_dirty(&sheet.part);
    if !changed {
        warnings.push(format!(
            "{} already has this formatting: left as it was",
            repr_str(style.name.as_deref().unwrap_or(""))
        ));
    }
    Ok(StyleStep {
        changed,
        warnings,
        ..StyleStep::of(Some(format!("style:{}", style.id)))
    })
}
