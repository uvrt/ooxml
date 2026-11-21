//! `word_design_facts` on a made-up document: a theme, a heading style coloured by a theme
//! colour with a shade, a list style, a table style whose header row is filled and bordered
//! by theme colours, two headings each with the same table under it. The theme colours are
//! labelled by the element that declared them; the replies are docx-agent's (checked
//! against it when written). Equality with docx-agent on the corpus is
//! `word_edits_diff.py --style`'s.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use crate::common::{Never, R};
use ooxml_base::Limits;
use ooxml_opc::Package;
use ooxml_tools::ToolArgs;
use ooxml_tools_word::{word_design_facts, Version, WordDesignFacts};
use ooxml_word_model::docx::Versions;
use ooxml_zip::{Method, NewEntry, Writer};
use serde_json::{json, Value};
use std::sync::Arc;

const W: &str = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main""#;
const CT: &str = "application/vnd.openxmlformats-officedocument";

const THEME: &str = r#"<a:theme xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" name="Made Up"><a:themeElements><a:clrScheme name="M"><a:dk1><a:sysClr val="windowText" lastClr="000000"/></a:dk1><a:lt1><a:srgbClr val="FFFFFF"/></a:lt1><a:dk2><a:srgbClr val="44546A"/></a:dk2><a:lt2><a:srgbClr val="E7E6E6"/></a:lt2><a:accent1><a:srgbClr val="4472C4"/></a:accent1><a:accent2><a:srgbClr val="ED7D31"/></a:accent2><a:accent3><a:srgbClr val="A5A5A5"/></a:accent3><a:accent4><a:srgbClr val="FFC000"/></a:accent4><a:accent5><a:srgbClr val="5B9BD5"/></a:accent5><a:accent6><a:srgbClr val="70AD47"/></a:accent6><a:hlink><a:srgbClr val="0563C1"/></a:hlink><a:folHlink><a:srgbClr val="954F72"/></a:folHlink></a:clrScheme><a:fontScheme name="M"><a:majorFont><a:latin typeface="Calibri Light"/><a:ea typeface=""/><a:cs typeface=""/></a:majorFont><a:minorFont><a:latin typeface="Calibri"/><a:ea typeface=""/><a:cs typeface=""/></a:minorFont></a:fontScheme><a:fmtScheme name="M"/></a:themeElements></a:theme>"#;

const STYLES: &str = r#"<w:docDefaults><w:rPrDefault><w:rPr><w:rFonts w:asciiTheme="minorHAnsi" w:hAnsiTheme="minorHAnsi"/><w:sz w:val="22"/></w:rPr></w:rPrDefault><w:pPrDefault><w:pPr><w:spacing w:after="160" w:line="259" w:lineRule="auto"/></w:pPr></w:pPrDefault></w:docDefaults><w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/></w:style><w:style w:type="paragraph" w:styleId="Heading1"><w:name w:val="heading 1"/><w:basedOn w:val="Normal"/><w:next w:val="Normal"/><w:pPr><w:keepNext/><w:keepLines/><w:spacing w:before="240" w:after="0"/><w:outlineLvl w:val="0"/></w:pPr><w:rPr><w:color w:val="2F5496" w:themeColor="accent1" w:themeShade="BF"/><w:sz w:val="32"/></w:rPr></w:style><w:style w:type="paragraph" w:styleId="Quote"><w:name w:val="Quote"/><w:basedOn w:val="Normal"/><w:pPr><w:pBdr><w:left w:val="single" w:sz="12" w:space="4" w:color="ED7D31" w:themeColor="accent2"/></w:pBdr><w:shd w:val="clear" w:color="auto" w:fill="DEEAF6" w:themeFill="accent1" w:themeFillTint="33"/></w:pPr></w:style><w:style w:type="character" w:styleId="Strong"><w:name w:val="Strong"/><w:rPr><w:b/></w:rPr></w:style><w:style w:type="table" w:default="1" w:styleId="TableNormal"><w:name w:val="Normal Table"/><w:tblPr><w:tblCellMar><w:left w:w="108" w:type="dxa"/><w:right w:w="108" w:type="dxa"/></w:tblCellMar></w:tblPr></w:style><w:style w:type="table" w:styleId="Grid"><w:name w:val="Made Grid"/><w:basedOn w:val="TableNormal"/><w:tblPr><w:tblBorders><w:top w:val="single" w:sz="4" w:space="0" w:color="auto"/><w:bottom w:val="single" w:sz="4" w:space="0" w:color="auto"/><w:insideH w:val="single" w:sz="4" w:space="0" w:color="A5A5A5" w:themeColor="accent3"/></w:tblBorders></w:tblPr><w:tblStylePr w:type="firstRow"><w:rPr><w:b/><w:color w:val="FFFFFF" w:themeColor="background1"/></w:rPr><w:tcPr><w:tcBorders><w:bottom w:val="single" w:sz="12" w:space="0" w:color="2F5496" w:themeColor="accent1" w:themeShade="BF"/></w:tcBorders><w:shd w:val="clear" w:color="auto" w:fill="4472C4" w:themeFill="accent1"/></w:tcPr></w:tblStylePr></w:style>"#;

