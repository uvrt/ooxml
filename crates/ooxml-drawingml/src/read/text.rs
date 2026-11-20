//! Owns: reading a text body (`p:txBody`, `a:txBody`, `c:rich`, `c:txPr`) -- body
//! properties, list styles, paragraphs and their runs in document order -- as
//! ooxml-common's `drawingml/read_text.py` reads it; and a paragraph's plain text as
//! ooxml-edit's `dmltext.paragraph_text` reads it, for an editor's view of a chart title or
//! a diagram node.
//!
//! Must not: fill a run's properties in from its paragraph, list style or theme (the
//! consumer's inheritance).

use super::colour::colour;
use super::{decode_char_refs, int_of, is_true, non_empty, or, or_str, owned, NS_R};
use crate::text::{
    Autofit, BodyProperties, Bullet, ListStyle, Paragraph, ParagraphProperties, RunProperties, Spacing, TabStop,
    TextBody, TextRun,
};
use ooxml_colour::SourceColour;
use ooxml_schema::a;
use ooxml_xml::view::attr_ns;
use ooxml_xml::Element;

const AUTO_NUMBER_SCHEMES: [&str; 9] = [
    "arabicPeriod",
    "arabicParenR",
    "romanUcPeriod",
    "romanLcPeriod",
    "alphaUcPeriod",
    "alphaLcPeriod",
    "alphaLcParenR",
    "alphaUcParenR",
    "arabicPlain",
];
const VERTICAL: [&str; 6] = ["horz", "vert", "vert270", "eaVert", "wordArtVert", "mongolianVert"];

/// A text body, `None` where there is none.
pub fn text_body(body: Option<&Element>) -> Option<TextBody> {
    let body = a::TextBody::new(body?);
    Some(TextBody {
        paragraphs: body.paragraphs().map(paragraph).collect(),
        properties: body_properties(body.body()),
        list_style: list_style(body.list_style().map(a::ListStyle::element)),
    })
}

/// `a:bodyPr`, `None` where it states nothing read here.
pub fn body_properties(body: Option<a::BodyProperties<'_>>) -> Option<BodyProperties> {
    let body = body?;
    let (autofit, font_scale, line_reduction) = if let Some(norm) = body.norm_autofit() {
        (
            Some(Autofit::Normal),
            Some(or(norm.font_scale(), 100_000.0) / 100_000.0),
            Some(or(norm.line_reduction(), 0.0) / 100_000.0),
        )
    } else if body.shape_autofit().is_some() {
        (Some(Autofit::Shape), Some(1.0), Some(0.0))
    } else if body.no_autofit().is_some() {
        (Some(Autofit::None), Some(1.0), Some(0.0))
    } else {
        (None, None, None)
    };
    let anchor = match body.anchor().unwrap_or("") {
        "ctr" => Some("ctr"),
        "b" => Some("b"),
        "t" | "just" | "dist" => Some("t"),
        _ => None,
    };
    let properties = BodyProperties {
        inset_left: body.inset_left(),
        inset_right: body.inset_right(),
        inset_top: body.inset_top(),
        inset_bottom: body.inset_bottom(),
        anchor: anchor.map(str::to_owned),
        wrap: body
            .wrap()
            .filter(|w| matches!(*w, "square" | "none"))
            .map(str::to_owned),
        autofit,
        font_scale,
        line_reduction,
        columns: body.columns().map(|c| int_of(c).max(1)),
        vert: body.vert().filter(|v| VERTICAL.contains(v)).map(str::to_owned),
        rotation: body.rotation(),
        tab_size: body.tab_size(),
    };
    (properties != BodyProperties::default()).then_some(properties)
}

/// `a:lstStyle` and its kin, `None` where it states no level.
pub fn list_style(style: Option<&Element>) -> Option<ListStyle> {
    let style = a::ListStyle::new(style?);
    let levels = [
        style.level1(),
        style.level2(),
        style.level3(),
        style.level4(),
        style.level5(),
        style.level6(),
        style.level7(),
        style.level8(),
        style.level9(),
    ]
    .map(paragraph_properties);
    let default = paragraph_properties(style.default());
    (default.is_some() || levels.iter().any(Option::is_some)).then(|| ListStyle {
        default,
        levels: Box::new(levels),
    })
}

fn paragraph(p: a::Paragraph<'_>) -> Paragraph {
    Paragraph {
        runs: p.items().filter_map(run).collect(),
        properties: paragraph_properties(p.properties()),
        end: run_properties(p.end()),
    }
}

fn run(item: a::TextRun<'_>) -> Option<TextRun> {
    let properties = run_properties(item.properties());
    let text = || {
        item.text()
            .map(|t| decode_char_refs(&t.element().text_content()))
            .unwrap_or_default()
    };
    match item.element().name().local() {
        "r" => Some(TextRun {
            text: text(),
            properties,
            field_type: None,
        }),
        "fld" => Some(TextRun {
            text: text(),
            properties,
            field_type: owned(item.field_type()),
        }),
        "br" => Some(TextRun {
            text: "\n".into(),
            properties,
            field_type: None,
        }),
        _ => None,
    }
}

