//! `set_series_type` and `set_point_labels` against ooxml-edit's results (`charts/styling.py`).

use super::{ChartTree, PointLabels};
use ooxml_xml::Limits;
use std::sync::Arc;

const C: &str = r#"xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main""#;

/// A column chart of two series on axes 1 and 2.
fn columns() -> ChartTree {
    let series = |i: u32| {
        format!(
            r#"<c:ser><c:idx val="{i}"/><c:order val="{i}"/><c:spPr><a:solidFill><a:srgbClr val="FF0000"/></a:solidFill></c:spPr><c:invertIfNegative val="0"/><c:val><c:numLit><c:ptCount val="2"/><c:pt idx="0"><c:v>1</c:v></c:pt><c:pt idx="1"><c:v>2</c:v></c:pt></c:numLit></c:val></c:ser>"#
        )
    };
    let xml = format!(
        r#"<c:chartSpace {C}><c:chart><c:plotArea><c:layout/><c:barChart><c:barDir val="col"/><c:grouping val="clustered"/>{}{}<c:axId val="1"/><c:axId val="2"/></c:barChart><c:catAx><c:axId val="1"/></c:catAx><c:valAx><c:axId val="2"/></c:valAx></c:plotArea></c:chart></c:chartSpace>"#,
        series(0),
        series(1)
    );
    let tree = ooxml_xml::parse(xml.as_bytes(), &Limits::default()).unwrap_or_else(|e| panic!("{e:?}"));
    ChartTree::new(Arc::clone(tree.root()))
}

fn xml(tree: &ChartTree) -> String {
    String::from_utf8(ooxml_xml::Tree::from_root(Arc::clone(tree.root())).write()).unwrap_or_default()
}

fn hash(_: &[u8]) -> [u8; 32] {
    [0xAB; 32]
}

#[test]
fn a_series_drawn_as_a_line_on_secondary_axes_and_back() {
    let mut tree = columns();
    assert_eq!(tree.set_series_type("c", 1, true, true, None, &hash), Ok(true));
    let out = xml(&tree);
    assert!(out.contains(r#"<c:lineChart><c:grouping val="standard"/>"#), "{out}");
    assert!(
        out.contains(r#"<a:ln w="28575" cap="rnd"><a:solidFill><a:srgbClr val="FF0000"/>"#),
        "{out}"
    );
    // the axes after the primary ones: 0xabababa + 1 for the category, 0xbababab + 1 the value
    assert!(
        out.contains(r#"<c:valAx><c:axId val="2"/></c:valAx><c:valAx><c:axId val="195799980"/>"#),
        "{out}"
    );
    assert!(
        out.contains(r#"<c:axPos val="r"/><c:numFmt formatCode="General""#),
        "{out}"
    );
    assert_eq!(tree.set_series_type("c", 1, true, true, None, &hash), Ok(false));
    assert_eq!(
        tree.set_series_type("c", 1, false, true, None, &hash).map_err(|e| e.0),
        Err("c: columns stay on the primary axis".to_owned())
    );
    // back as a column: the line plot and its axes go
    assert_eq!(tree.set_series_type("c", 1, false, false, None, &hash), Ok(true));
    let back = xml(&tree);
    assert!(!back.contains("lineChart") && !back.contains("195799980"), "{back}");
    assert!(back.contains(r#"<c:spPr><a:solidFill><a:srgbClr val="FF0000"/></a:solidFill><a:ln><a:noFill/></a:ln><a:effectLst/></c:spPr><c:invertIfNegative val="0"/>"#), "{back}");
}

#[test]
fn point_labels_written_checked_and_taken_away() {
    let mut tree = columns();
    let labels = vec!["R1".to_owned(), "R2".to_owned()];
    assert_eq!(
        tree.check_point_labels("c", 0, Some(&labels), Some("right"))
            .map_err(|e| e.0),
        Err("c: label position 'right'; this chart takes center, inside_end, inside_base, outside_end".to_owned())
    );
    assert_eq!(
        tree.check_point_labels("c", 0, Some(&labels[..1]), None)
            .map_err(|e| e.0),
        Err("c: 1 labels for 2 points".to_owned())
    );
    let code = tree
        .check_point_labels("c", 0, Some(&labels), Some("center"))
        .unwrap_or_default();
    let look = PointLabels {
        labels: Some(&labels),
        position: code,
        cells: None,
        size: Some(900),
        lang: Some("en-US"),
        part: "word/charts/chart1.xml",
    };
    tree.set_point_labels(0, &look, &hash);
    let out = xml(&tree);
    assert!(
        out.contains(r#"<c:dLbl><c:idx val="0"/><c:tx><c:rich><a:bodyPr"#),
        "{out}"
    );
    assert!(
        out.contains(r#"<a:r><a:rPr lang="en-US"/><a:t>R2</a:t></a:r>"#),
        "{out}"
    );
    assert!(
        out.contains(r#"<c:dLblPos val="ctr"/><c:showLegendKey val="0"/><c:showVal val="1"/>"#),
        "{out}"
    );
    let gone = PointLabels { labels: None, ..look };
    tree.set_point_labels(0, &gone, &hash);
    assert!(!xml(&tree).contains("dLbl"), "{}", xml(&tree));
}
