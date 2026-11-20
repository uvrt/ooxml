//! A made-up workbook read as ooxml-edit's `Workbook` reads it: sheets by name, every kind of
//! cell value, rows and cells without `r`, shared strings plain and rich; the expected values
//! are what `ooxml_edit.sheets.workbook` gives on the same parts.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use ooxml_opc::Limits;
use ooxml_sheets::{Cell, SheetsError, Value, Workbook};
use ooxml_zip::{Method, NewEntry, Writer};
use std::sync::Arc;

const NS: &str = r#"xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships""#;
const ROOT_RELS: &str = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#;
const BOOK_RELS: &str = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet2.xml"/><Relationship Id="rId3" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/sharedStrings" Target="sharedStrings.xml"/><Relationship Id="rId4" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/gone.xml"/><Relationship Id="rId5" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet3.xml"/></Relationships>"#;

fn book() -> String {
    format!(
        r#"<workbook {NS}><workbookPr date1904=" True "/><sheets><sheet name="Sheet1" sheetId="1" r:id="rId1"/><sheet name="My 'data'" sheetId="2" r:id="rId2"/><sheet name="Gone" sheetId="3" r:id="rId4"/><sheet name="NoRel" sheetId="4"/><sheet name="Bare" sheetId="5" r:id="rId5"/></sheets></workbook>"#
    )
}

fn strings() -> String {
    format!(
        r#"<sst {NS} count="3" uniqueCount="3"><si><t>Plain</t></si><si><r><t>Ri</t></r><r><rPr><b/></rPr><t>ch</t></r><rPh><t>x</t></rPh></si><si><t xml:space="preserve"> lead</t><phoneticPr fontId="1"/></si></sst>"#
    )
}

fn sheet1() -> String {
    format!(
        r#"<worksheet {NS}><sheetData>
<row r="1"><c r="A1" t="s"><v>0</v></c><c r="B1" t="s"><v>1</v></c><c t="s"><v> 2 </v></c><c r="E1" t="inlineStr"><is><t>inline</t></is></c></row>
<row r="2"><c r="A2"><v>3</v></c><c r="B2"><v>3.0</v></c><c r="C2"><v>1e2</v></c><c r="D2"><v>2.5</v></c><c r="E2"><v>1_000</v></c><c r="F2"><v>inf</v></c><c r="G2"><v>n/a</v></c><c r="H2" t="d"><v>2024-01-02</v></c></row>
<row><c r="A3" t="b"><v> true</v></c><c r="B3" t="b"><v>0</v></c><c r="C3" t="e"><v>#N/A</v></c><c r="D3" t="str"><f>A1</f><v>Plain</v></c><c r="E3" t="str"><f>""</f><v></v></c><c r="F3" s="2"/></row>
<row r="3"><c r="A3"><v>99</v></c></row>
<row r="5"><c r="A5"><v>1</v></c><c r="bad"><v>2</v></c></row>
</sheetData></worksheet>"#
    )
}

fn sheet2() -> String {
    format!(r#"<worksheet {NS}><sheetData><row r="x"><c><v>1</v></c></row></sheetData></worksheet>"#)
}

fn sheet3() -> String {
    format!(r#"<worksheet {NS}><sheetPr/></worksheet>"#)
}

fn package() -> Vec<u8> {
    let mut w = Writer::new();
    let parts = [
        ("_rels/.rels", ROOT_RELS.to_owned()),
        ("xl/workbook.xml", book()),
        ("xl/_rels/workbook.xml.rels", BOOK_RELS.to_owned()),
        ("xl/sharedStrings.xml", strings()),
        ("xl/worksheets/sheet1.xml", sheet1()),
        ("xl/worksheets/sheet2.xml", sheet2()),
        ("xl/worksheets/sheet3.xml", sheet3()),
    ];
    for (name, data) in parts {
        let entry = NewEntry {
            name: name.into(),
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

fn open() -> Workbook {
    Workbook::open(Arc::from(package()), &Limits::default()).unwrap()
}

fn at(reference: &str) -> Cell {
    Cell::parse(reference).unwrap()
}

fn text(s: &str) -> Option<Value> {
    Some(Value::Text(s.to_owned()))
}

#[test]
fn finds_sheets_by_name() {
    let book = open();
    assert_eq!(book.part(), "xl/workbook.xml");
    assert_eq!(book.sheet_names(), ["Sheet1", "My 'data'", "Gone", "NoRel", "Bare"]);
    assert!(book.date1904());
    assert_eq!(
        book.sheet("Sheet1").unwrap().unwrap().part(),
        "xl/worksheets/sheet1.xml"
    );
    assert_eq!(book.sheet("My 'data'").unwrap().unwrap().name(), "My 'data'");
    for missing in ["sheet1", "Gone", "NoRel"] {
        assert!(book.sheet(missing).unwrap().is_none(), "{missing}");
    }
    assert_eq!(book.sheet("Bare").unwrap_err(), SheetsError::NoSheetData("Bare".into()));
}

#[test]
fn reads_every_kind_of_value() {
    let book = open();
    let sheet = book.sheet("Sheet1").unwrap().unwrap();
    let value = |r: &str| sheet.value(at(r)).unwrap();
    assert_eq!(value("A1"), text("Plain"));
    assert_eq!(value("B1"), text("Rich"));
    assert_eq!(value("C1"), text(" lead"));
    assert_eq!(value("D1"), None);
    assert_eq!(value("E1"), text("inline"));
    assert_eq!(value("A2"), Some(Value::Integer(3.0)));
    assert_eq!(value("B2"), Some(Value::Number(3.0)));
    assert_eq!(value("C2"), Some(Value::Number(100.0)));
    assert_eq!(value("D2"), Some(Value::Number(2.5)));
    assert_eq!(value("E2"), Some(Value::Integer(1000.0)));
    assert_eq!(value("F2"), Some(Value::Number(f64::INFINITY)));
    assert_eq!(value("G2"), text("n/a"));
    assert_eq!(value("H2"), text("2024-01-02"));
    // The row without `r` is row 3; the later row 3 is not read.
    assert_eq!(value("A3"), Some(Value::Bool(true)));
    assert_eq!(value("B3"), Some(Value::Bool(false)));
    assert_eq!(value("C3"), text("#N/A"));
    assert_eq!(value("D3"), text("Plain"));
    assert_eq!(value("E3"), None);
    assert_eq!(value("F3"), None);
    assert_eq!(value("Z9"), None);
    // A malformed reference after the cell asked for is never read; one before it is.
    assert_eq!(value("A5"), Some(Value::Integer(1.0)));
    assert!(matches!(sheet.value(at("C5")), Err(SheetsError::Address(_))));
}

#[test]
fn refuses_what_cannot_be_read() {
    let book = open();
    let rows = book.sheet("My 'data'").unwrap().unwrap();
    assert!(matches!(rows.value(at("A1")), Err(SheetsError::RowNumber { .. })));
    assert_eq!(book.shared_string(3), Err(SheetsError::NoSharedString(3)));
    assert_eq!(book.shared_string(-1), Err(SheetsError::NoSharedString(-1)));
    let not_a_package = Workbook::open(Arc::from(b"not a zip".as_slice()), &Limits::default());
    assert!(matches!(not_a_package, Err(SheetsError::Package(_))));
}
