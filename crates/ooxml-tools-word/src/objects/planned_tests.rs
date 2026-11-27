//! Forced interleavings of the object tools' planned edits ([`crate::planned::Planned`]): a
//! handler's plan held after its first prepare while another writer's edit commits
//! ([`held::raced`]), then committed. Each plan reads what it works out its edits from in
//! the edit's transaction, so the edit meanwhile is a conflict and the plan, made again on
//! the newer version, does what running the two one at a time does. A plan worked out on
//! the head before the command runs (as the handlers did) is shown stale there.
// Tests may unwrap, panic and index (docs/architecture/README.md, section 2.5).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use crate::planned::held::{calling as editing, markdown, opened, raced};
use crate::planned::run_planned;
use crate::review::Editing;
use crate::sources::{Cell, SourceRows};
use ooxml_store::{Caller, Command, Txn};
use ooxml_tools::{ToolError, TryCall};
use ooxml_word_edit::{EditError, EditTable, Editor, RevisionPolicy, Setup, TableChange, WordDoc};
use ooxml_word_model::address::Address;
use ooxml_word_model::docx::{at_path, find_path};
use ooxml_word_model::ids::W;
use serde_json::json;

pub(super) use crate::planned::held::args;

/// An editor of a document whose body ends with a table of a header row and the rows
/// `r1` to `r4` (two columns: the row's name, then `x`), its every block stamped; the
/// table's id.
pub(super) fn with_table() -> (Editor, String) {
    let editor = opened("ids-and-markup.docx");
    let table = "| name | value |\n|---|---|\n| r1 | x |\n| r2 | x |\n| r3 | x |\n| r4 | x |\n";
    editor
        .store()
        .run(&Caller::anonymous(), &markdown("end", table))
        .unwrap();
    let docx = editor.store().snapshot().doc().docx().unwrap();
    let table = docx.body().unwrap().ids.tables().iter().last().unwrap().id;
    (editor, table)
}

/// A row put in at the top of `table`'s body (after the header row): every body row a place
/// further down.
pub(super) fn row_on_top(table: &str) -> EditTable {
    EditTable {
        table: Address::parse(table),
        change: TableChange::InsertRow {
            index: Some(0),
            below: true,
            plain: true,
        },
        policy: RevisionPolicy::Untracked,
    }
}

/// One row of a table as the tests compare it: its cells' text and fills (`w:shd/@w:fill`),
/// and whether it may not split across pages (`w:cantSplit`).
pub(super) type Row = (Vec<(String, Option<String>)>, bool);

/// `table`'s rows on `editor`'s head.
pub(super) fn rows_of(editor: &Editor, table: &str) -> Vec<Row> {
    let head = editor.store().snapshot();
    let docx = head.doc().docx().unwrap();
    let story = docx.body().unwrap();
    let found = story.ids.tables().iter().find(|t| t.id == table).unwrap().clone();
    found
        .rows
        .iter()
        .map(|(path, _)| {
            let row = at_path(story.root(), path).unwrap();
            let cells = row
                .child_elements()
                .filter(|c| c.name().is(Some(W), "tc"))
                .map(|cell| {
                    let text = texts(cell);
                    let fill =
                        find_path(cell, &["tcPr", "shd"]).and_then(|s| s.attribute(Some(W), "fill").map(str::to_owned));
                    (text, fill)
                })
                .collect();
            (cells, find_path(row, &["trPr", "cantSplit"]).is_some())
        })
        .collect()
}

/// The text of `element`'s `w:t`s, in order.
fn texts(element: &ooxml_xml::Element) -> String {
    if element.name().is(Some(W), "t") {
        return ooxml_xml::view::text(element);
    }
    element.child_elements().map(|c| texts(c)).collect()
}

