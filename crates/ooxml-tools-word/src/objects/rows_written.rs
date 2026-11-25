//! Owns: what `word_edit_table` does after a call that adds rows or writes into them
//! (docx-agent's `tools/objects.py`, `_rows_written`, round 58, R126): the rows the call
//! added or whose text it changed, told from the version it was applied to and the one it
//! made; their `w14:paraId`s kept in the host's record of the rows written this session
//! ([`RowsWritten`], which `check`'s `row_text_format` reads); text written into a cell that
//! was empty given its column's formatting ([`LikeTheColumn`], in the transaction of the
//! call's command: one commit, one step); and a warning for each written row whose text looks unlike the rows around
//! it, with the `word_format` call that makes it match.
//!
//! Only the main story's tables are looked at, as docx-agent looks at them.
//!
//! Must not: change the document but through one transaction, or read a row by its address
//! (rows are found by their places in the versions before and after).

use super::table_view::table_view;
use crate::review::Editing;
use ooxml_store::{Command, Txn};
use ooxml_tools::{python_dumps, TryCall};
use ooxml_word_edit::{
    row_shown, rows_unlike, ColumnCopy, EditError, LikeTheColumn, LookField, RowUnlike, TextLook, WordDoc,
};
use ooxml_word_model::docx::{at_path, Docx};
use ooxml_word_model::ids::{TableId, W14};
use serde_json::{Map, Value};
use std::collections::BTreeSet;
use std::sync::{Mutex, PoisonError};

/// Rows a warning names at most (the rest counted; check lists them).
const ROWS_SAID: usize = 3;

/// The rows a session's `word_edit_table` calls added or wrote into, by their
/// `w14:paraId`: kept by the host per document, never pruned (an undone row is simply not
/// found again), read by `check`'s `row_text_format`. Writers on several threads add to it
/// at once; a reader of an older version that does not hold a row named here finds nothing.
#[derive(Debug, Default)]
pub struct RowsWritten {
    ids: Mutex<BTreeSet<String>>,
}

