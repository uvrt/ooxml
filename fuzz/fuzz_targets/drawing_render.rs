//! The DrawingML painter: no drawable panics it. An input that is XML is read as a chart
//! part and laid out (as `chart_layout` does), and its drawables painted; any other input is
//! read as a little program that builds drawables -- every preset name, custom paths of any
//! text, fills, outlines with arrowheads and dashes, effects, pictures of any bytes, tables,
//! nested and stretched groups, text with bullets, tabs and East Asian runs, and transforms
//! with extreme, infinite or missing values -- and paints them. Both under PowerPoint's
//! rules and Word's, written as SVG.
#![no_main]

use libfuzzer_sys::fuzz_target;
use ooxml_chart::layout::{chart_text_body, drawable_plots, lay_out, ChartInput, ChartStyle, Resolve};
use ooxml_colour::ColourContext;
use ooxml_drawingml::drawable::{
    ArrowEnd, BlipEffects, BodyProperties, Bullet, CellBorders, Colour, Connector, CustomPath, Effects, Element, Fill,
    Geometry, Glow, GradientFill, GradientStop, Group, Image, ImageTile, Insets, OuterShadow, Outline, Paragraph,
    ParagraphProperties, RunProperties, Shape, Table, TableCell, TableRow, TextBody, TextRun, Transform,
};
use ooxml_drawingml::source;
use ooxml_drawingml::text::{Spacing, TabStop};
use ooxml_render::{render_element, table_measurer, Painter, POWERPOINT, WORD};
use ooxml_xml::{parse, Limits};

struct Unthemed(ColourContext);

impl Resolve for Unthemed {
    fn fill(&self, fill: Option<&source::Fill>) -> Option<Fill> {
        match fill? {
            source::Fill::None => Some(Fill::None),
            source::Fill::Solid(c) => self.0.resolve(c).ok().flatten().map(|c| Fill::Solid(Colour::from(c))),
            _ => None,
        }
    }

    fn outline(&self, outline: Option<&source::Outline>) -> Option<Outline> {
        let outline = outline?;
        let fill = outline.fill.as_ref().and_then(|f| self.fill(Some(f)));
        if matches!(fill, Some(Fill::None)) {
            return None;
        }
        Some(Outline { fill, ..Outline::solid(outline.width.unwrap_or(9525.0), Colour::opaque("#000000")) })
    }

    fn text(&self, _rich: Option<&ooxml_drawingml::text::TextBody>, text: &str, size: f64, align: &str) -> TextBody {
        let mut run = TextRun { text: text.to_owned(), ..TextRun::default() };
        run.properties.font_size = Some(size);
        TextBody {
            paragraphs: vec![Paragraph { runs: vec![run], properties: ParagraphProperties::aligned(align), end_properties: None }],
            properties: chart_text_body(),
        }
    }

    fn typeface(&self, face: &str) -> Option<String> {
        Some(face.to_owned())
    }
}

/// The input read a byte at a time; past its end, zeros.
struct Bytes<'a>(&'a [u8]);

impl Bytes<'_> {
    fn byte(&mut self) -> u8 {
        let Some((first, rest)) = self.0.split_first() else { return 0 };
        self.0 = rest;
        *first
    }

    fn pick<'s>(&mut self, options: &[&'s str]) -> &'s str {
        options.get(usize::from(self.byte()) % options.len().max(1)).copied().unwrap_or("")
    }

    fn number(&mut self) -> f64 {
        match self.byte() % 8 {
            0 => f64::NAN,
            1 => f64::INFINITY,
            2 => -1e12,
            3 => 0.0,
            _ => f64::from(i16::from_le_bytes([self.byte(), self.byte()])) * 9525.0,
        }
    }

    fn text(&mut self) -> String {
        let n = usize::from(self.byte() % 24);
        (0..n)
            .map(|_| match self.byte() % 10 {
                0 => '\t',
                1 => '\n',
                2 => ' ',
                3 => '日',
                4 => '、',
                5 => '（',
                6 => '\u{200b}',
                _ => char::from(b'A' + self.byte() % 26),
            })
            .collect()
    }

    fn colour(&mut self) -> Colour {
        Colour { hex: format!("#{:02x}{:02x}{:02x}", self.byte(), self.byte(), self.byte()), alpha: f64::from(self.byte()) / 255.0 }
    }
}

const PRESETS: [&str; 12] =
    ["rect", "ellipse", "roundRect", "wedgeEllipseCallout", "leftBrace", "cloud", "arc", "star5", "line", "chartX", "noSuch", "bendUpArrow"];
const FACES: [&str; 6] = ["Aptos", "Calibri", "MS Gothic", "Arial", "+mn-lt", ""];

fn transform(b: &mut Bytes<'_>) -> Transform {
    Transform {
        offset_x: b.number(),
        offset_y: b.number(),
        extent_width: b.number(),
        extent_height: b.number(),
        rotation: f64::from(b.byte()) * 3.0 - 90.0,
        flip_h: b.byte() % 2 == 0,
        flip_v: b.byte() % 3 == 0,
    }
}