/// `plan` worked out on the head and then run after `between` committed -- the handlers'
/// way before the plans moved into the command: whether it ran.
pub(super) fn planned_before<C, P, B>(
    editing: &Editing<'_>,
    plan: impl Fn(&mut Txn<'_, WordDoc>) -> Result<(C, P), ToolError>,
    between: &B,
) -> bool
where
    C: Command<WordDoc, Error = EditError>,
    B: Command<WordDoc, Error = EditError>,
{
    let mut head = editing.editor.store().begin(&Setup::default());
    let Ok((baked, _)) = plan(&mut head) else {
        panic!("the plan refused on the head")
    };
    editing.editor.store().run(&Caller::named("between"), between).unwrap();
    editing.editor.run(editing.caller, &baked).is_ok()
}

/// `plan` run alone through the call's editor: what a call does one at a time.
pub(super) fn alone<C, P>(editing: &Editing<'_>, plan: impl Fn(&mut Txn<'_, WordDoc>) -> Result<(C, P), ToolError>)
where
    C: Command<WordDoc, Error = EditError>,
{
    let done = run_planned(editing, plan);
    assert!(matches!(done, Ok(Ok(_))), "the call alone made its edit");
}

/// The try_call the handlers' refusals offer (unused by a plan that does not refuse).
pub(super) fn retry() -> TryCall {
    crate::structure::common::call("test", &[("doc", json!("d1"))])
}

/// A row put in after the table's last.
pub(super) fn row_at_end(table: &str) -> EditTable {
    EditTable {
        table: Address::parse(table),
        change: TableChange::InsertRow {
            index: None,
            below: true,
            plain: true,
        },
        policy: RevisionPolicy::Untracked,
    }
}

/// Rows of a source with the columns `name` and `value`.
pub(super) fn source(rows: &[(&str, &str)]) -> SourceRows {
    SourceRows {
        given: "d:s".to_owned(),
        address: "d:s".to_owned(),
        columns: vec!["name".to_owned(), "value".to_owned()],
        types: None,
        rows: rows
            .iter()
            .map(|(n, v)| vec![Cell::Text((*n).to_owned()), Cell::Text((*v).to_owned())])
            .collect(),
        sources: None,
        doc: None,
        name: None,
        version: None,
        stale: false,
        cache: None,
    }
}

/// `word_format_table` `cant_split` on the table: every row counted where the edit is applied.
mod format_table {
    use super::*;
    use crate::objects::format_table::plan;
    use crate::objects::format_table_args::WordFormatTable;

    fn kept_whole(table: &str) -> WordFormatTable {
        args(json!({"doc": "d1", "table": table, "scope": "table", "cant_split": true}))
    }

    /// The row put in, then every row kept whole, one at a time.
    fn serial() -> Vec<Row> {
        let (alone_editor, table) = with_table();
        alone_editor
            .store()
            .run(&Caller::anonymous(), &row_at_end(&table))
            .unwrap();
        let call = kept_whole(&table);
        editing(&alone_editor, |editing| {
            let retry = retry();
            alone(editing, |tx| plan(&call, editing, tx, &retry));
        });
        rows_of(&alone_editor, &table)
    }

    #[test]
    fn the_rows_counted_in_the_transaction_conflict_with_a_row_put_in() {
        let (shared, table) = with_table();
        let call = kept_whole(&table);
        let (refused, conflicts) = editing(&shared, |editing| {
            let retry = retry();
            raced(
                &shared,
                |tx: &mut Txn<'_, WordDoc>| plan(&call, editing, tx, &retry),
                &row_at_end(&table),
            )
        });
        assert!(refused.is_none(), "{refused:?}");
        assert!(conflicts >= 1, "the row put in is a conflict");
        let rows = rows_of(&shared, &table);
        assert_eq!(rows, serial());
        assert_eq!(rows.len(), 6);
        assert!(rows.iter().all(|r| r.1), "every row, the one put in too, kept whole");
    }

    #[test]
    fn rows_counted_on_the_head_before_the_edit_are_stale_after_a_row_put_in() {
        let (shared, table) = with_table();
        let call = kept_whole(&table);
        editing(&shared, |editing| {
            let retry = retry();
            assert!(planned_before(
                editing,
                |tx| plan(&call, editing, tx, &retry),
                &row_at_end(&table)
            ));
        });
        assert_ne!(rows_of(&shared, &table), serial());
    }
}

