//! A1 references and chart formulas, read as ooxml-edit's `sheets/workbook.py` reads them
//! (each expected value is what its `Cell.parse`, `Area.parse` and `parse_formula` give).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use ooxml_sheets::{column_index, column_letters, parse_formula, AddressError, Area, Cell, MAX_COLUMN};

fn cell(row: u32, column: u32) -> Cell {
    Cell { row, column }
}

#[test]
fn columns() {
    assert_eq!(column_letters(1).unwrap(), "A");
    assert_eq!(column_letters(27).unwrap(), "AA");
    assert_eq!(column_letters(MAX_COLUMN).unwrap(), "XFD");
    assert_eq!(column_letters(0), Err(AddressError::ColumnOutside(0)));
    assert_eq!(column_index("xfd").unwrap(), MAX_COLUMN);
    assert_eq!(column_index("XFE"), Err(AddressError::PastLastColumn("XFE".into())));
    for bad in ["", "A1", "$A", "ABCD", "É"] {
        assert_eq!(
            column_index(bad),
            Err(AddressError::NotColumnLetters(bad.into())),
            "{bad}"
        );
    }
}

#[test]
fn cells_and_areas() {
    assert_eq!(Cell::parse(" $b$2 ").unwrap(), cell(2, 2));
    assert_eq!(Cell::parse("A0").unwrap(), cell(0, 1));
    assert_eq!(cell(2, 28).reference().unwrap(), "AB2");
    assert_eq!(Cell::parse("XFE1"), Err(AddressError::PastLastColumn("XFE1".into())));
    for bad in ["A", "1", "A12345678", "ABCD1", "A1:B2", "A$$1"] {
        assert_eq!(Cell::parse(bad), Err(AddressError::NotACell(bad.into())), "{bad}");
    }
    let area = Area::parse("D4:B2").unwrap();
    assert_eq!((area.first, area.last), (cell(2, 2), cell(4, 4)));
    assert_eq!((area.rows(), area.columns()), (3, 3));
    assert_eq!(area.reference().unwrap(), "B2:D4");
    assert!(area.contains(cell(3, 3)) && !area.contains(cell(5, 3)));
    assert_eq!(area.cells().count(), 9);
    assert_eq!(Area::parse("C3").unwrap().reference().unwrap(), "C3");
    assert_eq!(Area::parse("A1:B2'"), Err(AddressError::NotARange("A1:B2'".into())));
}

#[test]
fn formulas() {
    let range = parse_formula(" 'My ''data'''!$B$2:B$5 ").unwrap().unwrap();
    assert_eq!(range.sheet, "My 'data'");
    assert_eq!(range.sheet_text, "'My ''data'''");
    assert_eq!(range.area, Area::parse("B2:B5").unwrap());
    assert_eq!(range.absolute, [[true, true], [false, true]]);
    assert_eq!(range.formula().unwrap(), "'My ''data'''!$B$2:B$5");
    assert!(range.is_line());
    assert_eq!(range.length(), 4);
    assert_eq!(range.cell(3), Some(cell(5, 2)));
    assert_eq!(range.cell(4), None);
    let row = parse_formula("Sheet1!C1:E1").unwrap().unwrap();
    assert_eq!(row.cell(2), Some(cell(1, 5)));
    let one = parse_formula("Sheet1!$A$1").unwrap().unwrap();
    assert_eq!(one.formula().unwrap(), "Sheet1!$A$1");
    assert_eq!(one.absolute, [[true, true], [true, true]]);
    let rectangle = parse_formula("Sheet1!A1:B2").unwrap().unwrap();
    assert_eq!(rectangle.cell(0), None);
    for none in [
        "",
        "Name",
        "Sheet1!A1:B2,Sheet1!C1",
        "[1]Sheet1!A1",
        "Sheet1!B2:A1",
        "''!A1",
        "'open!A1",
        "Sheet 1!A1",
        "Sheet1!A1:",
        "Sheet1!A12345678",
    ] {
        assert_eq!(parse_formula(none), Ok(None), "{none}");
    }
    assert_eq!(
        parse_formula("Sheet1!XFE1"),
        Err(AddressError::PastLastColumn("XFE1".into()))
    );
}
