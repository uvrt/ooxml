//! Owns: reading a shape tree (`p:spTree`, `dsp:spTree`, a group) in z-order -- shapes,
//! connectors, pictures, groups, tables, and frames positioned only -- with markup
//! compatibility's choices taken as ooxml-common's `drawingml/read_tree.py` takes them: the
//! first choice that reads as a drawable node, else the fallback, else a positioned-only
//! choice.
//!
//! Must not: follow a relationship (a chart's part, a diagram's drawing) or resolve a
//! placeholder.

use super::fill::{blip_effects, effects, fill, line, relative_rect, shape_style, svg_blip, tile};
use super::geometry::{geometry, geometry_spec, group_xfrms, text_rect, xfrm};
use super::text::text_body;
use super::{int_of, is_true, non_empty, or, owned, NS_R};
use crate::tree::{
    Connector, Frame, Group, Node, NodeInfo, Picture, Placeholder, Shape, Table, TableCell, TableRegions, TableRow,
    TextRect,
};
use ooxml_schema::{a, mc};
use ooxml_xml::view::{attr_ns, elements};
use ooxml_xml::Element;

const CHART: &str = "http://schemas.openxmlformats.org/drawingml/2006/chart";
const DIAGRAM: &str = "http://schemas.openxmlformats.org/drawingml/2006/diagram";
const OLE: &str = "http://schemas.openxmlformats.org/presentationml/2006/ole";
const CHARTEX: &str = "http://schemas.microsoft.com/office/drawing/2014/chartex";

/// A shape tree's (or a group's) members, in z-order.
pub fn shape_tree(tree: Option<&Element>) -> Vec<Node> {
    tree.map(|t| elements(t).filter_map(node).collect()).unwrap_or_default()
}

/// One member read, `None` for what is not a drawable member.
pub fn node(element: &Element) -> Option<Node> {
    match element.name().local() {
        "sp" => Some(Node::Shape(Box::new(shape(a::Shape::new(element))))),
        "pic" => Some(Node::Picture(Box::new(picture(a::Picture::new(element))))),
        "cxnSp" => Some(Node::Connector(Box::new(connector(a::Connector::new(element))))),
        "grpSp" => Some(Node::Group(Box::new(group(a::GroupShape::new(element))))),
        "graphicFrame" => Some(frame(a::GraphicFrame::new(element))),
        "AlternateContent" => alternate_content(mc::AlternateContent::new(element)),
        _ => None,
    }
}

fn alternate_content(content: mc::AlternateContent<'_>) -> Option<Node> {
    let mut positioned_only = None;
    for choice in content.choices() {
        for candidate in choice.content() {
            match node(candidate.element()) {
                Some(Node::Frame(f)) => {
                    positioned_only.get_or_insert(Node::Frame(f));
                }
                Some(found) => return Some(found),
                None => {}
            }
        }
    }
    if let Some(found) = content
        .fallback()
        .and_then(|f| f.content().find_map(|candidate| node(candidate.element())))
    {
        return Some(found);
    }
    positioned_only
}

fn info(non_visual: Option<a::NonVisual<'_>>) -> NodeInfo {
    let drawing = non_visual.and_then(a::NonVisual::drawing);
    NodeInfo {
        name: owned(drawing.and_then(a::DrawingProperties::name)),
        id: owned(drawing.and_then(a::DrawingProperties::id)),
        alt_text: non_empty(drawing.and_then(a::DrawingProperties::descr)),
        hidden: is_true(drawing.and_then(a::DrawingProperties::hidden)),
    }
}

fn hyperlink(non_visual: Option<a::NonVisual<'_>>) -> Option<String> {
    let link = non_visual.and_then(a::NonVisual::drawing)?.hyperlink()?;
    owned(link.id())
}

fn placeholder(non_visual: Option<a::NonVisual<'_>>) -> Option<Placeholder> {
    let ph = non_visual?.application()?.placeholder()?;
    Some(Placeholder {
        kind: owned(ph.kind()),
        idx: ph.idx().map(int_of),
    })
}

fn shape(sp: a::Shape<'_>) -> Shape {
    let nv = sp.non_visual();
    let properties = sp.properties();
    let element = properties.map(a::ShapeProperties::element);
    Shape {
        info: info(nv),
        placeholder: placeholder(nv),
        xfrm: xfrm(properties.and_then(a::ShapeProperties::xfrm)),
        geometry: geometry(properties),
        fill: fill(element),
        outline: line(properties.and_then(a::ShapeProperties::line)),
        effects: effects(properties.and_then(a::ShapeProperties::effects)),
        style: shape_style(sp.style()),
        text: text_body(sp.text().map(a::TextBody::element)),
        text_xfrm: xfrm(sp.text_xfrm()),
        text_rect: text_rect(properties).map(|rect| TextRect {
            geometry: geometry_spec(properties),
            rect,
        }),
        hyperlink: hyperlink(nv),
    }
}

fn connector(cxn: a::Connector<'_>) -> Connector {
    let properties = cxn.properties();
    Connector {
        info: info(cxn.non_visual()),
        xfrm: xfrm(properties.and_then(a::ShapeProperties::xfrm)),
        geometry: geometry(properties),
        outline: line(properties.and_then(a::ShapeProperties::line)),
        effects: effects(properties.and_then(a::ShapeProperties::effects)),
        style: shape_style(cxn.style()),
    }
}