fn fill(b: &mut Bytes<'_>) -> Option<Fill> {
    Some(match b.byte() % 6 {
        0 => return None,
        1 => Fill::None,
        2 => Fill::Solid(b.colour()),
        3 => Fill::Gradient(GradientFill {
            stops: (0..b.byte() % 4).map(|_| GradientStop { position: b.number() / 9525.0, colour: b.colour() }).collect(),
            angle: b.number(),
            gradient_type: b.pick(&["linear", "radial"]).into(),
            center_x: Some(b.number()),
            center_y: None,
            path: Some(b.pick(&["circle", "rect", "shape"]).into()),
            focus: Some([b.number(), 0.2, 0.3, 1.5]),
            scaled: Some(b.byte() % 2 == 0),
            rotate_with_shape: Some(false),
        }),
        4 => Fill::Pattern { preset: b.pick(&["pct5", "horz", "nope", "trellis"]).into(), foreground: b.colour(), background: b.colour() },
        _ => Fill::Image {
            data: b.pick(&["iVBORw0KGgo=", "", "!!", "R0lGODlhAQABAAAAACw="]).into(),
            mime: "image/png".into(),
            tile: Some(ImageTile { tx: b.number(), ty: 0.0, sx: b.number(), sy: 0.5, flip: b.pick(&["x", "xy", "none"]).into(), align: b.pick(&["ctr", "br", "tl"]).into() }),
        },
    })
}

fn outline(b: &mut Bytes<'_>) -> Option<Outline> {
    if b.byte() % 3 == 0 {
        return None;
    }
    let end = |b: &mut Bytes<'_>| {
        Some(ArrowEnd { kind: b.pick(&["triangle", "stealth", "oval", "arrow", "diamond", "none"]).into(), width: b.pick(&["sm", "med", "lg", "x"]).into(), length: "lg".into() })
    };
    Some(Outline {
        width: b.number(),
        fill: fill(b),
        dash_style: b.pick(&["solid", "dash", "sysDashDot", "bogus"]).into(),
        custom_dash: (b.byte() % 4 == 0).then(|| vec![b.number(), 3.0]),
        line_cap: Some(b.pick(&["round", "square", ""]).into()),
        line_join: None,
        head_end: end(b),
        tail_end: end(b),
        compound: None,
    })
}

fn body(b: &mut Bytes<'_>) -> TextBody {
    let paragraphs = (0..b.byte() % 4)
        .map(|_| {
            let runs = (0..b.byte() % 4)
                .map(|_| TextRun {
                    text: b.text(),
                    properties: RunProperties {
                        font_size: (b.byte() % 3 != 0).then(|| b.number() / 9525.0),
                        font_family: Some(b.pick(&FACES).into()),
                        font_family_ea: Some(b.pick(&FACES).into()),
                        bold: b.byte() % 2 == 0,
                        italic: b.byte() % 2 == 0,
                        underline: b.byte() % 2 == 0,
                        baseline: b.number(),
                        highlight: (b.byte() % 4 == 0).then(|| b.colour()),
                        ..RunProperties::default()
                    },
                })
                .collect();
            let bullet = match b.byte() % 5 {
                0 => Some(Bullet::Char(b.text())),
                1 => Some(Bullet::AutoNumber { scheme: b.pick(&["romanLcPeriod", "alphaUcParenR", "x"]).into(), start: i64::from(b.byte()) - 100 }),
                2 => Some(Bullet::Picture { data: "AA==".into(), mime: "image/png".into() }),
                3 => Some(Bullet::None),
                _ => None,
            };
            Paragraph {
                runs,
                properties: ParagraphProperties {
                    alignment: Some(b.pick(&["l", "ctr", "r", "just"]).into()),
                    line_spacing: Some(if b.byte() % 2 == 0 { Spacing::Points(b.number()) } else { Spacing::Percent(b.number()) }),
                    bullet,
                    margin_left: Some(b.number()),
                    indent: Some(b.number()),
                    tab_stops: vec![TabStop { position: b.number(), align: b.pick(&["l", "ctr", "dec"]).into() }],
                    ..ParagraphProperties::default()
                },
                end_properties: None,
            }
        })
        .collect();
    TextBody {
        paragraphs,
        properties: BodyProperties {
            anchor: b.pick(&["t", "ctr", "b"]).into(),
            wrap: b.pick(&["square", "none"]).into(),
            auto_fit: b.pick(&["noAutofit", "normAutofit", "spAutofit"]).into(),
            font_scale: b.number() / 9525.0,
            line_reduction: f64::from(b.byte()) / 255.0,
            columns: i64::from(b.byte() % 4),
            vert: b.pick(&["horz", "vert", "vert270", "eaVert"]).into(),
            rotation: b.number(),
            default_tab_size: b.number(),
            ..BodyProperties::default()
        },
    }
}