fn spacing(node: Option<a::Spacing<'_>>) -> Option<Spacing> {
    let node = node?;
    if let Some(points) = node.points().and_then(a::Val::num) {
        return Some(Spacing::Points(points));
    }
    node.percent().and_then(a::Val::num).map(Spacing::Percent)
}

fn bullet(p: a::ParagraphProperties<'_>) -> Option<Bullet> {
    if p.bullet_none().is_some() {
        return Some(Bullet::None);
    }
    if let Some(embed) = p
        .bullet_blip()
        .and_then(a::BulletBlip::blip)
        .and_then(|b| attr_ns(b.element(), NS_R, "embed"))
    {
        return Some(Bullet::Picture(embed.to_owned()));
    }
    if let Some(c) = p.bullet_char() {
        return Some(Bullet::Char(decode_char_refs(&or_str(c.char(), "\u{2022}"))));
    }
    let auto = p.bullet_auto()?;
    let scheme = or_str(auto.scheme(), "arabicPeriod");
    Some(Bullet::AutoNumber {
        scheme: if AUTO_NUMBER_SCHEMES.contains(&scheme.as_str()) {
            scheme
        } else {
            "arabicPeriod".into()
        },
        start: auto.start().map(int_of).filter(|v| *v != 0).unwrap_or(1),
    })
}

/// `a:pPr` and the list style's levels, `None` where nothing read here is stated.
pub fn paragraph_properties(p: Option<a::ParagraphProperties<'_>>) -> Option<ParagraphProperties> {
    let p = p?;
    let align = match p.align().unwrap_or("") {
        "l" => Some("l"),
        "ctr" => Some("ctr"),
        "r" => Some("r"),
        "just" | "justLow" | "dist" => Some("just"),
        _ => None,
    };
    let tabs: Vec<TabStop> = p
        .tabs()
        .into_iter()
        .flat_map(a::TabList::tabs)
        .map(|t| TabStop {
            position: or(t.position(), 0.0),
            align: or_str(t.align(), "l"),
        })
        .collect();
    let properties = ParagraphProperties {
        align: align.map(str::to_owned),
        level: p.level().map(int_of),
        line_spacing: spacing(p.line_spacing()),
        space_before: spacing(p.space_before()),
        space_after: spacing(p.space_after()),
        margin_left: p.margin_left(),
        indent: p.indent(),
        bullet: bullet(p),
        bullet_font: owned(p.bullet_font().and_then(a::TextFont::typeface)),
        bullet_colour: colour(p.bullet_colour().map(a::Any::element)),
        bullet_size_percent: p.bullet_size_percent().and_then(a::Val::num),
        bullet_size_points: p.bullet_size_points().and_then(a::Val::num).map(|v| v / 100.0),
        tabs: (!tabs.is_empty()).then_some(tabs),
        default_run: run_properties(p.default_run()),
    };
    (properties != ParagraphProperties::default()).then_some(properties)
}

/// `a:rPr` and its kin, `None` where nothing read here is stated.
pub fn run_properties(r: Option<a::RunProperties<'_>>) -> Option<RunProperties> {
    let r = r?;
    let hyperlink = r.hyperlink();
    let linked = hyperlink.is_some();
    let underline = r.underline();
    let face = |f: Option<a::TextFont<'_>>| owned(f.and_then(a::TextFont::typeface));
    let line = r.line();
    let properties = RunProperties {
        bold: r.bold().map(|v| is_true(Some(v))),
        italic: r.italic().map(|v| is_true(Some(v))),
        underline: underline.map(|u| u != "none").or(linked.then_some(true)),
        underline_style: underline.filter(|u| !matches!(*u, "none" | "sng")).map(str::to_owned),
        strike: r.strike().map(|s| s != "noStrike"),
        baseline: r.baseline().map(|b| b / 1000.0),
        size: r.size().map(|s| s / 100.0),
        latin: face(r.latin()),
        east_asian: face(r.east_asian()),
        complex_script: face(r.complex_script()),
        colour: colour(r.fill().map(a::SolidFill::element)).or_else(|| linked.then(|| SourceColour::scheme("hlink"))),
        highlight: colour(r.highlight().map(a::Any::element)),
        outline_width: line.and_then(a::Ln::width),
        outline_colour: line.and_then(|l| colour(a::Fills::new(l.element()).solid().map(a::SolidFill::element))),
        hyperlink: hyperlink.and_then(a::Hyperlink::id).map(str::to_owned),
        tooltip: hyperlink.and_then(a::Hyperlink::tooltip).map(str::to_owned),
        lang: non_empty(r.lang()),
        alt_lang: non_empty(r.alt_lang()),
    };
    (properties != RunProperties::default()).then_some(properties)
}

/// A paragraph's (`a:p`) plain text as ooxml-edit reads it: its runs' and fields' `a:t`, a
/// line break as `\v`; nothing else.
#[must_use]
pub fn paragraph_text(paragraph: &Element) -> String {
    use ooxml_xml::view::{child, elements, local, text};
    let mut out = String::new();
    for node in elements(paragraph) {
        match local(node) {
            "r" | "fld" => out.push_str(&child(node, "t").map(text).unwrap_or_default()),
            "br" => out.push('\u{b}'),
            _ => {}
        }
    }
    out
}