/// `word_edit_table` `insert_column` without an index: the last column read where the
/// column goes in.
mod edit_table {
    use super::*;
    use crate::objects::edit_table::change_plan;
    use crate::objects::edit_table_args::WordEditTable;

    fn column_after_last(table: &str) -> WordEditTable {
        args(json!({"doc": "d1", "table": table, "action": "insert_column"}))
    }

    /// The first column (the names) deleted.
    fn names_gone(table: &str) -> EditTable {
        EditTable {
            table: Address::parse(table),
            change: TableChange::DeleteColumn { index: 0 },
            policy: RevisionPolicy::Untracked,
        }
    }

    /// The first column deleted, then a column put in after the last, one at a time.
    fn serial() -> Vec<Row> {
        let (alone_editor, table) = with_table();
        alone_editor
            .store()
            .run(&Caller::anonymous(), &names_gone(&table))
            .unwrap();
        let call = column_after_last(&table);
        editing(&alone_editor, |editing| {
            let retry = retry();
            alone(editing, |tx| change_plan(&call, editing, tx, &retry));
        });
        rows_of(&alone_editor, &table)
    }

    #[test]
    fn the_last_column_read_in_the_transaction_conflicts_with_a_column_deleted() {
        let (shared, table) = with_table();
        let call = column_after_last(&table);
        let (refused, conflicts) = editing(&shared, |editing| {
            let retry = retry();
            raced(
                &shared,
                |tx: &mut Txn<'_, WordDoc>| change_plan(&call, editing, tx, &retry),
                &names_gone(&table),
            )
        });
        assert!(refused.is_none(), "{refused:?}");
        assert!(conflicts >= 1, "the column deleted is a conflict");
        let rows = rows_of(&shared, &table);
        assert_eq!(rows, serial());
        assert_eq!(rows[1].0.len(), 2, "the values, then the column put in");
        assert_eq!(rows[1].0[0].0, "x");
    }

    #[test]
    fn the_last_column_read_on_the_head_before_the_edit_is_stale_after_a_column_deleted() {
        let (shared, table) = with_table();
        let call = column_after_last(&table);
        let ran = editing(&shared, |editing| {
            let retry = retry();
            planned_before(
                editing,
                |tx| change_plan(&call, editing, tx, &retry),
                &names_gone(&table),
            )
        });
        // column 1 is not there any more: refused, or a column put in elsewhere
        assert!(!ran || rows_of(&shared, &table) != serial());
    }
}

/// `word_edit_table` `set_cells` filling a column: its rows read where the cells are written.
mod set_cells {
    use super::*;
    use crate::objects::edit_table_args::WordEditTable;
    use crate::objects::set_cells::plan;

    fn column_filled(table: &str) -> WordEditTable {
        args(json!({"doc": "d1", "table": table, "action": "set_cells",
                    "cells": [{"col": 1, "rows": "*", "text": "F"}]}))
    }

    /// The row put in, then the column filled, one at a time.
    fn serial() -> Vec<Row> {
        let (alone_editor, table) = with_table();
        alone_editor
            .store()
            .run(&Caller::anonymous(), &row_at_end(&table))
            .unwrap();
        let call = column_filled(&table);
        let cells = call.cells.clone().unwrap();
        editing(&alone_editor, |editing| {
            let retry = retry();
            alone(editing, |tx| plan(&call, &cells, editing, tx, &retry));
        });
        rows_of(&alone_editor, &table)
    }

    #[test]
    fn a_fill_s_rows_read_in_the_transaction_conflict_with_a_row_put_in() {
        let (shared, table) = with_table();
        let call = column_filled(&table);
        let cells = call.cells.clone().unwrap();
        let (refused, conflicts) = editing(&shared, |editing| {
            let retry = retry();
            raced(
                &shared,
                |tx: &mut Txn<'_, WordDoc>| plan(&call, &cells, editing, tx, &retry),
                &row_at_end(&table),
            )
        });
        assert!(refused.is_none(), "{refused:?}");
        assert!(conflicts >= 1, "the row put in is a conflict");
        let rows = rows_of(&shared, &table);
        assert_eq!(rows, serial());
        assert_eq!(rows[5].0[1].0, "F", "the row put in is filled too");
    }

