//! The style edits against ooxml-edit's results (`charts/styling.py`, `format.py`).

use super::{ChartTree, Colour, DataError, ScaleChange, SeriesFormat, TextHost, TextLook, TextStop};
use ooxml_xml::Limits;
use std::fmt::Debug;
use std::sync::Arc;

fn ok<T, E: Debug>(result: Result<T, E>) -> T {
    result.unwrap_or_else(|e| panic!("{e:?}"))
}

fn err<T: Debug, E>(result: Result<T, E>) -> E {
    match result {
        Ok(done) => panic!("not refused: {done:?}"),
        Err(error) => error,
    }
}

const C: &str = r#"xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main""#;

fn chart(plot: &str, axes: &str) -> ChartTree {
    let xml = format!(
        r#"<c:chartSpace {C}><c:chart><c:plotArea><c:layout/>{plot}{axes}</c:plotArea><c:legend><c:legendPos val="r"/></c:legend></c:chart></c:chartSpace>"#
    );
    let tree = ok(ooxml_xml::parse(xml.as_bytes(), &Limits::default()));
    ChartTree::new(Arc::clone(tree.root()))
}

fn bars() -> ChartTree {
    let plot = r#"<c:barChart><c:barDir val="col"/><c:ser><c:idx val="0"/><c:order val="0"/><c:val><c:numRef><c:f>Sheet1!$B$2:$B$3</c:f><c:numCache><c:formatCode>General</c:formatCode><c:ptCount val="2"/><c:pt idx="0"><c:v>1</c:v></c:pt><c:pt idx="1"><c:v>2</c:v></c:pt></c:numCache></c:numRef></c:val></c:ser><c:axId val="1"/><c:axId val="2"/></c:barChart>"#;
    let axes = r#"<c:catAx><c:axId val="1"/><c:scaling><c:orientation val="minMax"/></c:scaling><c:delete val="0"/><c:axPos val="b"/><c:crossAx val="2"/><c:crosses val="autoZero"/></c:catAx><c:valAx><c:axId val="2"/><c:scaling><c:orientation val="minMax"/></c:scaling><c:delete val="0"/><c:axPos val="l"/><c:crossAx val="1"/><c:crosses val="autoZero"/><c:crossBetween val="between"/></c:valAx>"#;
    chart(plot, axes)
}

fn xml(tree: &ChartTree) -> String {
    String::from_utf8(ooxml_xml::Tree::from_root(Arc::clone(tree.root())).write()).unwrap_or_default()
}

#[test]
fn colours_read_as_ooxml_edit_reads_them() {
    let parsed = ok(Colour::parse("Accent 1 lumMod=75% alpha=50000 inv"));
    assert!(format!("{parsed:?}").contains("accent1"));
    assert!(Colour::parse("#1f4e79").is_ok());
    assert!(Colour::parse("text2").is_ok());
    let error = err(Colour::parse("blue"));
    assert!(error
        .0
        .starts_with("'blue' is neither a hex colour nor a theme colour (accent1, accent2"));
    assert_eq!(
        err(Colour::parse("tx1 lumMod=x")).0,
        "colour modifier 'lumMod=x': a percentage or an integer"
    );
    assert_eq!(err(Colour::parse("tx1 glow=1")).0, "unknown colour modifier 'glow=1'");
}