fn effects(b: &mut Bytes<'_>) -> Option<Effects> {
    (b.byte() % 3 == 0).then(|| Effects {
        outer_shadow: Some(OuterShadow { blur_radius: b.number(), distance: b.number(), direction: b.number(), colour: b.colour(), alignment: "b".into(), rotate_with_shape: true }),
        inner_shadow: None,
        glow: Some(Glow { radius: b.number(), colour: b.colour() }),
        soft_edge: Some(b.number()),
    })
}

fn geometry(b: &mut Bytes<'_>) -> Geometry {
    if b.byte() % 4 == 0 {
        let commands = b.pick(&["M 0 0 L 10 10 Z", "m1e5 2 c 1 2 3 4 5 6 a 1 1 0 1 1 2 2 h -3 v .5", "", "1 2 L"]).into();
        return Geometry::Custom(vec![CustomPath { width: b.number(), height: b.number(), commands }]);
    }
    Geometry::Preset { name: b.pick(&PRESETS).into(), adjust: vec![("adj".into(), b.number()), ("adj1".into(), b.number())] }
}

fn element(b: &mut Bytes<'_>, depth: u8) -> Element {
    match b.byte() % 5 {
        0 => Element::Connector(Box::new(Connector { transform: transform(b), geometry: geometry(b), outline: outline(b), effects: effects(b), ..Connector::default() })),
        1 => Element::Image(Box::new(Image {
            transform: transform(b),
            data: b.pick(&["iVBORw0KGgo=", "/9j/4AAQ", "Qk0=", "x"]).into(),
            mime: "image/png".into(),
            effects: effects(b),
            blip_effects: Some(BlipEffects { grayscale: true, bi_level: Some(b.number()), lum: Some((b.number(), 0.1)), duotone: Some((b.colour(), b.colour())), clr_change: Some((b.colour(), b.colour())), alpha: Some(0.5), blur: Some((b.number(), true)) }),
            src_rect: Some(Insets { left: b.number(), top: 0.1, right: 2.0, bottom: -1.0 }),
            stretch: Some(Insets { left: -0.5, ..Insets::default() }),
            tile: None,
            geometry: Some(geometry(b)),
            outline: outline(b),
            ..Image::default()
        })),
        2 if depth < 4 => Element::Group(Box::new(Group {
            transform: transform(b),
            child_transform: transform(b),
            children: (0..b.byte() % 4).map(|_| element(b, depth + 1)).collect(),
            ..Group::default()
        })),
        3 => Element::Table(Box::new(Table {
            transform: transform(b),
            rows: (0..b.byte() % 3)
                .map(|_| TableRow {
                    height: b.number(),
                    cells: (0..b.byte() % 3)
                        .map(|_| TableCell { text: Some(body(b)), fill: fill(b), borders: Some(CellBorders { top: outline(b), ..CellBorders::default() }), grid_span: i64::from(b.byte()) - 128, row_span: 3, ..TableCell::default() })
                        .collect(),
                })
                .collect(),
            columns: (0..b.byte() % 3).map(|_| b.number()).collect(),
            background: fill(b),
            effects: effects(b),
            ..Table::default()
        })),
        _ => Element::Shape(Box::new(Shape { transform: transform(b), geometry: geometry(b), fill: fill(b), outline: outline(b), text: Some(body(b)), effects: effects(b), ..Shape::default() })),
    }
}

fn paint(elements: &[Element]) {
    for rules in [POWERPOINT, WORD] {
        let measurer = table_measurer(&rules);
        let mut painter = Painter::new(rules, &measurer);
        for e in elements {
            let _ = render_element(e, &mut painter).to_svg();
        }
        for def in &painter.defs {
            let _ = def.to_svg();
        }
    }
}

fn chart(data: &[u8]) -> Vec<Element> {
    let Ok(tree) = parse(data, &Limits::default()) else { return Vec::new() };
    let Some(chart) = ooxml_chart::chart_space(tree.root()) else { return Vec::new() };
    let plots = drawable_plots(&chart);
    let Some(&plot) = plots.first() else { return Vec::new() };
    let resolve = Unthemed(ColourContext::unthemed());
    let style = ChartStyle {
        font_family: Some("Aptos".into()),
        font_size: 10.0,
        colour: Colour::opaque("#000000"),
        accents: vec![Colour::opaque("#4472c4")],
        font_family_ea: Some("MS Gothic".into()),
        office_2007: false,
        line_colour: None,
    };
    let input = ChartInput {
        chart: &chart,
        plot,
        plots: plots.clone(),
        width_pt: 480.0,
        height_pt: 288.0,
        style,
        resolve: &resolve,
        rules: &ooxml_chart::rules::POWERPOINT,
        locale: &ooxml_numfmt::EN_US,
        default_title: None,
    };
    lay_out(&input).elements
}

fuzz_target!(|data: &[u8]| {
    let elements = if data.first() == Some(&b'<') {
        chart(data)
    } else {
        let mut bytes = Bytes(data);
        (0..1 + bytes.byte() % 4).map(|_| element(&mut bytes, 0)).collect()
    };
    paint(&elements);
});
