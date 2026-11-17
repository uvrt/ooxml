//! The generated views read by local name, and their constructors insert in schema order.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use ooxml_schema::{a, c, mc, rels};
use ooxml_xml::{parse, Limits, Tree};

fn tree(xml: &str) -> Tree {
    parse(xml.as_bytes(), &Limits::default()).unwrap()
}

#[test]
fn a_shape_s_properties_read_by_local_name() {
    let t = tree(
        r#"<p:spPr xmlns:p="urn:p" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main">
             <a:xfrm rot="5400000" flipH="1"><a:off x="10" y="20"/><a:ext cx="1_000" cy="2e3"/></a:xfrm>
             <a:prstGeom prst="roundRect"><a:avLst><a:gd name="adj" fmla="val 1"/></a:avLst></a:prstGeom>
             <a:solidFill><a:schemeClr val="accent1"><a:lumMod val="75000"/></a:schemeClr></a:solidFill>
             <a:ln w="12700"><a:prstDash val="dash"/><a:round/></a:ln>
           </p:spPr>"#,
    );
    let sp_pr = a::ShapeProperties::of(t.root()).unwrap();
    let xfrm = sp_pr.xfrm().unwrap();
    assert_eq!(xfrm.rotation(), Some(5_400_000.0));
    assert_eq!(xfrm.flip_h(), Some("1"));
    assert_eq!(xfrm.offset().and_then(a::Point::y), Some(20.0));
    assert_eq!(xfrm.extent().and_then(a::Size::cx), Some(1000.0));
    assert_eq!(xfrm.extent().and_then(a::Size::cy), Some(2000.0));
    let preset = sp_pr.preset().unwrap();
    assert_eq!(preset.preset(), Some("roundRect"));
    let guides: Vec<_> = preset.adjustments().unwrap().guides().collect();
    assert_eq!(guides[0].formula(), Some("val 1"));
    let fill = a::Fills::new(sp_pr.element()).solid().unwrap();
    let colour = fill.elements().next().unwrap();
    assert_eq!(
        a::SchemeColour::of(colour.element()).and_then(a::SchemeColour::val),
        Some("accent1")
    );
    let line = sp_pr.line().unwrap();
    assert_eq!(line.width(), Some(12700.0));
    assert_eq!(line.preset_dash().and_then(a::Val::val), Some("dash"));
    assert!(line.round().is_some() && line.miter().is_none());
}

#[test]
fn a_constructor_makes_the_element_and_insert_keeps_the_schema_order() {
    let ln = a::Ln::create();
    assert_eq!(ln.name().qualified(), "a:ln");
    assert_eq!(ln.name().namespace(), Some(a::NS_A));
    let head = tree(r#"<a:headEnd xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" type="arrow"/>"#);
    let fill = a::SolidFill::create();
    let ln = a::Ln::new(&ln).insert((**head.root()).clone());
    let ln = a::Ln::new(&ln).insert(fill);
    let ln = a::Ln::new(&ln).insert(tree("<x:extLst xmlns:x=\"urn:x\"/>").root().as_ref().clone());
    let names: Vec<_> = ln.child_elements().map(|e| e.name().local().to_string()).collect();
    assert_eq!(names, ["solidFill", "headEnd", "extLst"]);
}

#[test]
fn chart_relationships_and_alternate_content_read() {
    let chart = tree(
        r#"<c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart">
             <c:chart><c:plotArea><c:barChart><c:barDir val="col"/><c:ser><c:idx val="2"/></c:ser>
             <c:axId val="7"/></c:barChart></c:plotArea></c:chart></c:chartSpace>"#,
    );
    let space = c::ChartSpace::of(chart.root()).unwrap();
    let plot = space
        .chart()
        .and_then(c::Chart::plot_area)
        .unwrap()
        .members()
        .next()
        .unwrap();
    let plot = c::Plot::new(plot.element());
    assert_eq!(plot.bar_direction().and_then(c::Val::val), Some("col"));
    assert_eq!(
        plot.series().next().and_then(c::Series::idx).and_then(c::Val::num),
        Some(2.0)
    );
    assert_eq!(plot.axis_ids().map(|v| v.val().unwrap()).collect::<Vec<_>>(), ["7"]);

    let parts = tree(
        r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
             <Relationship Id="rId1" Type="t" Target="slides/slide1.xml"/></Relationships>"#,
    );
    let first = rels::Relationships::of(parts.root()).unwrap().items().next().unwrap();
    assert_eq!((first.id(), first.target()), (Some("rId1"), Some("slides/slide1.xml")));

    let choice = tree(
        r#"<mc:AlternateContent xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006">
             <mc:Choice Requires="a14"><x/></mc:Choice><mc:Fallback><y/></mc:Fallback></mc:AlternateContent>"#,
    );
    let content = mc::AlternateContent::of(choice.root()).unwrap();
    assert_eq!(content.choices().next().and_then(mc::Choice::requires), Some("a14"));
    assert_eq!(content.fallback().unwrap().content().count(), 1);
}
