//! The chart's look against what ooxml-edit's `Chart` reads of the same part (worked out
//! with it, 2026-11-20): blanks, the gap width, number formats from a point's own label, the
//! axes' scales (a category axis without bounds, a unit that is not a number), a series' and
//! its points' fills, lines, dashes and markers, and point labels with a Value From Cells field.

use super::{
    axis_names, axis_scale, blanks, gap_width, is_dot, look::series, number_formats, point_labels, series_look, Point,
    Said, Scale,
};

const CHART: &str = r##"<c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:c15="http://schemas.microsoft.com/office/drawing/2012/chart"><c:chart><c:dispBlanksAs val="span"/><c:plotArea><c:barChart><c:barDir val="col"/><c:gapWidth val=" 80 "/><c:dLbls><c:showVal val="0"/></c:dLbls><c:ser><c:idx val="0"/><c:spPr><a:solidFill><a:schemeClr val="accent1"><a:lumMod val="75000"/><a:lumOff val="12345"/><a:alpha val="x"/><a:inv/></a:schemeClr></a:solidFill><a:ln w="9525"><a:noFill/><a:prstDash val="lgDash"/></a:ln></c:spPr><c:dPt><c:idx val="1"/><c:spPr><a:gradFill/><a:ln w="x"><a:solidFill><a:sysClr val="windowText" lastClr="000000"/></a:solidFill><a:prstDash val="sysDashDot"/></a:ln></c:spPr></c:dPt><c:dPt><c:idx val="2"/><c:spPr><a:noFill/></c:spPr></c:dPt><c:dLbls><c:dLbl><c:idx val="1"/><c:tx><c:rich><a:bodyPr/><a:p><a:r><a:t>Big, </a:t></a:r></a:p></c:rich></c:tx><c:showVal val="1"/></c:dLbl><c:dLbl><c:idx val="0"/><c:numFmt formatCode="0.0%" sourceLinked="0"/><c:tx><c:rich><a:p><a:r><a:t>A</a:t></a:r><a:fld type="CELLRANGE"><a:t>[CELLRANGE]</a:t></a:fld><a:r><a:t>B</a:t></a:r></a:p></c:rich></c:tx><c:showVal val="1"/></c:dLbl><c:showVal val="0"/></c:dLbls><c:tx><c:strRef><c:f>Sheet1!$B$1</c:f><c:strCache><c:ptCount val="1"/><c:pt idx="0"><c:v>North</c:v></c:pt></c:strCache></c:strRef></c:tx><c:cat><c:strRef><c:f>Sheet1!$A$2:$A$4</c:f><c:strCache><c:ptCount val="3"/><c:pt idx="0"><c:v>Q1</c:v></c:pt><c:pt idx="1"><c:v>Q2</c:v></c:pt><c:pt idx="2"><c:v>Q3</c:v></c:pt></c:strCache></c:strRef></c:cat><c:val><c:numRef><c:f>Sheet1!$B$2:$B$4</c:f><c:numCache><c:formatCode>General</c:formatCode><c:ptCount val="3"/><c:pt idx="0"><c:v>1</c:v></c:pt><c:pt idx="2"><c:v>2.5</c:v></c:pt></c:numCache></c:numRef></c:val><c:extLst><c:ext uri="{02D57815-91ED-43cb-92C2-25804820EDAC}"><c15:datalabelsRange><c15:f>Sheet1!$C$2:$C$4</c15:f><c15:dlblRangeCache><c:ptCount val="3"/><c:pt idx="0"><c:v>cell0</c:v></c:pt></c15:dlblRangeCache></c15:datalabelsRange></c:ext></c:extLst></c:ser><c:axId val="10"/><c:axId val="20"/></c:barChart><c:lineChart><c:marker val="1"/><c:ser><c:idx val="1"/><c:marker><c:symbol val="diamond"/><c:size val="7"/><c:spPr><a:solidFill><a:srgbClr val="1F4E79"/></a:solidFill><a:ln><a:solidFill><a:prstClr val="red"/></a:solidFill></a:ln></c:spPr></c:marker><c:tx><c:v>Line</c:v></c:tx><c:val><c:numLit><c:ptCount val="3"/><c:pt idx="1"><c:v>4</c:v></c:pt></c:numLit></c:val></c:ser><c:axId val="10"/><c:axId val="20"/></c:lineChart><c:catAx><c:axId val="10"/><c:scaling><c:orientation val="maxMin"/></c:scaling><c:axPos val="l"/><c:majorGridlines/></c:catAx><c:valAx><c:axId val="20"/><c:scaling><c:orientation val="minMax"/><c:max val="10"/><c:min val="-2.50"/></c:scaling><c:axPos val="b"/><c:minorGridlines/><c:numFmt formatCode="#,##0" sourceLinked="0"/><c:majorUnit val="2"/><c:minorUnit val="junk"/></c:valAx></c:plotArea></c:chart></c:chartSpace>"##;

fn text(t: &str) -> Said {
    Said::Text(t.to_owned())
}

#[test]
fn a_charts_look_reads_as_ooxml_edit_reads_it() {
    let tree = ooxml_xml::parse(CHART.as_bytes(), &ooxml_xml::Limits::default()).unwrap_or_else(|e| panic!("{e}"));
    let root = tree.root();
    assert_eq!(blanks(root), "span");
    assert_eq!(gap_width(root), Some(80));
    assert!(!is_dot(root));
    let formats = number_formats(root, Some("General"));
    assert_eq!(formats.data_labels.as_deref(), Some("0.0%"));
    assert_eq!(formats.value_axis.as_deref(), Some("#,##0"));
    assert_eq!(axis_names(root), ["category", "value"]);
    assert_eq!(
        axis_scale(root, "category"),
        Some(Scale {
            bounds: None,
            number_format: None,
            reverse: true,
            gridlines: (true, false)
        })
    );
    assert_eq!(
        axis_scale(root, "value"),
        Some(Scale {
            bounds: Some([
                Some(Point::Float(-2.5)),
                Some(Point::Int(10)),
                Some(Point::Int(2)),
                None
            ]),
            number_format: Some("#,##0".into()),
            reverse: false,
            gridlines: (false, true),
        })
    );
    let all = series(root);
    let north = series_look(all[0].1);
    assert_eq!(
        north.own,
        [
            ("fill", text("accent1 lumMod=75% lumOff=12.345% alpha=x inv")),
            ("line", Said::Null),
            ("line_width", Said::Number(Point::Float(0.75))),
            ("dash", text("long_dash")),
        ]
    );
    assert_eq!(
        north.points,
        [
            (1, vec![("line", text("000000")), ("dash", text("sysDashDot"))]),
            (2, vec![("fill", Said::Null)])
        ]
    );
    let line = series_look(all[1].1);
    assert_eq!(
        line.own,
        [
            ("marker", text("diamond")),
            ("marker_size", Said::Number(Point::Int(7))),
            ("marker_fill", text("1F4E79")),
            ("marker_line", text("red")),
        ]
    );
    assert_eq!(
        point_labels(all[0].1),
        Ok(vec![Some("Acell0".into()), Some("Big".into()), None])
    );
    assert_eq!(point_labels(all[1].1), Ok(vec![None, None, None]));
}