fn table() -> String {
    let cell = |text: &str| {
        format!(
            r#"<w:tc><w:tcPr><w:tcW w:w="2000" w:type="dxa"/></w:tcPr><w:p><w:r><w:t>{text}</w:t></w:r></w:p></w:tc>"#
        )
    };
    let row = |a: &str, b: &str| format!("<w:tr>{}{}</w:tr>", cell(a), cell(b));
    format!(
        r#"<w:tbl><w:tblPr><w:tblStyle w:val="Grid"/><w:tblW w:w="4000" w:type="dxa"/><w:tblLook w:val="04A0" w:firstRow="1" w:lastRow="0" w:firstColumn="1" w:lastColumn="0" w:noHBand="0" w:noVBand="1"/></w:tblPr><w:tblGrid><w:gridCol w:w="2000"/><w:gridCol w:w="2000"/></w:tblGrid>{}{}{}</w:tbl>"#,
        row("Name", "Value"),
        row("a", "1"),
        row("b", ""),
    )
}

fn body() -> String {
    let heading = |id: &str, text: &str| {
        format!(
            r#"<w:p w14:paraId="{id}" xmlns:w14="http://schemas.microsoft.com/office/word/2010/wordml"><w:pPr><w:pStyle w:val="Heading1"/></w:pPr><w:r><w:t>{text}</w:t></w:r></w:p>"#
        )
    };
    let quote = r#"<w:p><w:pPr><w:pStyle w:val="Quote"/></w:pPr><w:r><w:rPr><w:rStyle w:val="Strong"/></w:rPr><w:t>Said.</w:t></w:r></w:p>"#;
    format!(
        r#"{quote}{}<w:p><w:r><w:t>Intro.</w:t></w:r></w:p>{}{}<w:p><w:r><w:t>Intro.</w:t></w:r></w:p>{}<w:p/><w:sectPr><w:pgSz w:w="12240" w:h="15840"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440" w:header="720" w:footer="720" w:gutter="0"/><w:cols w:space="720"/></w:sectPr>"#,
        heading("10000001", "One"),
        table(),
        heading("10000002", "Two"),
        table(),
    )
}

fn package() -> Package {
    let parts = [
        (
            "[Content_Types].xml".to_owned(),
            format!(
                r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="{CT}.wordprocessingml.document.main+xml"/><Override PartName="/word/styles.xml" ContentType="{CT}.wordprocessingml.styles+xml"/><Override PartName="/word/theme/theme1.xml" ContentType="{CT}.theme+xml"/></Types>"#
            ),
        ),
        (
            "_rels/.rels".to_owned(),
            format!(
                r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="{R}officeDocument" Target="word/document.xml"/></Relationships>"#
            ),
        ),
        (
            "word/_rels/document.xml.rels".to_owned(),
            format!(
                r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="{R}styles" Target="styles.xml"/><Relationship Id="rId2" Type="{R}theme" Target="theme/theme1.xml"/></Relationships>"#
            ),
        ),
        (
            "word/styles.xml".to_owned(),
            format!("<w:styles {W}>{STYLES}</w:styles>"),
        ),
        ("word/theme/theme1.xml".to_owned(), THEME.to_owned()),
        (
            "word/document.xml".to_owned(),
            format!("<w:document {W}><w:body>{}</w:body></w:document>", body()),
        ),
    ];
    let mut w = Writer::new();
    for (name, data) in parts {
        let entry = NewEntry {
            name,
            method: Method::Deflated,
            dos_time: 0,
            dos_date: 33,
            external_attr: 0,
            create_system: 3,
        };
        w.add(&entry, data.as_bytes()).unwrap();
    }
    Package::open(Arc::from(w.finish(b"").unwrap()), &Limits::default()).unwrap()
}

fn reply(arguments: Value) -> Value {
    let package = package();
    let versions = Versions::new();
    let version = Version::new("a.docx", &package, 1, &versions);
    let (args, _) = WordDesignFacts::accept(arguments, Some("docx")).unwrap();
    match word_design_facts(&args, &version, &Never) {
        Ok(done) => json!({"summary": done.summary, "data": done.data, "total": done.total, "next": done.next_cursor}),
        Err(error) => json!({"error": error.message()}),
    }
}

#[test]
fn a_theme_colour_is_labelled_by_the_element_that_declared_it() {
    let got = reply(json!({"doc": "d1", "include": ["styles"]}));
    let styles = got["data"]["styles"].as_array().unwrap();
    let heading = styles.iter().find(|s| s["role"] == "heading 1").unwrap();
    assert_eq!(heading["look"]["color"], "2F5496");
    assert_eq!(heading["look"]["theme_color"], "accent1 shade BF");
    assert_eq!(heading["look"]["keep_lines"], true, "said as word_format says it");
    assert_eq!(heading["paragraphs"], 2);
    let quote = styles.iter().find(|s| s["style"] == "Quote").unwrap();
    assert_eq!(quote["look"]["theme_shading"], "accent1 tint 33");
    assert_eq!(quote["look"]["borders"]["left"], "single 1.5pt ED7D31 (accent2)");
    let table = styles.iter().find(|s| s["role"] == "table").unwrap();
    assert_eq!(table["table_style"], "Made Grid");
    assert_eq!(table["paragraphs"], 12);
}

