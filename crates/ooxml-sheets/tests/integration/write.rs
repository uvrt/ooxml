//! A made-up chart workbook written as ooxml-edit's `Worksheet.set_value`, `sync_tables` and
//! `Workbook.finish` write it (worked out with `ooxml_edit.sheets.workbook` on the same
//! parts, 2026-11-21): a number over a number, a text as a new shared string and as one
//! there already, a blank that empties a cell and its row, a new row and cell in order, the
//! table's column named after its header, the counts and the dimension.

use ooxml_opc::{Limits, Package};
use ooxml_sheets::{Cell, SheetsError, WorkbookWriter, Write};
use ooxml_zip::{Method, NewEntry, Writer};
use std::sync::Arc;

const NS: &str = r#"xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships""#;
const RELS: &str = r#"xmlns="http://schemas.openxmlformats.org/package/2006/relationships""#;
const T: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

fn package() -> Vec<u8> {
    let parts = [
        ("_rels/.rels".to_owned(), format!(r#"<Relationships {RELS}><Relationship Id="rId1" Type="{T}/officeDocument" Target="xl/workbook.xml"/></Relationships>"#)),
        ("xl/workbook.xml".to_owned(), format!(r#"<workbook {NS}><sheets><sheet name="Sheet1" sheetId="1" r:id="rId1"/></sheets></workbook>"#)),
        ("xl/_rels/workbook.xml.rels".to_owned(), format!(r#"<Relationships {RELS}><Relationship Id="rId1" Type="{T}/worksheet" Target="worksheets/sheet1.xml"/><Relationship Id="rId2" Type="{T}/sharedStrings" Target="sharedStrings.xml"/></Relationships>"#)),
        ("xl/sharedStrings.xml".to_owned(), format!(r#"<sst {NS} count="3" uniqueCount="3"><si><t>Q1</t></si><si><t>North</t></si><si><t>Q2</t></si></sst>"#)),
        ("xl/worksheets/sheet1.xml".to_owned(), format!(r#"<worksheet {NS}><dimension ref="A1:B3"/><sheetData><row r="1" spans="1:2"><c r="B1" t="s"><v>1</v></c></row><row r="2" spans="1:2"><c r="A2" t="s"><v>0</v></c><c r="B2" s="3"><v>10</v></c></row><row r="3" spans="1:2"><c r="A3" t="s"><v>2</v></c><c r="B3"><v>20</v></c></row></sheetData><tableParts count="1"><tablePart r:id="rId1"/></tableParts></worksheet>"#)),
        ("xl/worksheets/_rels/sheet1.xml.rels".to_owned(), format!(r#"<Relationships {RELS}><Relationship Id="rId1" Type="{T}/table" Target="../tables/table1.xml"/></Relationships>"#)),
        ("xl/tables/table1.xml".to_owned(), format!(r#"<table {NS} id="1" name="Table1" displayName="Table1" ref="A1:B3"><tableColumns count="2"><tableColumn id="1" name=" "/><tableColumn id="2" name="North"/></tableColumns></table>"#)),
    ];
    let mut w = Writer::new();
    for (name, data) in parts {
        let entry = NewEntry {
            name,
            method: Method::Deflated,
            dos_time: 0,
            dos_date: 0x21,
            external_attr: 0,
            create_system: 0,
        };
        w.add(&entry, data.as_bytes()).unwrap();
    }
    w.finish(b"").unwrap()
}

fn at(reference: &str) -> Cell {
    Cell::parse(reference).unwrap()
}

fn part(bytes: &[u8], name: &str) -> String {
    let package = Package::open(Arc::from(bytes.to_vec()), &Limits::default()).unwrap();
    String::from_utf8(package.read(name).unwrap().unwrap().to_vec()).unwrap()
}

#[test]
fn cells_are_written_and_kept_in_step() {
    let mut book = WorkbookWriter::open(Arc::from(package()), &Limits::default()).unwrap();
    book.set_value("Sheet1", at("B2"), &Write::Number(10.0), None).unwrap();
    book.set_value("Sheet1", at("B2"), &Write::Number(11.5), None).unwrap();
    book.set_value("Sheet1", at("B1"), &Write::Text("South".into()), None).unwrap();
    book.set_value("Sheet1", at("A3"), &Write::Text("Q1".into()), None).unwrap();
    book.set_value("Sheet1", at("B3"), &Write::Blank, None).unwrap();
    book.set_value("Sheet1", at("C5"), &Write::Number(1e16), Some(at("B2"))).unwrap();
    assert!(matches!(
        book.set_value("Other", at("A1"), &Write::Blank, None),
        Err(SheetsError::NoSheet(_))
    ));
    let out = book.finish().unwrap().unwrap();
    let sheet = part(&out, "xl/worksheets/sheet1.xml");
    assert!(sheet.contains(r#"<dimension ref="A1:C5"/>"#), "{sheet}");
    assert!(sheet.contains(r#"<row r="1" spans="1:2"><c r="B1" t="s"><v>3</v></c></row>"#), "{sheet}");
    assert!(sheet.contains(r#"<c r="B2" s="3"><v>11.5</v></c>"#), "{sheet}");
    assert!(sheet.contains(r#"<row r="3" spans="1:1"><c r="A3" t="s"><v>0</v></c></row><row r="5"><c r="C5" s="3"><v>1e+16</v></c></row>"#), "{sheet}");
    let strings = part(&out, "xl/sharedStrings.xml");
    assert!(strings.contains(r#"count="3" uniqueCount="4"><si><t>Q1</t></si><si><t>North</t></si><si><t>Q2</t></si><si><t>South</t></si></sst>"#), "{strings}");
    let table = part(&out, "xl/tables/table1.xml");
    assert!(table.contains(r#"<tableColumn id="1" name=" "/><tableColumn id="2" name="South"/>"#), "{table}");
}

#[test]
fn nothing_written_is_no_change() {
    let mut book = WorkbookWriter::open(Arc::from(package()), &Limits::default()).unwrap();
    book.set_value("Sheet1", at("B3"), &Write::Number(20.0), None).unwrap();
    book.set_value("Sheet1", at("D9"), &Write::Blank, None).unwrap();
    assert_eq!(book.finish().unwrap(), None);
}
