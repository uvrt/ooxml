//! Owns: a table's keeps as docx-agent's `edit/table_format.py` reads and writes them
//! (round 25, W2/W5; round 27, W7): per row, the paragraph Word decides the row's keep with
//! the next by and whether it keeps ([`row_keeps`]: `row_keeps`); whether those keep the
//! table together and with the next block ([`TableKeeps`]: `kept_together`,
//! `kept_with_next`); what `word_format_table`'s `keep_together` / `keep_with_next` write
//! there ([`plan`]: `set_keep_together`, `set_table_keep_with_next` and the tool's order
//! of them), each row's keep written directly only where its styles differ; and what is
//! read after the edit ([`table_keeps`], [`loose_paragraph_after`]; a table kept together's
//! pages, [`table_pages`], through the layout port).
//!
//! Must not: write but through the table's work, or read outside the table's story part
//! (the paragraph after a table aside).

use super::{find_table, Found};
use crate::fields::{EditLayout, Unknown};
use crate::format::values::write_paragraph;
use crate::format::{Property, Setting, Value};
use crate::names::W;
use crate::sink::EditSink;
use crate::validate::Orders;
use crate::work::{Id, Work};
use crate::{EditError, Result, WordDoc};
use ooxml_word_model::docx::formatting::Formatting;
pub(crate) use ooxml_word_model::docx::keeps::RowKeep;
pub use ooxml_word_model::docx::keeps::TableKeeps;
use ooxml_word_model::docx::keeps::{row_keeps as row_keeps_in, Keeps};
use ooxml_word_model::docx::{at_path, Docx};
use ooxml_xml::Element;

/// Per row of the table at `found`, its keep (docx-agent's `row_keeps`).
pub(crate) fn row_keeps(docx: &Docx, found: &Found) -> Result<Vec<RowKeep>> {
    rows_at(docx, &found.part, &found.path)
}

/// Per row of the table at `path` in `part`, its keep.
fn rows_at(docx: &Docx, part: &str, path: &[u32]) -> Result<Vec<RowKeep>> {
    let story = docx.part(part).ok_or_else(|| EditError::no_part(part.to_owned()))?;
    row_keeps_in(&Formatting::read(docx), story, path)
        .map(|rows| rows.to_vec())
        .ok_or_else(|| EditError::no_element(part.to_owned()))
}

/// What `keep_together` / `keep_with_next` write on a table: each row's direct keep
/// (`None`: removed), whether anything changes, and what is said.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct KeepWrite {
    /// `(row, value)`: the row's deciding paragraph's direct `w:keepNext`.
    pub(crate) changes: Vec<(usize, Option<bool>)>,
    /// What it says.
    pub(crate) warnings: Vec<String>,
}

/// The rows a table kept together writes on (docx-agent's `_keep_rows`).
fn keep_rows(rows: &[RowKeep], id: &str) -> Result<()> {
    if rows.len() < 2 {
        return Err(EditError::Refused(format!(
            "{id} has one row: nothing to keep together (cant_split keeps a row on one page)"
        )));
    }
    if rows.iter().any(|r| r.lead.is_none()) {
        return Err(EditError::Refused(format!(
            "{id} has a row with no paragraph to keep it by"
        )));
    }
    Ok(())
}

/// `rows` as they are once `changes` are written.
fn written(rows: &mut [RowKeep], changes: &[(usize, Option<bool>)]) {
    for &(k, value) in changes {
        if let Some(row) = rows.get_mut(k) {
            row.declared = value;
            row.keeps = value.unwrap_or(row.styled);
        }
    }
}

