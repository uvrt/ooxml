//! Where resolved values came from (`resolve::*_traced`): the level, the style of the
//! chain that declared each value, its region and properties element; and that tracing
//! leaves the values as the untraced resolution gives them.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use ooxml_word_model::docx::tables::cell_conditions;
use ooxml_word_model::model::{Look, Table};
use ooxml_word_model::resolve::{
    resolve_paragraph, resolve_paragraph_traced, resolve_run, resolve_run_traced, resolve_table, resolve_table_traced,
    Holder, Level, Origins, ParagraphInput, RunInput, Sources,
};
use ooxml_word_model::value::{key, Props};
use ooxml_word_model::{read_context, Context};
use std::sync::Arc;

const W: &str = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main""#;

fn context() -> Context {
    let styles = format!(
        r#"<w:styles {W}>
  <w:docDefaults><w:rPrDefault><w:rPr><w:sz w:val="22"/></w:rPr></w:rPrDefault>
    <w:pPrDefault><w:pPr><w:spacing w:after="160"/></w:pPr></w:pPrDefault></w:docDefaults>
  <w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/></w:style>
  <w:style w:type="paragraph" w:styleId="Base"><w:name w:val="Base"/><w:basedOn w:val="Normal"/>
    <w:pPr><w:shd w:val="clear" w:color="auto" w:fill="DDDDDD"/></w:pPr>
    <w:rPr><w:color w:val="1F3864" w:themeColor="accent1" w:themeShade="80"/></w:rPr></w:style>
  <w:style w:type="paragraph" w:styleId="Child"><w:name w:val="Child"/><w:basedOn w:val="Base"/>
    <w:pPr><w:jc w:val="center"/></w:pPr></w:style>
  <w:style w:type="table" w:styleId="Grid"><w:name w:val="Grid"/>
    <w:tblPr><w:tblBorders><w:insideH w:val="single" w:sz="4" w:color="000000"/></w:tblBorders></w:tblPr>
    <w:tblStylePr w:type="firstRow"><w:rPr><w:b/></w:rPr>
      <w:tcPr><w:tcBorders><w:insideV w:val="single" w:sz="8" w:color="FF0000"/></w:tcBorders>
      <w:shd w:val="clear" w:color="auto" w:fill="4472C4"/></w:tcPr></w:tblStylePr>
  </w:style>
</w:styles>"#
    );
    let tree = ooxml_xml::parse(styles.as_bytes(), &ooxml_xml::Limits::default()).expect("styles");
    read_context(None, None, Some(&tree), None)
}

fn sources(cx: &Context) -> Sources<'_> {
    Sources {
        styles: cx.styles.as_deref(),
        numbering: &cx.numbering,
        settings: &cx.settings,
    }
}

fn style_name(cx: &Context, origins: &Origins, k: &str) -> Option<String> {
    let sheet = cx.styles.as_deref()?;
    let origin = origins.of(k)?;
    Some(origin.declared_in(sheet)?.style_id.to_string())
}

#[test]
fn a_paragraph_and_a_run_say_which_style_of_the_chain_declared_each_value() {
    let cx = context();
    let direct: Props = [(
        key("ind.left").to_string().into(),
        ooxml_word_model::value::Value::Int(720),
    )]
    .into_iter()
    .collect();
    let input = ParagraphInput {
        style_id: Some("Child"),
        table_style: None,
        conditions: &[],
        direct: &direct,
    };
    let (props, origins) = resolve_paragraph_traced::<Origins>(&sources(&cx), &input);
    assert_eq!(props, resolve_paragraph(&sources(&cx), &input));
    assert_eq!(style_name(&cx, &origins, "shd").as_deref(), Some("Base"));
    assert_eq!(style_name(&cx, &origins, "jc").as_deref(), Some("Child"));
    let shd = origins.of("shd").copied();
    assert_eq!(
        shd.map(|o| (o.level, o.holder)),
        Some((Level::ParagraphStyle, Holder::Paragraph))
    );
    assert_eq!(origins.of("spacing.after").map(|o| o.level), Some(Level::DocDefaults));
    assert_eq!(origins.of("ind.left").map(|o| o.level), Some(Level::Direct));

    let run = RunInput {
        paragraph_style: Some("Child"),
        table_style: None,
        conditions: &[],
        character_style: None,
        toc_link: false,
        direct: &Props::new(),
    };
    let (props, origins) = resolve_run_traced::<Origins>(&sources(&cx), &run);
    assert_eq!(props, resolve_run(&sources(&cx), &run));
    assert_eq!(style_name(&cx, &origins, "color").as_deref(), Some("Base"));
    assert_eq!(origins.of("color").map(|o| o.holder), Some(Holder::Run));
    assert_eq!(origins.of("sz").map(|o| o.level), Some(Level::DocDefaults));
}

#[test]
fn a_cell_says_which_region_declared_its_fill_and_a_border_read_from_an_inside_line() {
    let cx = context();
    let look = Look {
        first_row: true,
        no_v_band: true,
        no_h_band: true,
        ..Look::default()
    };
    let conditions: Vec<_> = (0..2)
        .map(|r| -> Arc<[_]> {
            (0..2)
                .map(|c| -> Arc<[_]> { Arc::from(cell_conditions(look, r, c, 2, 2, 0)) })
                .collect()
        })
        .collect();
    let empty: Arc<[Props]> = Arc::from(vec![Props::new(), Props::new()]);
    let table = Table {
        style_id: Some(key("Grid")),
        grid: Arc::from(vec![1440, 1440]),
        row_properties: Arc::clone(&empty),
        cell_properties: Arc::from(vec![Arc::clone(&empty), Arc::clone(&empty)]),
        rows: Arc::from(vec![Arc::from(vec![Vec::new().into(), Vec::new().into()]); 2]),
        look,
        conditions: Arc::from(conditions),
        ..Table::default()
    };
    let (resolved, origins) = resolve_table_traced::<Origins>(&sources(&cx), &table);
    assert_eq!(resolved, resolve_table(&sources(&cx), &table));
    let sheet = cx.styles.as_deref().expect("styles");
    let header = &origins.cells[0][0];
    let fill = header.of("shd").copied().expect("no shd origin");
    let style = fill.declared_in(sheet).expect("no style");
    assert_eq!(&*style.style_id, "Grid");
    assert_eq!(fill.region_type(style).map(|t| &**t), Some("firstRow"));
    assert_eq!((fill.holder, fill.inside), (Holder::Cell, false));
    // The header row's first cell: its right side is inside the region (insideV).
    let right = header.of("tcBorders.right").copied().expect("no border origin");
    assert!(right.inside);
    let left = header.of("tcBorders.left").copied();
    assert!(left.is_none(), "the left edge is the region's edge, which states none");
    let table_origin = origins.table.of("tblBorders.insideH").copied();
    assert_eq!(
        table_origin.map(|o| (o.level, o.holder)),
        Some((Level::TableStyle, Holder::Table))
    );
    assert!(origins.cells[1][0].of("shd").is_none());
}
