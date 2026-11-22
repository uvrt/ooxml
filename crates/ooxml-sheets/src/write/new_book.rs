//! Owns: [`NewBook`], a new workbook of one sheet written from nothing, as ooxml-edit's
//! `charts/create.py` writes the workbook behind a new chart (`chart_workbook`): the sheet's
//! cells row by row (a text a shared string, a number with the style of a number format),
//! its shared strings counted as referenced, a style sheet holding Calibri 11 and one cell
//! format per number format given (ids from 164), and the package's parts in that order,
//! each with the XML declaration Office writes, deflated as Python's `zipfile` writes them
//! (dated 1 January 1980, made on Unix).
//!
//! Must not: write anything of Office's beyond those parts (no theme, no document
//! properties), or know what the cells mean.

use crate::{column_letters, SheetsError};
use ooxml_opc::OpcError;
use ooxml_xml::view::py_space;
use ooxml_zip::{Method, NewEntry, Writer};

/// SpreadsheetML's main namespace.
const SML: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
/// The package's relationships namespace.
const PKG_RELS: &str = "http://schemas.openxmlformats.org/package/2006/relationships";
/// The office document's relationships namespace.
const DOC_REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
/// SpreadsheetML's content types' stem.
const MAIN: &str = "application/vnd.openxmlformats-officedocument.spreadsheetml";
/// The first id of a custom number format.
const FIRST_FORMAT: usize = 164;
/// MS-DOS date of 1 January 1980 (zipfile's `date_time=(1980, 1, 1, 0, 0, 0)`).
const DOS_1980: u16 = (1 << 5) | 1;
/// Unix as the creating system (`create_system = 3`).
const UNIX: u8 = 3;

/// What a new cell holds.
#[derive(Debug, Clone, PartialEq)]
pub enum NewValue {
    /// A text: a shared string.
    Text(String),
    /// A number, written as Excel writes it (`number_text`).
    Number(f64),
}

/// A new cell: its column (1-based), its value and its style (`s`: 0 none, else the
/// `n`-th number format given, from 1).
#[derive(Debug, Clone, PartialEq)]
pub struct NewCell {
    /// The column, from 1.
    pub column: u32,
    /// The value.
    pub value: NewValue,
    /// The cell format, from 1 for the number formats given; 0 is the default.
    pub style: usize,
}

/// A new workbook of one sheet.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct NewBook {
    /// The sheet's name.
    pub sheet: String,
    /// The rows that have cells, in order: the row number (from 1) and its cells.
    pub rows: Vec<(u32, Vec<NewCell>)>,
    /// The sheet's `dimension` (`A1:C4`).
    pub dimension: String,
    /// The number formats the cells' styles name, in order (format ids from 164).
    pub formats: Vec<String>,
}

/// The shared strings: each text once, in the order first referenced, and the references.
#[derive(Default)]
struct Shared {
    texts: Vec<String>,
    references: usize,
}

impl Shared {
    fn index(&mut self, text: &str) -> usize {
        self.references += 1;
        match self.texts.iter().position(|t| t == text) {
            Some(found) => found,
            None => {
                self.texts.push(text.to_owned());
                self.texts.len() - 1
            }
        }
    }
}

/// `text` with `&`, `<` and `>` escaped (`xml.sax.saxutils.escape`).
fn escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// `text` as a quoted attribute value (`xml.sax.saxutils.quoteattr`).
fn quoteattr(text: &str) -> String {
    let escaped = escape(text)
        .replace('\n', "&#10;")
        .replace('\r', "&#13;")
        .replace('\t', "&#9;");
    if !escaped.contains('"') {
        format!("\"{escaped}\"")
    } else if !escaped.contains('\'') {
        format!("'{escaped}'")
    } else {
        format!("\"{}\"", escaped.replace('"', "&quot;"))
    }
}

impl NewBook {
    /// The sheet's XML, its texts put in `shared` as they come.
    fn sheet_xml(&self, shared: &mut Shared) -> Result<String, SheetsError> {
        let mut rows = String::new();
        for (row, cells) in &self.rows {
            rows.push_str(&format!("<row r=\"{row}\">"));
            for cell in cells {
                let at = format!("{}{row}", column_letters(cell.column)?);
                let style = if cell.style == 0 {
                    String::new()
                } else {
                    format!(" s=\"{}\"", cell.style)
                };
                match &cell.value {
                    NewValue::Text(text) => {
                        let index = shared.index(text);
                        rows.push_str(&format!("<c r=\"{at}\"{style} t=\"s\"><v>{index}</v></c>"));
                    }
                    NewValue::Number(number) => {
                        let value = super::sheet::number_text(*number);
                        rows.push_str(&format!("<c r=\"{at}\"{style}><v>{value}</v></c>"));
                    }
                }
            }
            rows.push_str("</row>");
        }
        Ok(format!(
            "<worksheet xmlns=\"{SML}\" xmlns:r=\"{DOC_REL}\"><dimension ref=\"{}\"/>\
             <sheetViews><sheetView tabSelected=\"1\" workbookViewId=\"0\"/></sheetViews>\
             <sheetFormatPr defaultRowHeight=\"15\"/><sheetData>{rows}</sheetData>\
             <pageMargins left=\"0.7\" right=\"0.7\" top=\"0.75\" bottom=\"0.75\" header=\"0.3\" \
             footer=\"0.3\"/></worksheet>",
            self.dimension
        ))
    }