/// docx-agent's `set_keep_together`; `own_rows`, the table's rows as its index counts them.
fn set_keep_together(rows: &[RowKeep], id: &str, own_rows: usize, on: bool, with_next: bool) -> Result<KeepWrite> {
    let mut out = KeepWrite::default();
    if on && own_rows < 2 {
        out.warnings.push(format!(
            "{id} has one row: nothing to keep together, left as it was (cant_split keeps a row on one page; \
             keep_with_next on its paragraph, word_format, keeps it with the next block)"
        ));
        return Ok(out);
    }
    if !on && !TableKeeps::of(rows).together {
        out.warnings.push(format!("{id} was not kept together"));
        return Ok(out);
    }
    keep_rows(rows, id)?;
    let last = rows.len() - 1;
    for (k, row) in rows.iter().enumerate() {
        if on {
            let want = with_next || k < last;
            if row.keeps != want {
                out.changes.push((k, (row.styled != want).then_some(want)));
            }
        } else if row.declared.is_some_and(|d| d != row.styled) {
            out.changes.push((k, None));
        }
    }
    Ok(out)
}

/// docx-agent's `set_table_keep_with_next`; `released`, the warning a last row keeping
/// alone gets when released (said with the id the write leaves).
fn set_keep_with_next(rows: &[RowKeep], id: &str, on: bool) -> Result<(KeepWrite, bool)> {
    let mut out = KeepWrite::default();
    let keeps = TableKeeps::of(rows);
    let last = rows.len().saturating_sub(1);
    if !on {
        if !keeps.with_next {
            out.warnings.push(format!("{id}'s last row did not keep with the next"));
            return Ok((out, false));
        }
        if !keeps.together {
            let styled = rows.get(last).is_some_and(|r| r.styled);
            out.changes.push((last, styled.then_some(false)));
            return Ok((out, true));
        }
    }
    keep_rows(rows, id)?;
    if !keeps.together {
        return Err(EditError::Refused(format!(
            "{id} is not kept together: keep_with_next keeps a table kept together with the next block (its last \
             row alone would go without the rest); give keep_together true"
        )));
    }
    if let Some(row) = rows.get(last).filter(|r| r.keeps != on) {
        out.changes.push((last, (row.styled != on).then_some(on)));
    }
    Ok((out, false))
}

/// What `word_format_table`'s `keep_together` and `keep_with_next` write, in its order:
/// `keep_together` with `keep_with_next` as its `with_next`; off, both off, on a table that
/// was not kept together, a last row keeping alone released instead; `keep_with_next` alone
/// otherwise. `released` is true where the write releases a last row keeping alone (its
/// warning names the table's id after the write).
///
/// # Errors
/// docx-agent's refusals: a table to keep with the next that is not kept together, one row,
/// a row with no paragraph to keep it by.
pub(crate) fn plan(
    rows: &[RowKeep],
    id: &str,
    own_rows: usize,
    keep_together: Option<bool>,
    keep_with_next: Option<bool>,
) -> Result<(KeepWrite, bool)> {
    let Some(on) = keep_together else {
        return match keep_with_next {
            Some(on) => set_keep_with_next(rows, id, on),
            None => Ok((KeepWrite::default(), false)),
        };
    };
    let first = set_keep_together(rows, id, own_rows, on, keep_with_next == Some(true))?;
    if keep_with_next != Some(false) || !first.changes.is_empty() {
        return Ok((first, false));
    }
    let mut now = rows.to_vec();
    written(&mut now, &first.changes);
    if TableKeeps::of(&now).together {
        return Ok((first, false));
    }
    let (released, said) = set_keep_with_next(&now, id, false)?;
    if released.changes.is_empty() {
        return Ok((first, false));
    }
    Ok((released, said))
}

/// The warning a released last row gets (docx-agent's `set_table_keep_with_next`), `id`
/// the table's after the write.
pub(crate) fn released_note(id: &str) -> String {
    format!("{id} was not kept together: its last row's keep with the next is released")
}

