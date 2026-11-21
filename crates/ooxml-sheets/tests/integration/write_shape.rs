//! The made-up chart workbook (`write::package_with`) changed in shape as ooxml-edit's
//! `Worksheet` changes it when a chart's data grows or shrinks -- `move`, `clear`,
//! `set_value` with `style_from`, `shift_tables` and `sync_tables`, then `Workbook.finish`
//! (worked out with `ooxml_edit.sheets.workbook` on the same parts, 2026-11-21) -- and a
//! chart formula given a new area, spelled as ooxml-edit's `SheetRange.with_area` spells it.

use super::write::{at, package_with, part};
use ooxml_opc::Limits;
use ooxml_sheets::{parse_formula, Area, Axis, Cell, SheetsError, Shift, WorkbookWriter, Write};
use std::sync::Arc;

fn book() -> WorkbookWriter {
    WorkbookWriter::open(Arc::from(package_with(true)), &Limits::default()).unwrap()
}

fn shift(axis: Axis, position: u32, delta: i64, span: (u32, u32)) -> Shift {
    Shift {
        axis,
        position,
        delta,
        span,
    }
}

fn parts(book: WorkbookWriter) -> (String, String, String) {
    let out = book.finish().unwrap().unwrap();
    (
        part(&out, "xl/worksheets/sheet1.xml"),
        part(&out, "xl/tables/table1.xml"),
        part(&out, "xl/sharedStrings.xml"),
    )
}

