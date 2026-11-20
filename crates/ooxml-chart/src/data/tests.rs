//! The data view against what ooxml-edit's `chart_model` and docx-agent's `chart_summary`
//! give for the same part (worked out with them, 2026-11-20): rich title lines and breaks, a
//! numeric series name, a scatter's x axis found by its position, a category code inside
//! `mc:AlternateContent`, dates kept as serials, a blank, an index that is not a number and a
//! value that is not one, an integer and a float as written.

use super::{chart_data, Point, SeriesData};

const CHART: &str = r#"<c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006"><c:chart><c:title><c:tx><c:rich><a:bodyPr/><a:p><a:r><a:t>Sales  by</a:t></a:r><a:br/><a:r><a:t>month</a:t></a:r></a:p><a:p><a:fld id="x" type="y"><a:t>2026</a:t></a:fld></a:p></c:rich></c:tx></c:title><c:plotArea><c:scatterChart><c:ser><c:idx val="0"/><c:order val="0"/><c:tx><c:numRef><c:f>S!$B$1</c:f><c:numCache><c:formatCode>General</c:formatCode><c:ptCount val="1"/><c:pt idx="0"><c:v>2025</c:v></c:pt></c:numCache></c:numRef></c:tx><c:xVal><c:numRef><c:f>S!$A$2:$A$4</c:f><c:numCache><mc:AlternateContent><mc:Choice Requires="c16r3"><c16r3:formatcode2 xmlns:c16r3="http://schemas.microsoft.com/office/drawing/2017/03/chart">[$-en-BE]d mmm</c16r3:formatcode2></mc:Choice><mc:Fallback><c:formatCode>[$]d mmm</c:formatCode></mc:Fallback></mc:AlternateContent><c:ptCount val="3"/><c:pt idx="0"><c:v>46027</c:v></c:pt><c:pt idx="2"><c:v>46027.5</c:v></c:pt></c:numCache></c:numRef></c:xVal><c:yVal><c:numRef><c:f>S!$B$2:$B$4</c:f><c:numCache><c:formatCode>0.0</c:formatCode><c:pt idx="0"><c:v>3</c:v></c:pt><c:pt idx="1"><c:v>11.70</c:v></c:pt><c:pt idx="x"><c:v>9</c:v></c:pt><c:pt idx="3"><c:v>junk</c:v></c:pt></c:numCache></c:numRef></c:yVal></c:ser></c:scatterChart><c:valAx><c:axId val="1"/><c:axPos val="l"/><c:title><c:tx><c:strRef><c:f>S!$C$1</c:f><c:strCache><c:ptCount val="1"/><c:pt idx="0"><c:v> Amount </c:v></c:pt></c:strCache></c:strRef></c:tx></c:title></c:valAx><c:valAx><c:axId val="2"/><c:axPos val="b"/></c:valAx></c:plotArea><c:legend/></c:chart></c:chartSpace>"#;

#[test]
fn a_chart_reads_as_ooxml_edit_reads_it() {
    let tree = ooxml_xml::parse(CHART.as_bytes(), &ooxml_xml::Limits::default()).unwrap_or_else(|e| panic!("{e}"));
    let data = chart_data(tree.root()).unwrap_or_else(|_| panic!("unreadable"));
    assert_eq!(data.types, ["scatter"]);
    assert_eq!(data.title, Some(Point::Text("Sales  by\u{b}month\n2026".into())));
    assert_eq!(
        data.axis_titles,
        [("category", None), ("value", Some(Point::Text(" Amount ".into())))]
    );
    assert!(data.legend);
    assert_eq!(data.format.as_deref(), Some("0.0"));
    assert_eq!(
        data.categories,
        [Some(Point::Int(46027)), None, Some(Point::Float(46027.5))]
    );
    assert_eq!(
        data.series,
        [SeriesData {
            name: Some(Point::Int(2025)),
            values: vec![Some(Point::Int(3)), Some(Point::Float(11.7)), None, None],
        }]
    );
    assert_eq!(
        data.category_dates,
        Some(vec![
            Some("2026-01-05".into()),
            None,
            Some("2026-01-05T12:00:00".into())
        ])
    );
    assert_eq!(data.category_format.as_deref(), Some("[$-en-BE]d mmm"));
    assert_eq!(
        data.summary(),
        "scatter; title: Sales by month 2026; series: 2025; categories: 2026-01-05, , 2026-01-05T12:00:00"
    );
}

#[test]
fn a_count_past_the_bound_is_unreadable() {
    let xml = CHART.replace(r#"<c:ptCount val="3"/>"#, r#"<c:ptCount val="99999999999"/>"#);
    let tree = ooxml_xml::parse(xml.as_bytes(), &ooxml_xml::Limits::default()).unwrap_or_else(|e| panic!("{e}"));
    assert!(chart_data(tree.root()).is_err());
}