/// Each row's deciding paragraph `changes` write on, found in the table `table` of
/// `work` (at `rows`' steps below it), with its direct `w:keepNext` to write.
pub(crate) fn leads(
    work: &Work,
    table: Id,
    rows: &[RowKeep],
    changes: &[(usize, Option<bool>)],
) -> Vec<(Id, Option<bool>)> {
    changes
        .iter()
        .filter_map(|&(k, value)| Some((node_at(work, table, rows.get(k)?.lead.as_ref()?)?, value)))
        .collect()
}

/// Each paragraph of `leads` given its direct `w:keepNext`, or none.
pub(crate) fn write(work: &mut Work, leads: &[(Id, Option<bool>)], orders: &Orders) {
    for &(paragraph, value) in leads {
        let setting = Setting {
            property: Property::KeepWithNext,
            value: value.map(Value::Flag),
        };
        write_paragraph(work, paragraph, &[setting], orders);
    }
}

fn node_at(work: &Work, from: Id, steps: &[u32]) -> Option<Id> {
    let mut node = from;
    for &k in steps {
        node = *work.children(node).get(k as usize)?;
    }
    work.is(node, W, "p").then_some(node)
}

/// Whether the table `table` names is kept together and with the next block, as its rows'
/// keeps read now (docx-agent's `table_properties`' `keep_together` / `keep_with_next`).
///
/// # Errors
/// A `table` that names no table.
pub fn table_keeps(doc: &WordDoc, table: &str) -> Result<TableKeeps> {
    let docx = doc.docx()?;
    let found = find_table(doc, &docx, table)?;
    Ok(TableKeeps::of(&row_keeps(&docx, &found)?))
}

/// [`table_keeps`] of the table at `path` in the story part `part` of `docx` (a reader's:
/// `word_describe`'s tables).
///
/// # Errors
/// A `part` or `path` that holds no table.
pub fn table_keeps_at(docx: &Docx, part: &str, path: &[u32]) -> Result<TableKeeps> {
    Ok(TableKeeps::of(&rows_at(docx, part, path)?))
}

/// A paragraph between a table and the next table that does not keep with the next
/// (docx-agent's `_loose_paragraph_after`, round 25 W5): `(table, paragraph, next table)`
/// by id; `None` where the two blocks after the table are not a paragraph and a table, or
/// the paragraph keeps with the next.
///
/// # Errors
/// A `table` that names no table.
pub fn loose_paragraph_after(doc: &WordDoc, table: &str) -> Result<Option<(String, String, String)>> {
    let docx = doc.docx()?;
    let found = find_table(doc, &docx, table)?;
    let story = docx
        .part(&found.part)
        .ok_or_else(|| EditError::no_part(found.part.clone()))?;
    let Some((&at, above)) = found.path.split_last() else {
        return Ok(None);
    };
    let Some(parent) = at_path(story.root(), above) else {
        return Ok(None);
    };
    let is = |e: &Element, local: &str| e.name().is(Some(W), local);
    let after: Vec<(Vec<u32>, &Element)> = parent
        .children()
        .iter()
        .enumerate()
        .skip(at as usize + 1)
        .filter_map(|(k, n)| Some((u32::try_from(k).ok()?, n.as_element()?)))
        .filter(|(_, e)| is(e, "p") || is(e, "tbl"))
        .take(2)
        .map(|(k, e)| ([above, &[k]].concat(), &**e))
        .collect();
    let [(p_at, paragraph), (t_at, next)] = after.as_slice() else {
        return Ok(None);
    };
    if !is(paragraph, "p") || !is(next, "tbl") {
        return Ok(None);
    }
    let formatting = Formatting::read(&docx);
    let mut look = Keeps::new(&formatting, story);
    if look.effective(paragraph, p_at, true).keep_with_next {
        return Ok(None);
    }
    let (Some(p), Some(t)) = (story.ids.paragraph_at(p_at), story.ids.table_at(t_at)) else {
        return Ok(None);
    };
    Ok(Some((found.id, p.id, t.id)))
}