impl RowsWritten {
    /// The rows written, by `w14:paraId`.
    #[must_use]
    pub fn ids(&self) -> BTreeSet<String> {
        self.ids.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    /// Keep `ids` as written.
    pub fn note<I: IntoIterator<Item = String>>(&self, ids: I) {
        self.ids.lock().unwrap_or_else(PoisonError::into_inner).extend(ids);
    }
}

/// A row's `w14:paraId`.
fn para_id(docx: &Docx, part: &str, path: &[u32]) -> Option<String> {
    let root = docx.part(part)?.root();
    at_path(root, path)?.attribute(Some(W14), "paraId").map(str::to_owned)
}

/// The text each of a table's own rows shows.
fn texts(docx: &Docx, part: &str, table: &TableId) -> Vec<String> {
    let Some(story) = docx.part(part) else {
        return Vec::new();
    };
    table
        .rows
        .iter()
        .map(|(path, _)| at_path(story.root(), path).map_or_else(String::new, |row| row_shown(row)))
        .collect()
}

/// The rows the call wrote: `made` the rows it added, if any (their ids); every other row
/// is the row at its place before, the added ones left out.
fn written_rows(before: &[String], after: &[String], after_ids: &[String], made: &[String]) -> Option<Vec<usize>> {
    let added: Vec<usize> = made
        .iter()
        .filter_map(|id| after_ids.iter().position(|r| r == id))
        .collect();
    if after.len() - added.len() != before.len() {
        return None;
    }
    let mut old = before.iter();
    Some(
        after
            .iter()
            .enumerate()
            .filter(|(k, text)| added.contains(k) || old.next() != Some(*text))
            .map(|(k, _)| k)
            .collect(),
    )
}

/// What looking at the rows a call wrote gave: the reply's warnings, in order, and the rows
/// to keep in the host's record once the call is committed (their `w14:paraId`s as its
/// transaction holds them).
#[derive(Debug, Default)]
struct Looked {
    warnings: Vec<String>,
    rows: Vec<String>,
}

/// A table `command`, then the rows it wrote looked at -- text written into empty cells
/// given its column's formatting in the same transaction: one commit, one history step, and
/// the rows kept in the host's record only once it is committed (as drawn). `table` is the
/// table as the call named it; `look` gives, from the command's out and the version it
/// made, the table's id after and the rows it added, `None` where the rows are not looked
/// at. The command's out and the warnings.
///
/// # Errors
/// The command's refusal.
pub(super) fn run_writing_rows<C, F>(
    editing: &Editing<'_>,
    command: &C,
    table: &str,
    look: F,
    retry: &TryCall,
) -> Result<(C::Out, Vec<String>), EditError>
where
    C: Command<WordDoc, Error = EditError>,
    F: Fn(&C::Out, &WordDoc) -> Option<(String, Vec<String>)>,
{
    let writing = WritingRows {
        command,
        editing,
        table,
        look,
        retry,
    };
    let done = editing.editor.run(editing.caller, &writing)?;
    let (out, looked) = done.out;
    if let Some(record) = editing.rows_written {
        record.note(looked.rows.iter().map(|row| done.drawn.resolve(row)));
    }
    Ok((out, looked.warnings))
}

/// A command, then the rows it wrote looked at, as one transaction ([`run_writing_rows`]).
struct WritingRows<'a, 'e, C, F> {
    command: &'a C,
    editing: &'a Editing<'e>,
    table: &'a str,
    look: F,
    retry: &'a TryCall,
}

impl<C, F> Command<WordDoc> for WritingRows<'_, '_, C, F>
where
    C: Command<WordDoc, Error = EditError>,
    F: Fn(&C::Out, &WordDoc) -> Option<(String, Vec<String>)>,
{
    type Out = (C::Out, Looked);
    type Error = EditError;

    fn validate(&self, doc: &WordDoc) -> Result<(), EditError> {
        self.command.validate(doc)
    }

    fn prepare(&self, tx: &mut Txn<'_, WordDoc>) -> Result<Self::Out, EditError> {
        let before = tx.doc().clone();
        let out = self.command.prepare(tx)?;
        if tx.changes().is_empty() {
            return Ok((out, Looked::default()));
        }
        let Some((now, made)) = (self.look)(&out, tx.doc()) else {
            return Ok((out, Looked::default()));
        };
        let looked = rows_written(self.editing, tx, &before, (self.table, &now), &made, self.retry);
        Ok((out, looked))
    }
}

/// What the call did beyond its command (`_rows_written`): `table` the table as the call
/// named it and its id after, `previous` the version its command was applied to and `tx`
/// the one it made, `made` the rows it added.
fn rows_written(
    editing: &Editing<'_>,
    tx: &mut Txn<'_, WordDoc>,
    previous: &WordDoc,
    (table, now): (&str, &str),
    made: &[String],
    retry: &TryCall,
) -> Looked {
    let (Ok(old_docx), Ok(docx)) = (previous.docx(), tx.doc().docx()) else {
        return Looked::default();
    };
    let (Ok(old), Ok(new)) = (
        table_view(previous, &old_docx, table, retry),
        table_view(tx.doc(), &docx, now, retry),
    ) else {
        return Looked::default();
    };
    if new.part != docx.main {
        return Looked::default();
    }
    let row_ids: Vec<String> = new.table.rows.iter().map(|(_, id)| id.clone()).collect();
    let Some(now_rows) = written_rows(
        &texts(&old_docx, &old.part, &old.table),
        &texts(&docx, &new.part, &new.table),
        &row_ids,
        made,
    ) else {
        return Looked::default();
    };
    if now_rows.is_empty() {
        return Looked::default();
    }
    let ids: Vec<Option<String>> = new
        .table
        .rows
        .iter()
        .map(|(path, _)| para_id(&docx, &new.part, path))
        .collect();
    let mut kept = editing.rows_written.map(RowsWritten::ids).unwrap_or_default();
    let mine: Vec<String> = now_rows.iter().filter_map(|r| ids.get(*r).cloned().flatten()).collect();
    kept.extend(mine.iter().cloned());
    let marked: Vec<bool> = (0..ids.len())
        .map(|r| now_rows.contains(&r) || ids.get(r).and_then(Option::as_ref).is_some_and(|i| kept.contains(i)))
        .collect();
    let ours =
        |found: Vec<RowUnlike>| -> Vec<RowUnlike> { found.into_iter().filter(|e| now_rows.contains(&e.row)).collect() };
    let mut unlike = ours(rows_unlike(&docx, &new.part, &new.table, &marked));
    let mut out = like_the_column(tx, now, &unlike, &ids);
    if !out.is_empty() {
        if let Ok(docx) = tx.doc().docx() {
            if let Ok(view) = table_view(tx.doc(), &docx, now, retry) {
                unlike = ours(rows_unlike(&docx, &view.part, &view.table, &marked));
            }
        }
    }
    for found in unlike.iter().take(ROWS_SAID) {
        let has = merged(found, |c| &c.has);
        let around = merged(found, |c| &c.around);
        let calls: Vec<Value> = found.fix.iter().map(call_json).collect();
        out.push(format!(
            "row {} of {}: its text is {} where the rows around it are {}; word_format makes it match: {}",
            found.row,
            found.table,
            said(&has),
            said(&around),
            python_dumps(&Value::Array(calls))
        ));
    }
    if unlike.len() > ROWS_SAID {
        out.push(format!(
            "{} more rows unlike the rows around them: check's row_text_format lists them",
            unlike.len() - ROWS_SAID
        ));
    }
    Looked {
        warnings: out,
        rows: mine,
    }
}

/// Text written into cells that were empty given its column's formatting, in the call's
/// transaction (left out, the call going on, where it refuses); what it did, said per cell.
fn like_the_column(
    tx: &mut Txn<'_, WordDoc>,
    table: &str,
    unlike: &[RowUnlike],
    ids: &[Option<String>],
) -> Vec<String> {
    let cells: Vec<(ColumnCopy, String)> = unlike
        .iter()
        .flat_map(|found| {
            found.cells.iter().map(|cell| {
                let copy = ColumnCopy {
                    row: found.row,
                    row_id: ids.get(found.row).cloned().flatten(),
                    start: cell.start,
                    from: cell.from,
                };
                let note = format!(
                    "{}: written in its column's formatting, row {}'s ({}); Word would give it the document default \
                     ({}) -- the cell was empty, its mark unformatted",
                    cell.cell,
                    cell.from,
                    said(&fields(&cell.around, &cell.fields)),
                    said(&fields(&cell.has, &cell.fields))
                );
                (copy, note)
            })
        })
        .collect();
    if cells.is_empty() {
        return Vec::new();
    }
    let command = LikeTheColumn {
        table: ooxml_word_model::address::Address::parse(table),
        cells: cells.iter().map(|(c, _)| c.clone()).collect(),
    };
    let point = tx.savepoint();
    let written = match command.validate(tx.doc()).and_then(|()| command.prepare(tx)) {
        Ok(written) => written,
        Err(_) => {
            tx.rollback(point);
            return Vec::new();
        }
    };
    written
        .iter()
        .zip(cells)
        .filter(|(written, _)| **written)
        .map(|(_, (_, note))| note)
        .collect()
}

/// A look's values, by field.
type Said = Vec<(LookField, Value)>;

fn value(look: &TextLook, field: LookField) -> Value {
    match field {
        LookField::Font => look.font.as_deref().map_or(Value::Null, Value::from),
        LookField::Size => Value::from(look.size),
        LookField::Color => Value::from(look.color.as_str()),
    }
}

fn fields(look: &TextLook, which: &[LookField]) -> Said {
    which.iter().map(|f| (*f, value(look, *f))).collect()
}

/// The row's cells' `has` (or `around`) as one dict, as docx-agent merges them: a later
/// cell's value for a field replaces the earlier, in its place.
fn merged(found: &RowUnlike, look: impl Fn(&ooxml_word_edit::CellUnlike) -> &TextLook) -> Said {
    let mut out: Said = Vec::new();
    for cell in &found.cells {
        for (field, v) in fields(look(cell), &cell.fields) {
            match out.iter_mut().find(|(f, _)| *f == field) {
                Some(entry) => entry.1 = v,
                None => out.push((field, v)),
            }
        }
    }
    out
}

/// docx-agent's `_look_said`: the face, the size in points, the colour.
fn said(look: &Said) -> String {
    let get = |f: LookField| look.iter().find(|(x, _)| *x == f).map(|(_, v)| v);
    let mut parts = Vec::new();
    if let Some(font) = get(LookField::Font) {
        parts.push(font.as_str().map_or_else(|| "None".to_owned(), str::to_owned));
    }
    if let Some(size) = get(LookField::Size) {
        parts.push(format!("{} pt", py_g(size.as_f64().unwrap_or(0.0))));
    }
    if let Some(color) = get(LookField::Color) {
        parts.push(format!("colour {}", color.as_str().unwrap_or_default()));
    }
    parts.join(" ")
}

/// Python's `f"{size:g}"` of a size in half points: no decimals where whole.
fn py_g(size: f64) -> String {
    if size.fract() == 0.0 && size.abs() < 1e15 {
        format!("{size:.0}")
    } else {
        format!("{size}")
    }
}

/// A fix call as docx-agent writes it: `{"targets": [...], <argument>: <value>, ...}`.
pub(crate) fn call_json(call: &ooxml_word_edit::FormatCall) -> Value {
    let mut map = Map::new();
    map.insert("targets".into(), Value::from(call.targets.clone()));
    for field in &call.fields {
        map.insert(field.argument().into(), value(&call.look, *field));
    }
    Value::Object(map)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn rows_written_are_told_by_place_with_the_added_row_left_out() {
        let before = ["a", "b", "c"].map(String::from);
        let after = ["a", "x", "b", "c2"].map(String::from);
        let ids = ["r0", "new", "r1", "r2"].map(String::from);
        assert_eq!(
            written_rows(&before, &after, &ids, &["new".to_owned()]),
            Some(vec![1, 3])
        );
        assert_eq!(written_rows(&before, &before, &ids[..3], &[]), Some(vec![]));
        assert_eq!(written_rows(&before, &after, &ids, &[]), None);
    }

    #[test]
    fn two_writers_at_once_both_keep_their_rows() {
        let record = Arc::new(RowsWritten::default());
        let writers: Vec<_> = (0..2)
            .map(|w| {
                let record = Arc::clone(&record);
                std::thread::spawn(move || {
                    for k in 0..100 {
                        record.note([format!("{w}{k:07X}")]);
                    }
                })
            })
            .collect();
        for writer in writers {
            writer.join().expect("writer");
        }
        assert_eq!(record.ids().len(), 200);
    }

    #[test]
    fn sizes_are_said_as_python_g_says_them() {
        assert_eq!(py_g(8.0), "8");
        assert_eq!(py_g(10.5), "10.5");
    }
}