#[test]
fn a_scale_reversed_moves_the_crossing_axis() {
    let mut tree = bars();
    let change = ScaleChange {
        maximum: Some(Some(5.into())),
        major_unit: Some(Some(1.into())),
        number_format: Some(None),
        ..ScaleChange::default()
    };
    ok(tree.set_axis_scale("d:1", "value", &change, Some("General")));
    let out = xml(&tree);
    assert!(
        out.contains(r#"<c:orientation val="minMax"/><c:max val="5"/></c:scaling>"#),
        "{out}"
    );
    assert!(
        out.contains(r#"<c:numFmt formatCode="General" sourceLinked="1"/><c:crossAx"#),
        "{out}"
    );
    assert!(
        out.contains(r#"<c:crossBetween val="between"/><c:majorUnit val="1"/>"#),
        "{out}"
    );
    let reverse = ScaleChange {
        reverse: Some(true),
        ..ScaleChange::default()
    };
    ok(tree.set_axis_scale("d:1", "category", &reverse, None));
    let out = xml(&tree);
    assert!(out.contains(r#"<c:orientation val="maxMin"/>"#), "{out}");
    assert!(out.contains(r#"<c:crossAx val="1"/><c:crosses val="max"/>"#), "{out}");
}

#[test]
fn scales_refused_in_ooxml_edits_words() {
    let mut tree = bars();
    let refuse =
        |tree: &mut ChartTree, axis: &str, change: ScaleChange| err(tree.set_axis_scale("d:1", axis, &change, None)).0;
    let bounds = ScaleChange {
        minimum: Some(Some(1.into())),
        ..ScaleChange::default()
    };
    assert_eq!(
        refuse(&mut tree, "category", bounds.clone()),
        "d:1: the category axis shows categories, which have no numeric bounds or units; give them for the value axis"
    );
    assert_eq!(
        refuse(&mut tree, "x", bounds),
        "d:1: the x axis shows categories, which have no numeric bounds or units; give them for the value axis"
    );
    let crossed = ScaleChange {
        minimum: Some(Some(9.into())),
        maximum: Some(Some(3.5.into())),
        ..ScaleChange::default()
    };
    assert_eq!(
        refuse(&mut tree, "value", crossed),
        "d:1: the minimum (9) must be below the maximum (3.5)"
    );
    // JSON's 9.0 is Python's float: written as one.
    let floats = ScaleChange {
        minimum: Some(Some(9.0.into())),
        maximum: Some(Some(3.0.into())),
        ..ScaleChange::default()
    };
    assert_eq!(
        refuse(&mut tree, "value", floats),
        "d:1: the minimum (9.0) must be below the maximum (3.0)"
    );
    let unit = ScaleChange {
        minor_unit: Some(Some(0.0.into())),
        ..ScaleChange::default()
    };
    assert_eq!(refuse(&mut tree, "value", unit), "minor unit must be greater than 0");
}

#[test]
fn gridlines_as_office_draws_them() {
    let mut tree = bars();
    ok(tree.set_gridlines("d:1", "value", Some(true), Some(false)));
    let out = xml(&tree);
    assert!(out.contains(r#"<c:axPos val="l"/><c:majorGridlines><c:spPr><a:ln w="9525" cap="flat" cmpd="sng" algn="ctr"><a:solidFill><a:schemeClr val="tx1"><a:lumMod val="15000"/><a:lumOff val="85000"/></a:schemeClr></a:solidFill><a:round/></a:ln><a:effectLst/></c:spPr></c:majorGridlines><c:crossAx"#), "{out}");
    ok(tree.set_gridlines("d:1", "value", Some(false), None));
    assert!(!xml(&tree).contains("majorGridlines"));
}

#[test]
fn a_point_starts_from_its_series_look() {
    let mut tree = bars();
    let fill = SeriesFormat {
        fill: Some(Some("accent2".to_owned())),
        ..SeriesFormat::default()
    };
    ok(tree.format_series("d:1", 0, None, &fill));
    let point = SeriesFormat {
        line: Some(None),
        line_width: Some(1.5.into()),
        ..SeriesFormat::default()
    };
    ok(tree.format_series("d:1", 0, Some(1), &point));
    let out = xml(&tree);
    assert!(out.contains(r#"<c:order val="0"/><c:spPr><a:solidFill><a:schemeClr val="accent2"/></a:solidFill><a:effectLst/></c:spPr><c:dPt><c:idx val="1"/><c:invertIfNegative val="0"/><c:bubble3D val="0"/><c:spPr><a:solidFill><a:schemeClr val="accent2"/></a:solidFill><a:ln w="19050"><a:noFill/></a:ln><a:effectLst/></c:spPr></c:dPt><c:val>"#), "{out}");
    let markers = SeriesFormat {
        marker: Some("circle".to_owned()),
        ..SeriesFormat::default()
    };
    assert_eq!(
        tree.format_series("d:1", 0, None, &markers),
        Err(DataError(
            "d:1: only line, scatter and radar series have markers".to_owned()
        ))
    );
    assert_eq!(
        tree.format_series("d:1", 0, Some(2), &fill),
        Err(DataError("d:1: no point 2 (have 2)".to_owned()))
    );
}

#[test]
fn text_styled_everywhere_and_made_in_offices_style() {
    let mut tree = bars();
    let look = TextLook {
        size: Some(1000),
        ..TextLook::default()
    };
    let host = TextHost {
        labels: Some(900),
        data_labels: Some(900),
        lang: Some("en-US"),
    };
    assert!(matches!(
        tree.clone()
            .set_text_style("d:1", None, &look, &TextHost { lang: None, ..host }),
        Err(TextStop::NeedsLanguage(_))
    ));
    ok(tree.set_text_style("d:1", None, &look, &host));
    let out = xml(&tree);
    assert_eq!(out.matches(r#"<a:defRPr sz="1000""#).count(), 4, "{out}");
    assert!(out.contains(r#"</c:chart><c:txPr><a:bodyPr/><a:lstStyle/><a:p><a:pPr><a:defRPr sz="1000"/></a:pPr></a:p></c:txPr></c:chartSpace>"#), "{out}");
    let none = tree.set_text_style("d:1", Some("title"), &look, &host);
    assert_eq!(
        none,
        Err(TextStop::Refused(DataError("d:1: the chart shows no title".to_owned())))
    );
}