    #[test]
    fn a_fill_s_rows_read_on_the_head_before_the_edit_are_stale_after_a_row_put_in() {
        let (shared, table) = with_table();
        let call = column_filled(&table);
        let cells = call.cells.clone().unwrap();
        editing(&shared, |editing| {
            let retry = retry();
            let plan = |tx: &mut Txn<'_, WordDoc>| plan(&call, &cells, editing, tx, &retry);
            assert!(planned_before(editing, plan, &row_at_end(&table)));
        });
        assert_ne!(rows_of(&shared, &table), serial());
    }
}

/// `word_edit_table` `set_cells` from rows with `grow`: the room below read where the block
/// is written.
mod set_from {
    use super::*;
    use crate::objects::edit_table_args::WordEditTable;
    use crate::objects::set_from::placing;

    fn block() -> SourceRows {
        source(&[("n1", "v1"), ("n2", "v2"), ("n3", "v3"), ("n4", "v4")])
    }

    fn placed_from_row_2(table: &str) -> WordEditTable {
        args(
            json!({"doc": "d1", "table": table, "action": "set_cells", "from": "d:s", "index": 2,
                    "grow": true}),
        )
    }

    /// The row put on top, then the block placed, one at a time.
    fn serial() -> Vec<Row> {
        let (alone_editor, table) = with_table();
        alone_editor
            .store()
            .run(&Caller::anonymous(), &row_on_top(&table))
            .unwrap();
        let (call, rows) = (placed_from_row_2(&table), block());
        editing(&alone_editor, |editing| {
            let retry = retry();
            alone(editing, |tx| placing(&call, editing, tx, &rows, &retry));
        });
        rows_of(&alone_editor, &table)
    }

    #[test]
    fn the_room_read_in_the_transaction_conflicts_with_a_row_put_on_top() {
        let (shared, table) = with_table();
        let (call, rows) = (placed_from_row_2(&table), block());
        let (refused, conflicts) = editing(&shared, |editing| {
            let retry = retry();
            raced(
                &shared,
                |tx: &mut Txn<'_, WordDoc>| placing(&call, editing, tx, &rows, &retry),
                &row_on_top(&table),
            )
        });
        assert!(refused.is_none(), "{refused:?}");
        assert!(conflicts >= 1, "the row put on top is a conflict");
        assert_eq!(rows_of(&shared, &table), serial());
    }

    #[test]
    fn the_room_read_on_the_head_before_the_edit_is_stale_after_a_row_put_on_top() {
        let (shared, table) = with_table();
        let (call, rows) = (placed_from_row_2(&table), block());
        editing(&shared, |editing| {
            let retry = retry();
            let plan = |tx: &mut Txn<'_, WordDoc>| placing(&call, editing, tx, &rows, &retry);
            assert!(planned_before(editing, plan, &row_on_top(&table)));
        });
        // the rows grown for the room before the row was put on top: one too many
        assert_ne!(rows_of(&shared, &table), serial());
    }
}

/// `word_edit_table` `sync`: the keys matched and their rows read where the cells are
/// written, the plan and the record with them.
mod sync {
    use super::*;
    use crate::objects::edit_table_args::WordEditTable;
    use crate::objects::table_sync::syncing;

    fn values() -> SourceRows {
        source(&[("r1", "A"), ("r2", "B"), ("r3", "C"), ("r4", "D")])
    }

    fn synced(table: &str) -> WordEditTable {
        args(
            json!({"doc": "d1", "table": table, "action": "sync", "from": "d:s", "key": "name",
                    "from_columns": ["name", "value"]}),
        )
    }

    /// The row put on top, then the sync, one at a time.
    fn serial() -> Vec<Row> {
        let (alone_editor, table) = with_table();
        alone_editor
            .store()
            .run(&Caller::anonymous(), &row_on_top(&table))
            .unwrap();
        let (call, rows) = (synced(&table), values());
        editing(&alone_editor, |editing| {
            let retry = retry();
            alone(editing, |tx| syncing(&call, editing, tx, &rows, &retry));
        });
        rows_of(&alone_editor, &table)
    }