    /// The style sheet: the default cell format, then one per number format.
    fn styles_xml(&self) -> String {
        let formats = if self.formats.is_empty() {
            String::new()
        } else {
            let listed: String = self
                .formats
                .iter()
                .enumerate()
                .map(|(i, code)| {
                    format!(
                        "<numFmt numFmtId=\"{}\" formatCode={}/>",
                        FIRST_FORMAT + i,
                        quoteattr(code)
                    )
                })
                .collect();
            format!("<numFmts count=\"{}\">{listed}</numFmts>", self.formats.len())
        };
        let styled: String = (0..self.formats.len())
            .map(|i| {
                format!(
                    "<xf numFmtId=\"{}\" fontId=\"0\" fillId=\"0\" borderId=\"0\" xfId=\"0\" applyNumberFormat=\"1\"/>",
                    FIRST_FORMAT + i
                )
            })
            .collect();
        format!(
            "<styleSheet xmlns=\"{SML}\">{formats}\
             <fonts count=\"1\"><font><sz val=\"11\"/><color theme=\"1\"/><name val=\"Calibri\"/>\
             <family val=\"2\"/><scheme val=\"minor\"/></font></fonts>\
             <fills count=\"2\"><fill><patternFill patternType=\"none\"/></fill>\
             <fill><patternFill patternType=\"gray125\"/></fill></fills>\
             <borders count=\"1\"><border><left/><right/><top/><bottom/><diagonal/></border></borders>\
             <cellStyleXfs count=\"1\"><xf numFmtId=\"0\" fontId=\"0\" fillId=\"0\" borderId=\"0\"/></cellStyleXfs>\
             <cellXfs count=\"{}\"><xf numFmtId=\"0\" fontId=\"0\" fillId=\"0\" borderId=\"0\" xfId=\"0\"/>{styled}</cellXfs>\
             <cellStyles count=\"1\"><cellStyle name=\"Normal\" xfId=\"0\" builtinId=\"0\"/></cellStyles>\
             </styleSheet>",
            1 + self.formats.len()
        )
    }

    /// The workbook's parts, in the order they are written.
    fn parts(&self) -> Result<Vec<(&'static str, String)>, SheetsError> {
        let mut shared = Shared::default();
        let sheet = self.sheet_xml(&mut shared)?;
        let strings: String = shared
            .texts
            .iter()
            .map(|s| {
                let kept = s.trim_matches(py_space).len() != s.len();
                let preserve = if kept { " xml:space=\"preserve\"" } else { "" };
                format!("<si><t{preserve}>{}</t></si>", escape(s))
            })
            .collect();
        let shared_strings = format!(
            "<sst xmlns=\"{SML}\" count=\"{}\" uniqueCount=\"{}\">{strings}</sst>",
            shared.references,
            shared.texts.len()
        );
        let workbook = format!(
            "<workbook xmlns=\"{SML}\" xmlns:r=\"{DOC_REL}\"><bookViews><workbookView/></bookViews>\
             <sheets><sheet name=\"{}\" sheetId=\"1\" r:id=\"rId1\"/></sheets></workbook>",
            self.sheet
        );
        let workbook_rels = format!(
            "<Relationships xmlns=\"{PKG_RELS}\">\
             <Relationship Id=\"rId1\" Type=\"{DOC_REL}/worksheet\" Target=\"worksheets/sheet1.xml\"/>\
             <Relationship Id=\"rId2\" Type=\"{DOC_REL}/styles\" Target=\"styles.xml\"/>\
             <Relationship Id=\"rId3\" Type=\"{DOC_REL}/sharedStrings\" Target=\"sharedStrings.xml\"/>\
             </Relationships>"
        );
        let root_rels = format!(
            "<Relationships xmlns=\"{PKG_RELS}\"><Relationship Id=\"rId1\" \
             Type=\"{DOC_REL}/officeDocument\" Target=\"xl/workbook.xml\"/></Relationships>"
        );
        let content_types = format!(
            "<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\
             <Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>\
             <Default Extension=\"xml\" ContentType=\"application/xml\"/>\
             <Override PartName=\"/xl/workbook.xml\" ContentType=\"{MAIN}.sheet.main+xml\"/>\
             <Override PartName=\"/xl/worksheets/sheet1.xml\" ContentType=\"{MAIN}.worksheet+xml\"/>\
             <Override PartName=\"/xl/styles.xml\" ContentType=\"{MAIN}.styles+xml\"/>\
             <Override PartName=\"/xl/sharedStrings.xml\" ContentType=\"{MAIN}.sharedStrings+xml\"/>\
             </Types>"
        );
        Ok(vec![
            ("[Content_Types].xml", content_types),
            ("_rels/.rels", root_rels),
            ("xl/workbook.xml", workbook),
            ("xl/_rels/workbook.xml.rels", workbook_rels),
            ("xl/worksheets/sheet1.xml", sheet),
            ("xl/styles.xml", self.styles_xml()),
            ("xl/sharedStrings.xml", shared_strings),
        ])
    }

