//! A made-up theme and shape read and resolved: the colour map layered, a scheme colour
//! with its transforms, a style reference's `phClr`, a custom geometry and a text body.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use ooxml_colour::{effective_colour_map, ColourMap, ResolvedColour, POWERPOINT, WORD};
use ooxml_drawingml::read::{fill, theme, tree};
use ooxml_drawingml::resolve::{self, colour_context, Resolved};
use ooxml_drawingml::source::{Fill, Geometry};
use ooxml_drawingml::text::Bullet;
use ooxml_drawingml::tree::Node;
use ooxml_xml::{parse, Limits, Tree};

const A: &str = r#"xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:p="urn:p" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships""#;

fn tree_of(xml: &str) -> Tree {
    parse(xml.as_bytes(), &Limits::default()).unwrap()
}

fn theme_xml() -> String {
    format!(
        r#"<a:theme {A}><a:themeElements>
             <a:clrScheme name="t"><a:dk1><a:sysClr val="windowText" lastClr="101010"/></a:dk1>
               <a:lt1><a:srgbClr val="fafafa"/></a:lt1><a:accent1><a:srgbClr val="4472C4"/></a:accent1></a:clrScheme>
             <a:fmtScheme><a:fillStyleLst><a:solidFill><a:schemeClr val="phClr"/></a:solidFill>
               <a:solidFill><a:schemeClr val="phClr"><a:lumMod val="50000"/></a:schemeClr></a:solidFill></a:fillStyleLst>
               <a:lnStyleLst><a:ln w="6350"><a:solidFill><a:schemeClr val="phClr"/></a:solidFill></a:ln></a:lnStyleLst>
               <a:effectStyleLst><a:effectStyle><a:effectLst/></a:effectStyle></a:effectStyleLst>
               <a:bgFillStyleLst><a:noFill/></a:bgFillStyleLst></a:fmtScheme>
           </a:themeElements></a:theme>"#
    )
}

#[test]
fn a_theme_reads_and_its_styles_resolve_with_the_reference_colour() {
    let t = tree_of(&theme_xml());
    let read = theme::theme(t.root());
    assert_eq!(
        read.colours.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>(),
        ["dk1", "lt1", "accent1"]
    );
    assert_eq!(read.formats.fill_styles.len(), 2);
    assert_eq!(read.formats.effect_styles, [None]);
    assert_eq!(read.formats.fill_style(1001), Some(&Fill::None));
    assert!(read.formats.fill_style(1000).is_none() && read.formats.fill_style(0).is_none());

    let master: ColourMap = [("tx1".to_owned(), "lt1".to_owned())].into();
    let map = effective_colour_map(&[Some(&master), None]);
    let context = colour_context(Some(&read), map, POWERPOINT);
    let tx1 = ooxml_colour::SourceColour::scheme("tx1");
    assert_eq!(context.resolve(&tx1).unwrap().unwrap().hex(), "#fafafa");
    let accent = ooxml_colour::SourceColour::scheme("accent1");
    let placeholder = context.resolve(&accent).unwrap();
    let shaded = resolve::fill(&context, read.formats.fill_style(2).unwrap(), placeholder, None).unwrap();
    let Some(Resolved::Solid(ResolvedColour { rgb, .. })) = shaded else {
        panic!("{shaded:?}");
    };
    // ooxml-common: apply_transforms("4472C4", [lumMod 50000], POWERPOINT) is #203864.
    assert_eq!(rgb, [0x20, 0x38, 0x64]);
    let no_placeholder = resolve::fill(&context, read.formats.fill_style(1).unwrap(), None, None).unwrap();
    assert_eq!(no_placeholder, None);
}

#[test]
fn a_shape_reads_its_geometry_fill_and_text() {
    let xml = format!(
        r#"<p:spTree {A}><p:sp><p:nvSpPr><p:cNvPr id="4" name="Box" descr=""/><p:cNvSpPr/><p:nvPr><p:ph type="body" idx="1"/></p:nvPr></p:nvSpPr>
             <p:spPr><a:xfrm><a:off x="1" y="2"/><a:ext cx="300" cy="200"/></a:xfrm>
               <a:custGeom><a:pathLst><a:path w="100" h="100"><a:moveTo><a:pt x="0" y="0"/></a:moveTo>
                 <a:arcTo wR="50" hR="50" stAng="10800000" swAng="21600000"/><a:close/></a:path></a:pathLst></a:custGeom>
               <a:gradFill rotWithShape="1"><a:gsLst><a:gs pos="0"><a:prstClr val="dkSeaGreen"/></a:gs></a:gsLst><a:lin ang="5400000"/></a:gradFill>
             </p:spPr>
             <p:txBody><a:bodyPr anchor="dist" numCol="0"/><a:p><a:pPr algn="justLow"><a:buChar char="&amp;#x2022;"/></a:pPr>
               <a:r><a:rPr sz="1800"><a:hlinkClick r:id="rId9"/></a:rPr><a:t>Hi</a:t></a:r><a:br/></a:p></p:txBody></p:sp></p:spTree>"#
    );
    let t = tree_of(&xml);
    let nodes = tree::shape_tree(Some(t.root()));
    let Node::Shape(shape) = &nodes[0] else {
        panic!("{nodes:?}");
    };
    assert_eq!(shape.info.name.as_deref(), Some("Box"));
    assert_eq!(shape.info.alt_text, None);
    assert_eq!(shape.placeholder.as_ref().and_then(|p| p.idx), Some(1));
    let Some(Geometry::Custom(paths)) = &shape.geometry else {
        panic!("{:?}", shape.geometry);
    };
    assert_eq!(paths[0].commands, "M 0 0 A 50 50 0 0 1 100 0 A 50 50 0 0 1 0 0 Z");
    let Some(Fill::Gradient(g)) = &shape.fill else {
        panic!("{:?}", shape.fill);
    };
    assert_eq!(g.angle, 5_400_000.0);
    let text = shape.text.as_ref().unwrap();
    let body = text.properties.as_ref().unwrap();
    assert_eq!((body.anchor.as_deref(), body.columns), (Some("t"), Some(1)));
    let paragraph = &text.paragraphs[0];
    let p = paragraph.properties.as_ref().unwrap();
    assert_eq!(
        (p.align.as_deref(), &p.bullet),
        (Some("just"), &Some(Bullet::Char("\u{2022}".into())))
    );
    let run = paragraph.runs[0].properties.as_ref().unwrap();
    assert_eq!(
        (run.size, run.underline, run.hyperlink.as_deref()),
        (Some(18.0), Some(true), Some("rId9"))
    );
    assert_eq!(paragraph.runs[1].text, "\n");
    let context = colour_context(None, effective_colour_map(&[]), WORD);
    let resolved = resolve::fill(&context, shape.fill.as_ref().unwrap(), None, None).unwrap();
    let Some(Resolved::Gradient(g)) = resolved else {
        panic!("{resolved:?}");
    };
    assert_eq!((g.angle, g.stops[0].colour.hex()), (90.0, "#8fbc8f".to_owned()));
    assert_eq!(fill::fill(None), None);
}