#[test]
fn a_row_put_in_moves_the_cells_below_and_grows_the_table() {
    let mut book = book();
    book.move_cell("Sheet1", at("A3"), at("A4")).unwrap();
    book.move_cell("Sheet1", at("B3"), at("B4")).unwrap();
    book.write_cell("Sheet1", at("A3"), &Write::Text("Q9".into()), Some(at("A4")))
        .unwrap();
    book.write_cell("Sheet1", at("B3"), &Write::Number(7.0), Some(at("B4")))
        .unwrap();
    book.shift_tables("Sheet1", shift(Axis::Row, 3, 1, (1, 2))).unwrap();
    book.clear("Sheet1", at("B1")).unwrap();
    book.sync_tables("Sheet1").unwrap();
    let (sheet, table, strings) = parts(book);
    assert!(sheet.contains(r#"<dimension ref="A2:B4"/><sheetData><row r="2" spans="1:2"><c r="A2" t="s"><v>0</v></c><c r="B2" s="3"><v>10</v></c></row><row r="3"><c r="A3" t="s"><v>3</v></c><c r="B3"><v>7</v></c></row><row r="4"><c r="A4" t="s"><v>2</v></c><c r="B4"><v>20</v></c></row></sheetData>"#), "{sheet}");
    assert!(table.contains(r#"ref="A1:B4"><autoFilter ref="A1:B4"/><tableColumns count="2"><tableColumn id="1" name=" "/><tableColumn id="2" name="North"/></tableColumns>"#), "{table}");
    assert!(strings.contains(r#"count="3" uniqueCount="4">"#), "{strings}");
}

#[test]
fn a_column_put_in_past_the_table_grows_it_and_names_its_column() {
    let mut book = book();
    book.write_cell("Sheet1", at("C1"), &Write::Text("East".into()), Some(at("B1")))
        .unwrap();
    book.write_cell("Sheet1", at("C2"), &Write::Number(1.5), Some(at("B2")))
        .unwrap();
    book.shift_tables("Sheet1", shift(Axis::Column, 3, 1, (1, 3))).unwrap();
    book.sync_tables("Sheet1").unwrap();
    let (sheet, table, strings) = parts(book);
    assert!(sheet.contains(r#"<dimension ref="A1:C3"/><sheetData><row r="1" spans="2:3"><c r="B1" t="s"><v>1</v></c><c r="C1" t="s"><v>3</v></c></row><row r="2" spans="1:3"><c r="A2" t="s"><v>0</v></c><c r="B2" s="3"><v>10</v></c><c r="C2" s="3"><v>1.5</v></c></row>"#), "{sheet}");
    assert!(table.contains(r#"ref="A1:C3"><autoFilter ref="A1:C3"/><tableColumns count="3"><tableColumn id="1" name=" "/><tableColumn id="2" name="North"/><tableColumn id="3" name="East"/></tableColumns>"#), "{table}");
    assert!(strings.contains(r#"count="4" uniqueCount="4">"#), "{strings}");
}

#[test]
fn a_column_taken_out_shrinks_the_table() {
    let mut book = book();
    for cell in ["B1", "B2", "B3"] {
        book.clear("Sheet1", at(cell)).unwrap();
    }
    book.shift_tables("Sheet1", shift(Axis::Column, 2, -1, (1, 3))).unwrap();
    book.sync_tables("Sheet1").unwrap();
    let (sheet, table, strings) = parts(book);
    assert!(sheet.contains(r#"<dimension ref="A2:A3"/><sheetData><row r="2" spans="1:1"><c r="A2" t="s"><v>0</v></c></row><row r="3" spans="1:1"><c r="A3" t="s"><v>2</v></c></row></sheetData>"#), "{sheet}");
    assert!(table.contains(r#"ref="A1:A3"><autoFilter ref="A1:A3"/><tableColumns count="1"><tableColumn id="1" name=" "/></tableColumns>"#), "{table}");
    assert!(strings.contains(r#"count="2" uniqueCount="3">"#), "{strings}");
}

#[test]
fn a_row_taken_out_moves_the_cells_below_back() {
    let mut book = book();
    book.clear("Sheet1", at("A2")).unwrap();
    book.clear("Sheet1", at("B2")).unwrap();
    book.move_cell("Sheet1", at("A3"), at("A2")).unwrap();
    book.move_cell("Sheet1", at("B3"), at("B2")).unwrap();
    book.move_cell("Sheet1", at("D9"), at("D8")).unwrap();
    book.shift_tables("Sheet1", shift(Axis::Row, 2, -1, (1, 2))).unwrap();
    book.sync_tables("Sheet1").unwrap();
    assert!(book.is_empty("Sheet1", at("A3")).unwrap());
    assert!(!book.is_empty("Sheet1", at("A2")).unwrap());
    let (sheet, table, _) = parts(book);
    assert!(sheet.contains(r#"<dimension ref="A1:B2"/><sheetData><row r="1" spans="1:2"><c r="B1" t="s"><v>1</v></c></row><row r="2"><c r="A2" t="s"><v>2</v></c><c r="B2"><v>20</v></c></row></sheetData>"#), "{sheet}");
    assert!(table.contains(r#"ref="A1:B2"><autoFilter ref="A1:B2"/>"#), "{table}");
}

#[test]
fn a_table_beside_the_change_stays_and_a_missing_sheet_is_refused() {
    let mut book = book();
    book.shift_tables("Sheet1", shift(Axis::Row, 2, 1, (5, 9))).unwrap();
    assert_eq!(book.finish().unwrap(), None);
    let mut book = self::book();
    assert!(matches!(
        book.shift_tables("Other", shift(Axis::Row, 2, 1, (1, 2))),
        Err(SheetsError::NoSheet(_))
    ));
}

#[test]
fn a_formula_given_a_new_area_keeps_its_spelling() {
    let range = parse_formula("'My data'!$B$2:$B$4").unwrap().unwrap();
    let cell = |row, column| Cell { row, column };
    let one = ooxml_sheets::SheetRange {
        area: Area {
            first: cell(2, 2),
            last: cell(2, 2),
        },
        ..range.clone()
    };
    assert_eq!(one.formula().unwrap(), "'My data'!$B$2");
    let moved = ooxml_sheets::SheetRange {
        area: Area {
            first: cell(2, 3),
            last: cell(5, 3),
        },
        ..range
    };
    assert_eq!(moved.formula().unwrap(), "'My data'!$C$2:$C$5");
    let mixed = parse_formula("Sheet1!B2:$B4").unwrap().unwrap();
    let grown = ooxml_sheets::SheetRange {
        area: Area {
            first: cell(2, 2),
            last: cell(6, 2),
        },
        ..mixed
    };
    assert_eq!(grown.formula().unwrap(), "Sheet1!B2:$B6");
}