    #[test]
    fn the_keys_rows_read_in_the_transaction_conflict_with_a_row_put_on_top() {
        let (shared, table) = with_table();
        let (call, rows) = (synced(&table), values());
        let (refused, conflicts) = editing(&shared, |editing| {
            let retry = retry();
            raced(
                &shared,
                |tx: &mut Txn<'_, WordDoc>| syncing(&call, editing, tx, &rows, &retry),
                &row_on_top(&table),
            )
        });
        assert!(refused.is_none(), "{refused:?}");
        assert!(conflicts >= 1, "the row put on top is a conflict");
        let synced_rows = rows_of(&shared, &table);
        assert_eq!(synced_rows, serial());
        let values: Vec<&str> = synced_rows.iter().skip(2).map(|r| r.0[1].0.as_str()).collect();
        assert_eq!(values, ["A", "B", "C", "D"], "each key's row given its value");
    }

    #[test]
    fn the_keys_rows_read_on_the_head_before_the_edit_are_stale_after_a_row_put_on_top() {
        let (shared, table) = with_table();
        let (call, rows) = (synced(&table), values());
        editing(&shared, |editing| {
            let retry = retry();
            let plan = |tx: &mut Txn<'_, WordDoc>| syncing(&call, editing, tx, &rows, &retry);
            assert!(planned_before(editing, plan, &row_on_top(&table)));
        });
        assert_ne!(rows_of(&shared, &table), serial());
    }
}

/// `edit_chart` `set_value`: the series named resolved to its index where the edit is applied.
mod edit_chart {
    use super::*;
    use crate::objects::graphics::WordGraphics;
    use ooxml_tools_chart::{category_names, planned, planner, series_names, ChartDataEdit, DataAction, EditChart};
    use ooxml_word_edit::EditGraphic;
    use ooxml_word_model::docx::drawings::graphics;
    use ooxml_word_model::docx::graphics::chart_part;

    /// The fixture's first chart with two series or more: its id, part, second series' name
    /// and first category's.
    fn chart(editor: &Editor) -> (String, String, String, String) {
        let docx = editor.store().snapshot().doc().docx().unwrap();
        graphics(&docx)
            .into_iter()
            .filter(|d| d.kind == "chart")
            .find_map(|d| {
                let part = chart_part(&docx, &d.part, &d.element)?;
                let tree = docx.package.tree(&part).ok()??;
                let data = ooxml_chart::data::chart_data(tree.root()).ok()?;
                let series = series_names(&data);
                let category = category_names(&data).first()?.clone();
                (series.len() >= 2).then(|| (d.id.clone(), part, series[1].clone(), category))
            })
            .unwrap()
    }

    /// The chart's first series removed.
    fn first_series_gone(id: &str) -> EditGraphic<ChartDataEdit> {
        EditGraphic {
            drawing: id.to_owned(),
            edit: ChartDataEdit {
                address: id.to_owned(),
                action: DataAction::RemoveSeries { series: 0 },
            },
            policy: RevisionPolicy::Untracked,
        }
    }

    fn set_value(id: &str, series: &str, category: &str) -> EditChart {
        args(
            json!({"doc": "d1", "target": id, "action": "set_value", "series": series,
                    "category": category, "value": 99}),
        )
    }