fn picture(pic: a::Picture<'_>) -> Picture {
    let nv = pic.non_visual();
    let properties = pic.properties();
    let blip_fill = pic.blip_fill();
    let blip = blip_fill.and_then(a::BlipFill::blip);
    Picture {
        info: info(nv),
        blip: blip.and_then(a::Blip::embed).map(str::to_owned),
        svg: svg_blip(blip),
        placeholder: placeholder(nv),
        xfrm: xfrm(properties.and_then(a::ShapeProperties::xfrm)),
        geometry: geometry(properties),
        outline: line(properties.and_then(a::ShapeProperties::line)),
        effects: effects(properties.and_then(a::ShapeProperties::effects)),
        blip_effects: blip_effects(blip),
        src_rect: relative_rect(blip_fill.and_then(a::BlipFill::src_rect)),
        stretch: relative_rect(blip_fill.and_then(a::BlipFill::stretch).and_then(a::Stretch::fill_rect)),
        tile: tile(blip_fill.and_then(a::BlipFill::tile)),
        hyperlink: hyperlink(nv),
    }
}

fn group(grp: a::GroupShape<'_>) -> Group {
    let properties = grp.properties();
    let (outer, inner) = group_xfrms(properties.and_then(a::GroupProperties::xfrm));
    Group {
        info: info(grp.non_visual()),
        xfrm: outer,
        child_xfrm: inner,
        fill: fill(properties.map(a::GroupProperties::element)),
        effects: effects(properties.and_then(a::GroupProperties::effects)),
        children: shape_tree(Some(grp.element())),
    }
}

fn frame(frame: a::GraphicFrame<'_>) -> Node {
    let info = info(frame.non_visual());
    let placed = xfrm(frame.xfrm());
    let data = frame.graphic().and_then(a::Graphic::data);
    if let Some(tbl) = data.and_then(a::GraphicData::table) {
        return Node::Table(Box::new(table(tbl, info, placed)));
    }
    let embed = |e: Option<&Element>, name: &str| e.and_then(|e| attr_ns(e, NS_R, name)).map(str::to_owned);
    let (what, fallback) = match data.and_then(a::GraphicData::uri).unwrap_or("") {
        CHART => (
            "chart".to_owned(),
            embed(
                data.and_then(a::GraphicData::chart).map(a::ChartReference::element),
                "id",
            ),
        ),
        DIAGRAM => (
            "diagram".to_owned(),
            embed(
                data.and_then(a::GraphicData::relationship_ids)
                    .map(a::RelationshipIds::element),
                "dm",
            ),
        ),
        CHARTEX => ("chartex".to_owned(), None),
        OLE => (
            "ole".to_owned(),
            data.and_then(a::GraphicData::ole)
                .and_then(a::OleObject::picture)
                .and_then(a::Picture::blip_fill)
                .and_then(a::BlipFill::blip)
                .and_then(a::Blip::embed)
                .map(str::to_owned),
        ),
        _ => (
            data.and_then(|d| d.elements().next()).map_or_else(
                || "graphicFrame".to_owned(),
                |first| first.element().name().local().to_owned(),
            ),
            None,
        ),
    };
    Node::Frame(Frame {
        what,
        info,
        xfrm: placed,
        fallback,
    })
}

fn table(tbl: a::Table<'_>, info: NodeInfo, placed: Option<crate::source::Xfrm>) -> Table {
    let properties = tbl.properties();
    let style_id = properties
        .and_then(a::TableProperties::style_id)
        .map(|id| id.content())
        .map(|text| {
            text.trim_matches(|c: char| c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c))
                .to_owned()
        })
        .filter(|id| !id.is_empty());
    Table {
        info,
        xfrm: placed,
        columns: tbl
            .grid()
            .into_iter()
            .flat_map(a::TableGrid::columns)
            .map(|c| or(c.width(), 0.0))
            .collect(),
        rows: tbl.rows().map(row).collect(),
        regions: TableRegions {
            first_row: is_true(properties.and_then(a::TableProperties::first_row)),
            last_row: is_true(properties.and_then(a::TableProperties::last_row)),
            first_col: is_true(properties.and_then(a::TableProperties::first_col)),
            last_col: is_true(properties.and_then(a::TableProperties::last_col)),
            band_row: is_true(properties.and_then(a::TableProperties::band_row)),
            band_col: is_true(properties.and_then(a::TableProperties::band_col)),
        },
        style_id,
    }
}

fn row(tr: a::TableRow<'_>) -> TableRow {
    TableRow {
        height: or(tr.height(), 0.0),
        cells: tr.cells().map(cell).collect(),
    }
}

fn cell(tc: a::TableCell<'_>) -> TableCell {
    let properties = tc.properties();
    let span = |v: Option<f64>| v.map(int_of).filter(|v| *v != 0).unwrap_or(1);
    let anchor = match properties
        .and_then(a::TableCellProperties::anchor)
        .filter(|v| !v.is_empty())
        .unwrap_or("t")
    {
        "t" => Some("t"),
        "ctr" => Some("ctr"),
        "b" => Some("b"),
        _ => None,
    };
    TableCell {
        text: text_body(tc.text().map(a::TextBody::element)),
        fill: fill(properties.map(a::TableCellProperties::element)),
        border_top: line(properties.and_then(a::TableCellProperties::border_top)),
        border_bottom: line(properties.and_then(a::TableCellProperties::border_bottom)),
        border_left: line(properties.and_then(a::TableCellProperties::border_left)),
        border_right: line(properties.and_then(a::TableCellProperties::border_right)),
        grid_span: span(tc.grid_span()),
        row_span: span(tc.row_span()),
        h_merge: is_true(tc.h_merge()),
        v_merge: is_true(tc.v_merge()),
        margin_left: properties.and_then(a::TableCellProperties::margin_left),
        margin_right: properties.and_then(a::TableCellProperties::margin_right),
        margin_top: properties.and_then(a::TableCellProperties::margin_top),
        margin_bottom: properties.and_then(a::TableCellProperties::margin_bottom),
        anchor: anchor.map(str::to_owned),
    }
}