    /// The workbook's package bytes.
    ///
    /// # Errors
    /// A column past the last one, or an archive the writer cannot write.
    pub fn write(&self) -> Result<Vec<u8>, SheetsError> {
        let declaration = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\r\n";
        let zip = |e| SheetsError::Package(OpcError::Zip(e));
        let mut writer = Writer::new();
        for (name, text) in self.parts()? {
            let entry = NewEntry {
                name: name.to_owned(),
                method: Method::Deflated,
                dos_time: 0,
                dos_date: DOS_1980,
                external_attr: 0,
                create_system: UNIX,
            };
            writer
                .add(&entry, format!("{declaration}{text}").as_bytes())
                .map_err(zip)?;
        }
        writer.finish(b"").map_err(zip)
    }
}

#[cfg(test)]
mod tests {
    use super::{quoteattr, NewBook, NewCell, NewValue};
    use ooxml_zip::{Archive, Limits};
    use std::sync::Arc;

    #[test]
    fn attribute_quotes_as_quoteattr() {
        assert_eq!(quoteattr("0.0"), "\"0.0\"");
        assert_eq!(quoteattr("0\"x\""), "'0\"x\"'");
        assert_eq!(quoteattr("'a' \"b\""), "\"'a' &quot;b&quot;\"");
        assert_eq!(quoteattr("a&b<\n"), "\"a&amp;b&lt;&#10;\"");
    }

    #[test]
    fn texts_shared_once_and_counted_each_time() {
        let text = |t: &str, column| NewCell {
            column,
            value: NewValue::Text(t.to_owned()),
            style: 0,
        };
        let book = NewBook {
            sheet: "Sheet1".to_owned(),
            rows: vec![(1, vec![text("a", 2), text(" a", 3)]), (2, vec![text("a", 1)])],
            dimension: "A1:C2".to_owned(),
            formats: vec![],
        };
        let parts = book.parts().unwrap_or_default();
        let strings = parts.last().map(|(_, xml)| xml.as_str()).unwrap_or_default();
        assert!(strings.contains("count=\"3\" uniqueCount=\"2\""));
        assert!(strings.contains("<si><t xml:space=\"preserve\"> a</t></si>"));
    }

    #[test]
    fn entries_as_chart_workbook_writes_them() {
        // ooxml-edit's chart_workbook("column", ["North", " South", "East & West"], two series,
        // number_format='0.0"x"'): golden/chart_column.xlsx
        let cell = |column, value, style| NewCell { column, value, style };
        let text = |t: &str| NewValue::Text(t.to_owned());
        let number = NewValue::Number;
        let book = NewBook {
            sheet: "Sheet1".to_owned(),
            rows: vec![
                (1, vec![cell(2, text("2024"), 0), cell(3, text("2025"), 0)]),
                (
                    2,
                    vec![
                        cell(1, text("North"), 0),
                        cell(2, number(1.0), 1),
                        cell(3, number(3.0), 1),
                    ],
                ),
                (
                    3,
                    vec![
                        cell(1, text(" South"), 0),
                        cell(2, number(2.5), 1),
                        cell(3, number(4.0), 1),
                    ],
                ),
                (4, vec![cell(1, text("East & West"), 0), cell(3, number(5.0), 1)]),
            ],
            dimension: "A1:C4".to_owned(),
            formats: vec!["0.0\"x\"".to_owned()],
        };
        let written = book.write().unwrap_or_default();
        // the same entries and contents (the deflated bytes are zlib's own choice)
        let entries = |bytes: &[u8]| -> Vec<(String, Vec<u8>)> {
            let archive = Archive::read(Arc::from(bytes), &Limits::default()).ok();
            let archive = archive.as_ref();
            let entries = archive.map(Archive::entries).unwrap_or_default();
            entries
                .iter()
                .map(|e| {
                    let data = archive.and_then(|a| a.read_entry(e).ok()).unwrap_or_default();
                    let m = e.meta();
                    let made = (
                        m.version_made_by,
                        m.version_needed,
                        m.flags,
                        m.method,
                        m.dos_time,
                        m.dos_date,
                    );
                    (format!("{} {made:?}", m.name), data)
                })
                .collect()
        };
        let golden = entries(include_bytes!("golden/chart_column.xlsx"));
        assert_eq!(golden.len(), 7);
        assert_eq!(entries(&written), golden);
    }
}