/// The pages the body table `table` names is drawn on in the version `sink` holds (a
/// table kept together: docx-agent's `_taller_than_page`, read after the call's edits);
/// `None` for a table outside the body (a header's has no page to break). Where there is
/// no layout, or laying the version out stops or is not done, what the pages are is
/// [`Unknown`]: the keep is made all the same, and its warning says so (docx-agent's "no
/// layout: said, not guessed").
///
/// # Errors
/// A `table` that names no table.
pub(crate) fn table_pages(
    sink: &EditSink<'_, '_>,
    layout: Option<EditLayout<'_>>,
    table: &str,
) -> Result<Option<std::result::Result<Vec<usize>, Unknown>>> {
    let docx = sink.docx()?;
    let found = find_table(sink.doc(), &docx, table)?;
    if found.part != docx.main {
        return Ok(None);
    }
    let Some(layout) = layout else {
        return Ok(Some(Err(Unknown("no layout".to_owned()))));
    };
    Ok(Some(match layout.lay_out(sink.doc().package()) {
        Ok(laid) => laid.block_pages(&found.id),
        Err(stopped) => Err(Unknown(stopped.to_string())),
    }))
}

#[cfg(test)]
mod tests {
    use super::{plan, RowKeep};

    /// Rows whose keeps are `keeps`, their styles' `styled`, declared where they differ.
    fn rows(keeps: &[bool], styled: &[bool]) -> Vec<RowKeep> {
        keeps
            .iter()
            .zip(styled)
            .enumerate()
            .map(|(k, (&keeps, &styled))| RowKeep {
                lead: Some(vec![u32::try_from(k).unwrap_or(0), 0, 0]),
                keeps,
                styled,
                declared: (keeps != styled).then_some(keeps),
            })
            .collect()
    }

    #[test]
    fn released_directly_where_only_direct_keeps_keep() {
        let (write, released) = plan(&rows(&[true, true, false], &[false; 3]), "t:A", 3, Some(false), None).unwrap();
        assert_eq!(write.changes, vec![(0, None), (1, None)]);
        assert!(!released);
    }

    #[test]
    fn a_table_not_kept_together_is_said() {
        let (write, _) = plan(&rows(&[false; 3], &[false; 3]), "t:A", 3, Some(false), None).unwrap();
        assert!(write.changes.is_empty());
        assert_eq!(write.warnings, vec!["t:A was not kept together".to_owned()]);
    }

    #[test]
    fn a_last_row_keeping_alone_is_released_when_both_are_off() {
        let (write, released) = plan(
            &rows(&[false, false, true], &[false; 3]),
            "t:A",
            3,
            Some(false),
            Some(false),
        )
        .unwrap();
        assert_eq!(write.changes, vec![(2, None)]);
        assert!(released);
        let (styled, _) = plan(
            &rows(&[false, false, true], &[false, false, true]),
            "t:A",
            3,
            None,
            Some(false),
        )
        .unwrap();
        assert_eq!(styled.changes, vec![(2, Some(false))]);
    }

    #[test]
    fn keep_with_next_needs_a_table_kept_together() {
        let refused = plan(&rows(&[false; 3], &[false; 3]), "t:A", 3, None, Some(true));
        assert!(refused.is_err());
        let (write, _) = plan(&rows(&[true, true, false], &[false; 3]), "t:A", 3, None, Some(true)).unwrap();
        assert_eq!(write.changes, vec![(2, Some(true))]);
    }

    #[test]
    fn keeping_writes_only_where_the_styles_differ() {
        let (write, _) = plan(&rows(&[true, true, true], &[true; 3]), "t:A", 3, Some(true), None).unwrap();
        assert_eq!(write.changes, vec![(2, Some(false))]);
        let (one, _) = plan(&rows(&[false], &[false]), "t:A", 1, Some(true), None).unwrap();
        assert!(one.changes.is_empty() && one.warnings.len() == 1);
    }
}