#[test]
fn a_table_style_by_place_names_its_theme_fill_and_borders() {
    let got = reply(json!({"doc": "d1", "include": ["table_styles"]}));
    let [grid] = got["data"]["table_styles"].as_array().unwrap().as_slice() else {
        panic!("{got}");
    };
    assert_eq!(grid["style"], "Made Grid");
    assert_eq!(grid["tables"], 2);
    let header = &grid["places"]["header_row"];
    assert_eq!(header["fill"], "4472C4");
    assert_eq!(header["theme_fill"], "accent1");
    assert_eq!(header["borders"]["bottom"], "single 1.5pt 2F5496 (accent1 shade BF)");
    assert_eq!(header["text"]["theme_color"], "background1");
    assert_eq!(header["text"]["bold"], true);
    // an inside line of the table's own borders, under the first band
    assert_eq!(
        grid["places"]["band1"]["borders"]["bottom"],
        "single 0.5pt A5A5A5 (accent3)"
    );
}

#[test]
fn frames_sections_theme_and_a_repeated_shape() {
    let got = reply(json!({"doc": "d1"}));
    let data = &got["data"];
    let frames = data["table_frames"].as_array().unwrap();
    assert_eq!(frames.len(), 2);
    assert_eq!(frames[0]["width"], 200);
    assert_eq!(frames[0]["body"], 468);
    assert_eq!(frames[0]["empty_last_cell"], json!([{"row": 2, "height": null}]));
    assert_eq!(data["usual_table_width"], 200);
    assert_eq!(data["sections"][0]["margins"]["gutter"], 0);
    assert_eq!(data["sections"][0]["columns"], json!({"count": 1, "space": 36}));
    assert_eq!(data["theme"]["name"], "Made Up");
    assert_eq!(data["theme"]["colors"]["accent1"], "4472C4");
    let [pattern] = data["patterns"].as_array().unwrap().as_slice() else {
        panic!("{data}");
    };
    assert_eq!(pattern["shape"], json!(["Normal", "table Made Grid, 2 columns"]));
    assert_eq!(pattern["of"], 2);
    assert!(got["summary"].as_str().unwrap().starts_with("Design facts: "));
}

#[test]
fn declared_styles_differ_from_the_default_and_page_by_cursor() {
    let got = reply(json!({"doc": "d1", "include": ["declared_styles"]}));
    let declared = got["data"]["declared_styles"].as_array().unwrap();
    assert_eq!(got["total"], 6);
    let normal = declared.iter().find(|s| s["name"] == "Normal").unwrap();
    assert_eq!(normal["default"], true);
    let strong = declared.iter().find(|s| s["name"] == "Strong").unwrap();
    assert_eq!(strong["look"], json!({"bold": true}));
    let grid = declared.iter().find(|s| s["name"] == "Made Grid").unwrap();
    assert_eq!(grid["based_on"], json!(["Normal Table"]));
    assert_eq!(grid["parts"], json!(["firstRow"]));
    let later = reply(json!({"doc": "d1", "cursor": "c4"}));
    assert_eq!(later["data"]["declared_styles"].as_array().unwrap().len(), 2);
    assert!(reply(json!({"doc": "d1", "cursor": "x"}))["error"].is_string());
}

#[test]
#[ignore = "a timing on a long document: DESIGN_BENCH=<docx> cargo test --release -- --ignored"]
fn design_facts_on_a_long_document() {
    let Ok(path) = std::env::var("DESIGN_BENCH") else {
        return;
    };
    let package = Package::open(Arc::from(std::fs::read(path).unwrap()), &Limits::default()).unwrap();
    let versions = Versions::new();
    let version = Version::new("long.docx", &package, 1, &versions);
    let start = std::time::Instant::now();
    let _ = version.docx().unwrap();
    println!("the version read: {:.1} ms", start.elapsed().as_secs_f64() * 1000.0);
    let kinds = [
        "styles",
        "table_styles",
        "table_frames",
        "sections",
        "theme",
        "patterns",
        "declared_styles",
    ];
    let mut calls = vec![("default".to_owned(), json!({"doc": "d1"}))];
    calls.extend(kinds.map(|k| (k.to_owned(), json!({"doc": "d1", "include": [k]}))));
    for (what, arguments) in calls {
        let (args, _) = WordDesignFacts::accept(arguments, Some("docx")).unwrap();
        let mut times = Vec::new();
        for _ in 0..3 {
            let start = std::time::Instant::now();
            let done = word_design_facts(&args, &version, &Never).unwrap();
            times.push(start.elapsed().as_secs_f64() * 1000.0);
            assert!(done.summary.starts_with("Design facts"));
        }
        let best = times.iter().skip(1).copied().fold(f64::MAX, f64::min);
        println!(
            "design facts {what}: first {:.1} ms, then {best:.1} ms (best of 2)",
            times[0]
        );
    }
}