    fn plan<'e>(
        call: &EditChart,
        editing: &Editing<'e>,
        tx: &mut Txn<'_, WordDoc>,
    ) -> Result<(EditGraphic<ChartDataEdit>, ()), ToolError> {
        let read = retry();
        let edit = |data: &ooxml_chart::data::ChartData, _: &ooxml_xml::Element, lang: &str| {
            planned(call, data, &read, lang).map(|action| ChartDataEdit {
                address: call.target.clone(),
                action,
            })
        };
        let writer = WordGraphics {
            editing,
            doc: &call.doc,
        };
        let plan = planner(call, &edit);
        writer.planned(tx, &call.target, &plan)
    }

    fn part_of(editor: &Editor, part: &str) -> Vec<u8> {
        editor
            .store()
            .snapshot()
            .doc()
            .package()
            .tree(part)
            .unwrap()
            .unwrap()
            .write()
    }

    /// The first series removed, then the value set, one at a time.
    fn serial() -> Vec<u8> {
        let alone_editor = opened("charts/charts.docx");
        let (id, part, series, category) = chart(&alone_editor);
        alone_editor
            .store()
            .run(&Caller::anonymous(), &first_series_gone(&id))
            .unwrap();
        let call = set_value(&id, &series, &category);
        editing(&alone_editor, |editing| alone(editing, |tx| plan(&call, editing, tx)));
        part_of(&alone_editor, &part)
    }

    #[test]
    fn a_series_resolved_in_the_transaction_conflicts_with_a_series_removed_before_it() {
        let shared = opened("charts/charts.docx");
        let (id, part, series, category) = chart(&shared);
        let call = set_value(&id, &series, &category);
        let (refused, conflicts) = editing(&shared, |editing| {
            raced(
                &shared,
                |tx: &mut Txn<'_, WordDoc>| plan(&call, editing, tx),
                &first_series_gone(&id),
            )
        });
        assert!(refused.is_none(), "{refused:?}");
        assert!(conflicts >= 1, "the series removed is a conflict");
        assert_eq!(part_of(&shared, &part), serial());
    }

    #[test]
    fn a_series_resolved_on_the_head_before_the_edit_is_stale_after_a_series_removed_before_it() {
        let shared = opened("charts/charts.docx");
        let (id, part, series, category) = chart(&shared);
        let call = set_value(&id, &series, &category);
        let ran = editing(&shared, |editing| {
            planned_before(editing, |tx| plan(&call, editing, tx), &first_series_gone(&id))
        });
        // the index names another series now, or none
        assert!(!ran || part_of(&shared, &part) != serial());
    }
}

/// `word_controls` `fill`: the control's kind read where the fill is applied.
mod controls {
    use super::*;
    use crate::objects::controls::fill;
    use ooxml_word_edit::{FillControl, Filling};
    use ooxml_word_model::docx::controls::controls;

    /// The fixture's first text control's id.
    fn text_control(editor: &Editor) -> String {
        let docx = editor.store().snapshot().doc().docx().unwrap();
        controls(&docx)
            .into_iter()
            .find(|c| c.kind() == "text" || c.kind() == "rich-text")
            .unwrap()
            .id
    }

    fn filled(id: &str) -> FillControl {
        FillControl {
            control: id.to_owned(),
            value: Filling::Text("Written meanwhile".to_owned()),
            policy: RevisionPolicy::Untracked,
        }
    }

    fn main_part(editor: &Editor) -> Vec<u8> {
        editor
            .store()
            .snapshot()
            .doc()
            .package()
            .tree("word/document.xml")
            .unwrap()
            .unwrap()
            .write()
    }

    /// The other fill, then this one, one at a time.
    fn serial() -> Vec<u8> {
        let alone_editor = opened("controls.docx");
        let id = text_control(&alone_editor);
        alone_editor.store().run(&Caller::anonymous(), &filled(&id)).unwrap();
        editing(&alone_editor, |editing| {
            let list = retry();
            alone(editing, |tx| fill(editing, tx, (&id, "last"), &list));
        });
        main_part(&alone_editor)
    }

    #[test]
    fn the_kind_read_in_the_transaction_conflicts_with_the_control_written() {
        let shared = opened("controls.docx");
        let id = text_control(&shared);
        let (refused, conflicts) = editing(&shared, |editing| {
            let list = retry();
            raced(
                &shared,
                |tx: &mut Txn<'_, WordDoc>| fill(editing, tx, (&id, "last"), &list),
                &filled(&id),
            )
        });
        assert!(refused.is_none(), "{refused:?}");
        assert!(conflicts >= 1, "the control written meanwhile is a conflict");
        assert_eq!(main_part(&shared), serial());
    }
}
