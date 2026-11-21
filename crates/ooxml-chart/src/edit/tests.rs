//! The cache edits against what ooxml-edit's `_Data.set_point` and `_give_name` write
//! (worked out with them, 2026-11-21): a point changed, made in index order, blanked; text
//! refused in numeric data; a literal name given.

use super::{ChartTree, PointValue, Which};
use ooxml_xml::Limits;

const CHART: &str = r#"<c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart"><c:chart><c:plotArea><c:barChart><c:ser><c:idx val="0"/><c:order val="0"/><c:cat><c:strRef><c:f>Sheet1!$A$2:$A$4</c:f><c:strCache><c:ptCount val="3"/><c:pt idx="0"><c:v>Q1</c:v></c:pt><c:pt idx="2"><c:v>Q3</c:v></c:pt></c:strCache></c:strRef></c:cat><c:val><c:numRef><c:f>Sheet1!$B$2:$B$4</c:f><c:numCache><c:formatCode>General</c:formatCode><c:ptCount val="3"/><c:pt idx="2"><c:v>3</c:v></c:pt><c:extLst/></c:numCache></c:numRef></c:val></c:ser></c:barChart></c:plotArea></c:chart></c:chartSpace>"#;

fn chart() -> ChartTree {
    let tree = ooxml_xml::parse(CHART.as_bytes(), &Limits::default()).unwrap_or_else(|e| panic!("{e}"));
    ChartTree::new(std::sync::Arc::clone(tree.root()))
}

fn written(chart: &ChartTree) -> String {
    String::from_utf8(ooxml_xml::Tree::from_root(std::sync::Arc::clone(chart.root())).write()).unwrap_or_default()
}

#[test]
fn points_are_written_in_index_order() {
    let mut chart = chart();
    assert_eq!(chart.set_point(0, Which::Values, 0, &PointValue::Number(1.5)), Ok(true));
    assert_eq!(chart.set_point(0, Which::Values, 1, &PointValue::Number(2.0)), Ok(true));
    assert_eq!(
        chart.set_point(0, Which::Values, 2, &PointValue::Number(3.0)),
        Ok(false)
    );
    assert_eq!(
        chart.set_point(0, Which::Categories, 1, &PointValue::Text(String::new())),
        Ok(true)
    );
    assert_eq!(chart.set_point(0, Which::Categories, 0, &PointValue::Blank), Ok(true));
    let out = written(&chart);
    assert!(out.contains(r#"<c:ptCount val="3"/><c:pt idx="0"><c:v>1.5</c:v></c:pt><c:pt idx="1"><c:v>2</c:v></c:pt><c:pt idx="2"><c:v>3</c:v></c:pt><c:extLst/>"#), "{out}");
    assert!(
        out.contains(r#"<c:strCache><c:ptCount val="3"/><c:pt idx="1"><c:v></c:v></c:pt><c:pt idx="2">"#),
        "{out}"
    );
}

#[test]
fn text_in_numbers_is_refused_and_a_name_is_given() {
    let mut chart = chart();
    let refused = chart.set_point(0, Which::Values, 0, &PointValue::Text("x".into()));
    assert_eq!(
        refused.map_err(|e| e.0),
        Err("'x' is not a number, and this data is numeric".to_owned())
    );
    assert_eq!(chart.source(0, Which::Name), None);
    chart.give_name(0, "North");
    assert!(written(&chart).contains(r#"<c:order val="0"/><c:tx><c:v>North</c:v></c:tx><c:cat>"#));
    assert_eq!(chart.source(0, Which::Name).map(|s| s.kind), Some("v".to_owned()));
}

#[test]
fn a_category_goes_in_and_out_of_every_source() {
    let mut chart = chart();
    assert_eq!(chart.point_count(), 3);
    let label = PointValue::Text("Q4".to_owned());
    assert_eq!(chart.insert_category(1, Some(&label), &[Some(9.0)]), Ok(()));
    let out = written(&chart);
    assert!(out.contains(r#"<c:ptCount val="4"/><c:pt idx="0"><c:v>Q1</c:v></c:pt><c:pt idx="1"><c:v>Q4</c:v></c:pt><c:pt idx="3"><c:v>Q3</c:v></c:pt>"#), "{out}");
    assert!(
        out.contains(r#"<c:pt idx="1"><c:v>9</c:v></c:pt><c:pt idx="3"><c:v>3</c:v></c:pt><c:extLst/>"#),
        "{out}"
    );
    assert_eq!(chart.delete_category(1), Ok(()));
    assert_eq!(written(&chart), written(&self::chart()));
}

#[test]
fn a_series_is_copied_from_the_last_with_its_own_id() {
    let mut chart = chart();
    let hash = |_: &[u8]| [0xAB; 32];
    assert_eq!(
        chart.append_series(
            Some("New"),
            &[Some(1.0), None, Some(2.5)],
            "word/charts/chart1.xml",
            &hash
        ),
        Ok(())
    );
    assert_eq!(chart.series_count(), 2);
    assert_eq!(chart.series_idx(1), 1);
    let out = written(&chart);
    assert!(
        out.contains(r#"<c:ser><c:idx val="1"/><c:order val="1"/><c:tx><c:v>New</c:v></c:tx>"#),
        "{out}"
    );
    assert!(
        out.contains(r#"<c:pt idx="0"><c:v>1</c:v></c:pt><c:pt idx="2"><c:v>2.5</c:v></c:pt>"#),
        "{out}"
    );
    assert!(
        out.contains(r#"<c16:uniqueId val="{00000000-ABAB-ABAB-ABAB-ABABABABABAB}"/>"#),
        "{out}"
    );
    chart.delete_series(1);
    assert_eq!(written(&chart), written(&self::chart()));
}

#[test]
fn the_legend_and_the_gap_width_are_set_once() {
    let mut chart = chart();
    assert!(chart.set_legend(Some("b")));
    assert!(!chart.set_legend(Some("t")));
    assert!(written(&chart).contains(r#"</c:plotArea><c:legend><c:legendPos val="b"/><c:overlay val="0"/></c:legend>"#));
    assert!(chart.set_legend(None));
    assert_eq!(chart.gap_width(), Some(150));
    assert!(chart.set_gap_width(80));
    assert!(!chart.set_gap_width(80));
    assert!(written(&chart).contains(r#"</c:ser><c:gapWidth val="80"/></c:barChart>"#));
}
