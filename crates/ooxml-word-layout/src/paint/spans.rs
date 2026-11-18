//! Owns: one line's paint (docx2svg's `_Placer._line`): its pieces at their pen positions
//! grouped into spans of one run and format -- a run's text, a list label, a soft hyphen's
//! hyphen, a note's number on whole px, a script's size and raise, `w:position` -- its
//! tabs' leaders and underlined tabs, its inline pictures and drawings by their extents,
//! a note separator's line, and its decorations.
//!
//! Must not: decide where the line is (the geometry has) or how its pieces are measured.

use super::decorate::{decorate, separator_rule, white_text};
use super::positions::{wrapped_positions, Positions, Setting};
use super::{Drawn, PaintContext, PaintedLine, Rule, Span, SpanKind, TextEffects};
use crate::geometry::DrawnLine;
use crate::input::{label_props, list_level, RunFormatter};
use crate::paragraph::MeasuredParagraph;
use crate::px::Px;
use ooxml_base::Q;
use ooxml_text::pieces::{LABEL_SHIFT, NOTE_CONTINUATION, NOTE_SEPARATOR, OBJECT_CHAR as PICTURE_BULLET};
use ooxml_text::{FaceKey, Piece, PieceKind, Units, VerticalAlign};
use ooxml_word_model::model::{InlineObject, Paragraph};
use ooxml_word_model::value::{Props, Value};
use ooxml_word_model::{ResolvedParagraph, WordDocument};
use std::sync::Arc;

/// What a page's lines add to it beyond themselves.
#[derive(Debug, Default)]
pub(super) struct PageOut {
    pub(super) rules: Vec<Rule>,
    pub(super) pictures: Vec<Drawn>,
    pub(super) drawings: Vec<super::drawings::DrawingSlot>,
    /// The drawings met so far (their paint order within a layer).
    pub(super) drawn: usize,
    pub(super) unpainted: Vec<String>,
    /// Floating drawings not placed (their frame is not modelled), by path.
    pub(super) unplaced: Vec<String>,
}

/// One line to paint.
pub(super) struct LineJob<'a> {
    pub(super) document: &'a WordDocument,
    pub(super) paragraph: &'a Paragraph,
    pub(super) resolved: &'a ResolvedParagraph,
    pub(super) measured: &'a MeasuredParagraph,
    pub(super) line: &'a DrawnLine,
    /// What the paginator broke the line again with: another column's setting, segments
    /// beside drawings.
    pub(super) extra: Option<&'a crate::rebreak::LineExtra>,
    /// The right edge a note separator's line is cut at.
    pub(super) clip: Option<Px>,
}

/// `w:highlight`'s names as colours.
const HIGHLIGHT: [(&str, &str); 16] = [
    ("black", "000000"),
    ("blue", "0000FF"),
    ("cyan", "00FFFF"),
    ("green", "00FF00"),
    ("magenta", "FF00FF"),
    ("red", "FF0000"),
    ("yellow", "FFFF00"),
    ("white", "FFFFFF"),
    ("darkBlue", "000080"),
    ("darkCyan", "008080"),
    ("darkGreen", "008000"),
    ("darkMagenta", "800080"),
    ("darkRed", "800000"),
    ("darkYellow", "808000"),
    ("darkGray", "808080"),
    ("lightGray", "C0C0C0"),
];

/// A colour as written: six hex digits, else none (`_color`).
pub(super) fn colour(value: Option<&str>) -> Option<String> {
    let v = value.filter(|v| *v != "auto")?;
    (v.len() == 6 && v.chars().all(|c| c.is_ascii_hexdigit())).then(|| v.to_owned())
}

/// A shading's fill colour (`_shading`).
pub(super) fn shading(value: Option<&Value>) -> Option<String> {
    let Some(Value::Shading(s)) = value else { return None };
    if s.kind.as_deref() == Some("nil") {
        return None;
    }
    colour(s.fill.as_deref())
}

fn string(props: &Props, key: &str) -> Option<String> {
    props.get(key).and_then(Value::str).map(str::to_owned)
}

fn flag(props: &Props, key: &str) -> bool {
    props.get(key).is_some_and(Value::truthy)
}

/// Device px a run raised by `half_points` is drawn above its line's baseline, rounded
/// half up in magnitude (`half_points_offset_px`).
pub(super) fn offset_px(half_points: i64) -> i64 {
    let magnitude = Px::points(Q::new(i128::from(half_points.unsigned_abs()), 2)).round_half_up();
    if half_points >= 0 {
        magnitude
    } else {
        -magnitude
    }
}

/// `floor(x + 1/2)` of a ratio.
fn round_ratio(n: i128, d: i128) -> i64 {
    if d == 0 {
        return 0;
    }
    let v = Q::new(n, d) + Q::new(1, 2);
    i64::try_from(v.floor().to_integer()).unwrap_or(0)
}

/// The size a script run is drawn at and how far it is raised (positive) or lowered,
/// device px (`script_half_points`, `script_raise_half_points`).
fn script(cx: &PaintContext<'_>, key: &FaceKey, half_points: i64, align: VerticalAlign) -> (i64, i64) {
    let Some(sm) = cx.faces.script_metrics(key) else {
        return (half_points, 0);
    };
    let hp = u32::try_from(half_points).unwrap_or(0);
    let drawn = ooxml_text::glyphs::script_half_points(hp, align, &sm).map_or(half_points, i64::from);
    let raise = match (
        cx.faces.metrics(key),
        ooxml_text::glyphs::script_half_points(hp, align, &sm),
    ) {
        (Some(m), Some(drawn)) => {
            let upm = i128::from(m.units_per_em);
            let extent = i128::from(match align {
                VerticalAlign::Superscript => m.ascent + m.line_gap,
                VerticalAlign::Subscript => m.descent,
            });
            let bound =
                (round_ratio(extent * i128::from(hp), upm) - round_ratio(extent * i128::from(drawn), upm)).max(0);
            let offset = match align {
                VerticalAlign::Superscript => sm.superscript_offset,
                VerticalAlign::Subscript => sm.subscript_offset,
            };
            match offset {
                Some(o) if ooxml_text::glyphs::script_fields_taken(&sm) => {
                    if o <= 0 {
                        0
                    } else {
                        round_ratio(i128::from(hp) * i128::from(o), upm).min(bound)
                    }
                }
                _ => bound,
            }
        }
        _ => 0,
    };
    let px = offset_px(raise);
    (drawn, if align == VerticalAlign::Superscript { px } else { -px })
}

fn vertical(format: &ooxml_word_model::resolve::CharacterFormat) -> Option<VerticalAlign> {
    match format.vertical_align.as_deref() {
        Some("superscript") => Some(VerticalAlign::Superscript),
        Some("subscript") => Some(VerticalAlign::Subscript),
        _ => None,
    }
}

/// `left` px moved by `units` layout units.
pub(super) fn px_at(left: Px, units: Units) -> Px {
    let (n, d) = units.parts();
    left + Px::from(Q::new(n, d) * Px::layout_unit())
}

/// What a span is grouped by: its run or label, kind, face, size and baseline.
#[derive(Debug, Clone, PartialEq, Eq)]
struct SpanKey {
    run: Option<usize>,
    kind: SpanKind,
    face: Text,
    bold: bool,
    italic: bool,
    half_points: i64,
    y: i64,
}

type Text = ooxml_word_model::value::Text;

/// A line's painter.
struct LinePainter<'a, 'c> {
    cx: &'a PaintContext<'c>,
    job: &'a LineJob<'a>,
    positions: Positions,
    spans: Vec<(SpanKey, Span)>,
    /// The last span takes the next glyph of its key (no tab, break or object between).
    open: bool,
    out: &'a mut PageOut,
    label: Option<Props>,
}

impl LinePainter<'_, '_> {
    fn run_path(&self, run: usize) -> String {
        let paragraph = self.job.paragraph;
        match paragraph.runs.get(run).map(|r| &*r.path) {
            Some(p) if !p.is_empty() => format!("{}/{p}", paragraph.path),
            _ => paragraph.path.to_string(),
        }
    }

    /// The label's properties: the mark's with the level's `w:rPr` on top.
    fn label_props(&mut self) -> Props {
        if let Some(found) = &self.label {
            return found.clone();
        }
        let resolved = self.job.resolved;
        let props = match list_level(self.job.document, &resolved.paragraph) {
            Some(level) => label_props(&resolved.mark, level),
            None => (*resolved.mark).clone(),
        };
        self.label = Some(props.clone());
        props
    }

    /// What a piece is drawn as: its properties, path, kind and run (`None`: the label's).
    fn source_of(&mut self, piece: &Piece) -> (Props, String, SpanKind, Option<usize>) {
        if piece.source < 0 {
            let props = self.label_props();
            return (
                props,
                format!("{}/label", self.job.paragraph.path),
                SpanKind::Label,
                None,
            );
        }
        let k = usize::try_from(piece.run).unwrap_or(usize::MAX);
        let props = self.job.resolved.runs.get(k).map(|p| (**p).clone()).unwrap_or_default();
        let kind = if piece.kind == PieceKind::SoftHyphen {
            SpanKind::Hyphen
        } else {
            SpanKind::Text
        };
        (props, self.run_path(k), kind, Some(k))
    }

    /// A new span of `key` from `x`, in `props`' colours and decorations.
    fn open_span(&mut self, key: SpanKey, props: &Props, path: String, x: Px) {
        let span = Span {
            path,
            chars: Vec::new(),
            xs: Vec::new(),
            y: key.y,
            end: x,
            face: Arc::clone(&key.face),
            bold: key.bold,
            italic: key.italic,
            half_points: key.half_points,
            color: colour(props.get("color").and_then(Value::str)),
            underline: string(props, "u").filter(|u| u != "none"),
            strike: flag(props, "strike"),
            double_strike: flag(props, "dstrike"),
            highlight: props
                .get("highlight")
                .and_then(Value::str)
                .and_then(|h| HIGHLIGHT.iter().find(|(name, _)| *name == h).map(|(_, c)| *c)),
            shading: shading(props.get("shd")),
            kind: key.kind,
            line_baseline: self.job.line.baseline,
            vertical_align: None,
            effects: TextEffects::of(props),
        };
        self.spans.push((key, span));
    }

    /// A text piece drawn at `x`.
    fn glyph(&mut self, piece: &Piece, x: Px) {
        let document = self.job.document;
        let (props, path, kind, run) = self.source_of(piece);
        let ch = if piece.kind == PieceKind::SoftHyphen || piece.ch == '\u{2011}' {
            '-'
        } else {
            piece.ch
        };
        let measured_ch = if piece.source >= 0 { ch } else { piece.ch };
        let format = RunFormatter::new(document, &props).format(measured_ch).clone();
        let face: Text = piece
            .format()
            .as_ref()
            .and_then(|f| f.face.as_ref())
            .map_or_else(|| format.face.clone(), |f| Arc::clone(&f.family));
        let mut drawn = format.half_points;
        let mut offset = 0;
        if let Some(align) = vertical(&format) {
            let key = FaceKey {
                family: Arc::clone(&format.face),
                bold: format.bold,
                italic: format.italic,
            };
            (drawn, offset) = script(self.cx, &key, format.half_points, align);
        }
        let y = self.job.line.baseline - offset_px(format.position) - offset;
        let key = SpanKey {
            run,
            kind,
            face: Arc::clone(&face),
            bold: format.bold,
            italic: format.italic,
            half_points: drawn,
            y,
        };
        if !self.open || self.spans.last().is_none_or(|(k, _)| *k != key) {
            self.open_span(key, &props, path, x);
            if let Some((_, span)) = self.spans.last_mut() {
                span.vertical_align.clone_from(&format.vertical_align);
            }
        }
        let note_number = piece.source >= 0
            && self
                .job
                .paragraph
                .runs
                .get(usize::try_from(piece.run).unwrap_or(usize::MAX))
                .is_some_and(|r| r.note_number);
        let width = if piece.kind == PieceKind::SoftHyphen {
            piece.hyphen()
        } else {
            piece.width()
        };
        if let Some((_, span)) = self.spans.last_mut() {
            span.chars
                .push(if piece.kind == PieceKind::Space { piece.ch } else { ch });
            span.xs.push(if note_number { Px::int(x.round_half_up()) } else { x });
            span.end = px_at(x, width);
        }
        self.open = true;
    }

    /// A picture bullet at its label's pen (`_picture_bullet`): a square of its size
    /// standing on the baseline, its left edge and side each on a whole px, moved by the
    /// level's `w:position` twice and a script's raise once.
    fn picture_bullet(&mut self, piece: &Piece, x: Px) {
        let props = self.label_props();
        let level = list_level(self.job.document, &self.job.resolved.paragraph);
        let bullet = level
            .and_then(|l| l.picture_bullet)
            .and_then(|id| self.job.document.picture_bullets.get(&id));
        let format = RunFormatter::new(self.job.document, &props).format(' ').clone();
        let mut bottom = self.job.line.baseline - 2 * offset_px(format.position);
        if let Some(align) = vertical(&format) {
            let key = FaceKey {
                family: Arc::clone(&format.face),
                bold: format.bold,
                italic: format.italic,
            };
            bottom -= script(self.cx, &key, format.half_points, align).1;
        }
        let side = px_at(Px::ZERO, piece.width()).round_half_up();
        self.out.pictures.push(Drawn {
            path: format!("{}/label", self.job.paragraph.path),
            relationship: bullet.and_then(|b| b.relationship.clone()),
            part: bullet.and_then(|b| b.part.clone()),
            x: Px::int(x.round_half_up()),
            y: Px::int(bottom - side),
            width: Px::int(side),
            height: Px::int(side),
            crop: None,
        });
    }

    /// An inline picture or drawing, by its extent, standing on the baseline.
    fn object(&mut self, piece: &Piece, x: Px, counts: &mut std::collections::HashMap<usize, usize>) {
        let k = usize::try_from(piece.run).unwrap_or(usize::MAX);
        let count = counts.entry(k).or_default();
        let n = *count;
        *count += 1;
        let Some(run) = self.job.paragraph.runs.get(k) else {
            return;
        };
        let marks: Vec<&str> = run
            .breaks
            .iter()
            .filter_map(|(_, kind)| kind.strip_prefix("drawing:"))
            .collect();
        let Some(mark) = marks.get(n) else { return };
        let v: Vec<i64> = mark.split(':').map(|s| s.parse().unwrap_or_default()).collect();
        let at = |i: usize| v.get(i).copied().unwrap_or_default();
        let (cx_emu, cy_emu, left, bottom) = (at(0), at(1), at(2), at(5));
        let emu = |e: i64| Px::points(Q::new(i128::from(e), 12_700));
        let (width, depth) = picture_size(cx_emu, cy_emu);
        let height = &self.job.measured.heights;
        let stands = match height.get(self.job.line.number).and_then(|h| h.object_drop) {
            Some(drop) => self.job.line.top + drop,
            None => Px::int(self.job.line.baseline),
        };
        let path = if run.path.is_empty() {
            String::new()
        } else {
            self.run_path(k)
        };
        let relationship = match run.drawings.get(n) {
            Some(InlineObject::Picture(r)) => Some(Arc::clone(r)),
            Some(InlineObject::Graphic(graphic)) => {
                // An inline shape, group, chart or SmartArt: drawn as a floating drawing is,
                // in its line's box, in front of the text, by its painter.
                self.out.drawn += 1;
                self.out.drawings.push(super::drawings::DrawingSlot {
                    path: if path.is_empty() {
                        self.job.paragraph.path.to_string()
                    } else {
                        path
                    },
                    paragraph_path: self.job.paragraph.path.to_string(),
                    run: k,
                    index: n,
                    inline: true,
                    x: x + emu(left),
                    y: stands - emu(bottom) - depth,
                    width,
                    height: depth,
                    layer: super::drawings::Layer::Front,
                    order: (0, self.out.drawn),
                    story: None,
                    graphic: Some(Arc::clone(graphic)),
                    no_overlap: false,
                });
                return;
            }
            Some(InlineObject::None) | None => None,
        };
        self.out.pictures.push(Drawn {
            path,
            relationship,
            part: None,
            x: x + emu(left),
            y: stands - emu(bottom) - depth,
            width,
            height: depth,
            crop: None,
        });
    }
}

/// The size Word draws an inline picture of extent `cx` x `cy` EMU, device px
/// (`picture_size`): the extent scaled, both ways alike, to fit the extent truncated to
/// whole twips.
pub(super) fn picture_size(cx: i64, cy: i64) -> (Px, Px) {
    let emu = |e: i64| Px::points(Q::new(i128::from(e), 12_700));
    if cx <= 0 || cy <= 0 {
        return (emu(cx), emu(cy));
    }
    let truncated = |e: i64| Q::new(i128::from(e.div_euclid(635) * 635), i128::from(e));
    let scale = truncated(cx).min(truncated(cy));
    (emu(cx) * scale, emu(cy) * scale)
}

/// Where each piece of `line` is drawn, as its paragraph sets it and as the paginator broke
/// it.
fn positions_of(cx: &PaintContext<'_>, job: &LineJob<'_>, line: &ooxml_text::Line) -> Positions {
    let measured = &job.measured.measured;
    let setting = Setting {
        alignment: job.resolved.paragraph.get("jc").and_then(Value::str),
        first_line: job.line.number == 0,
        last_line: job.line.number + 1 == measured.lines.len(),
        decimal: cx.decimal,
    };
    wrapped_positions(&measured.pieces, line, &job.measured.geometry, job.extra, &setting)
}

/// Paint one line: its spans (pushed onto the line), its rules, pictures and decorations
/// (onto the page).
pub(super) fn paint_line(cx: &PaintContext<'_>, job: &LineJob<'_>, out: &mut PageOut) -> Option<PaintedLine> {
    let measured = job.measured;
    let pieces = &measured.measured.pieces;
    let broken = measured.measured.lines.get(job.line.number)?;
    let pp = &job.resolved.paragraph;
    let positions = positions_of(cx, job, broken);
    let mut painter = LinePainter {
        cx,
        job,
        positions,
        spans: Vec::new(),
        open: false,
        out,
        label: None,
    };
    let left = job.line.left;
    let mut counts = std::collections::HashMap::new();
    for j in broken.start..broken.end {
        let Some(piece) = pieces.get(j) else { break };
        let Some(&at) = painter.positions.at.get(&j) else {
            continue;
        };
        match piece.kind {
            PieceKind::Tab => {
                painter.open = false;
                painter.tab_underline(piece, left, j);
                continue;
            }
            PieceKind::Break => {
                painter.open = false;
                continue;
            }
            PieceKind::SoftHyphen if !(broken.hyphenated && j + 1 == broken.end) => continue,
            _ => {}
        }
        if piece.source < 0 && piece.ch == LABEL_SHIFT {
            continue;
        }
        let x = px_at(left, at);
        if piece.kind == PieceKind::Object {
            painter.open = false;
            painter.object(piece, x, &mut counts);
            continue;
        }
        if piece.source >= 0 && matches!(piece.ch, NOTE_SEPARATOR | NOTE_CONTINUATION) {
            painter.open = false;
            painter.separator(piece, x, left);
            continue;
        }
        if piece.source < 0 && piece.ch == PICTURE_BULLET {
            painter.open = false;
            painter.picture_bullet(piece, x);
            continue;
        }
        painter.glyph(piece, x);
    }
    painter.leaders(left);
    let mut spans: Vec<Span> = painter.spans.into_iter().map(|(_, s)| s).collect();
    if job.line.white_text && shading(pp.get("shd")).is_none() {
        white_text(&mut spans);
    }
    decorate(cx, &spans, job.line.band, out);
    Some(PaintedLine {
        path: job.paragraph.path.clone(),
        paragraph_id: job.paragraph.para_id.clone(),
        number: job.line.number,
        baseline: job.line.baseline,
        top: job.line.top,
        pitch: job.line.pitch,
        column: job.line.column,
        band: job.line.band,
        spans,
        story: None,
    })
}

impl LinePainter<'_, '_> {
    /// The leader of a tab's stop: a custom stop's `w:leader`, none for a default one.
    fn leader_of(&self, tab: &super::TabSpan) -> Option<String> {
        if tab.default {
            return None;
        }
        let Some(Value::Tabs(stops)) = self.job.resolved.paragraph.get("tabs") else {
            return None;
        };
        let origin = self.job.measured.geometry.tab_origin;
        stops
            .iter()
            .filter(|s| !matches!(&*s.alignment, "clear" | "bar"))
            .find(|s| {
                ooxml_text::twips(s.position) + origin == tab.stop
                    && ooxml_text::TabAlign::parse(&s.alignment).unwrap_or(ooxml_text::TabAlign::Left) == tab.align
            })
            .and_then(|s| s.leader.as_deref().map(str::to_owned))
            .filter(|l| l != "none")
    }

    /// An underlined tab's underline across its room, from the pen at the tab to the pen
    /// after it, as a run's under a space (`_tab_underline`).
    fn tab_underline(&mut self, piece: &Piece, left: Px, j: usize) {
        if piece.run < 0 {
            return;
        }
        let Some(tab) = self.positions.tabs.get(&j).copied() else {
            return;
        };
        let k = usize::try_from(piece.run).unwrap_or(usize::MAX);
        let Some(props) = self.job.resolved.runs.get(k).cloned() else {
            return;
        };
        let Some(underline) = string(&props, "u").filter(|u| u != "none") else {
            return;
        };
        let start = px_at(left, self.positions.at.get(&j).copied().unwrap_or(tab.from));
        let _ = &tab;
        let shift = self.positions.at.get(&j).copied().unwrap_or(tab.from) - tab.from;
        let end = px_at(left, tab.to + shift);
        if end <= start {
            return;
        }
        let format = RunFormatter::new(self.job.document, &props).format(' ').clone();
        let span = Span {
            path: self.run_path(k),
            chars: vec![' '],
            xs: vec![start],
            y: self.job.line.baseline - offset_px(format.position),
            end,
            face: Arc::clone(&format.face),
            bold: format.bold,
            italic: format.italic,
            half_points: format.half_points,
            color: colour(props.get("color").and_then(Value::str)),
            underline: Some(underline),
            strike: false,
            double_strike: false,
            highlight: None,
            shading: None,
            kind: SpanKind::Tab,
            line_baseline: self.job.line.baseline,
            vertical_align: None,
            effects: None,
        };
        let key = SpanKey {
            run: None,
            kind: SpanKind::Tab,
            face: Arc::clone(&span.face),
            bold: span.bold,
            italic: span.italic,
            half_points: span.half_points,
            y: span.y,
        };
        self.spans.push((key, span));
    }

    /// A note separator's line.
    fn separator(&mut self, piece: &Piece, x: Px, left: Px) {
        let k = usize::try_from(piece.run).unwrap_or(usize::MAX);
        let Some(props) = self.job.resolved.runs.get(k).cloned() else {
            return;
        };
        let geometry = &self.job.measured.geometry;
        let (x, end) = if piece.ch == NOTE_CONTINUATION {
            let start = if self.job.line.number == 0 {
                geometry.first_start
            } else {
                geometry.start
            };
            (px_at(left, start), Some(px_at(left, geometry.right)))
        } else {
            (x, None)
        };
        let format = RunFormatter::new(self.job.document, &props).format(' ').clone();
        let key = FaceKey {
            family: Arc::clone(&format.face),
            bold: format.bold,
            italic: format.italic,
        };
        match separator_rule(
            self.cx.faces.decorations(&key).as_ref(),
            &format,
            (x, end, self.job.clip),
            self.job.line.baseline,
        ) {
            Some((x, y, width, height)) => self.out.rules.push(Rule {
                kind: "separator",
                path: self.run_path(k),
                x,
                y,
                width,
                height,
                color: colour(props.get("color").and_then(Value::str)),
                style: None,
            }),
            None => self.out.unpainted.push("a note separator".into()),
        }
    }

    /// The line's tabs' leaders, after its text (`_leader`): the glyph Word draws each
    /// with, in the tab's run's format, at whole multiples of its advance (rounded to the
    /// layout unit) from the page's left edge, from the first at or after the pen at the
    /// tab to the last that ends at or before the pen after it.
    fn leaders(&mut self, left: Px) {
        let tabs: Vec<(usize, super::TabSpan)> = self.positions.tabs.iter().map(|(k, t)| (*k, *t)).collect();
        for (j, tab) in tabs {
            let Some(leader) = self.leader_of(&tab) else { continue };
            let Some(piece) = self.job.measured.measured.pieces.get(j) else {
                continue;
            };
            let glyph = LEADER_GLYPHS.iter().find(|(name, _)| *name == leader).map(|(_, c)| *c);
            let (Some(ch), true) = (glyph, piece.run >= 0) else {
                self.out.unpainted.push(format!("a {leader} tab leader"));
                continue;
            };
            let at = self.positions.at.get(&j).copied().unwrap_or(tab.from);
            let (start, end) = (at, tab.to + (at - tab.from));
            self.leader(piece, ch, left, (start, end));
        }
    }

    fn leader(&mut self, piece: &Piece, ch: char, left: Px, (start, end): (Units, Units)) {
        let k = usize::try_from(piece.run).unwrap_or(usize::MAX);
        let Some(props) = self.job.resolved.runs.get(k).cloned() else {
            return;
        };
        let format = RunFormatter::new(self.job.document, &props).format(ch).clone();
        let key = FaceKey {
            family: Arc::clone(&format.face),
            bold: format.bold,
            italic: format.italic,
        };
        let (drawn, offset) = match vertical(&format) {
            Some(align) => script(self.cx, &key, format.half_points, align),
            None => (format.half_points, 0),
        };
        let Some(advance) = self.cx.faces.advance(&key, ch) else {
            self.out
                .unpainted
                .push("a tab leader of a face whose advance is not known".into());
            return;
        };
        let step = round_ratio(
            i128::from(advance.advance) * i128::from(drawn) * 2048,
            i128::from(advance.units_per_em),
        );
        if step <= 0 {
            return;
        }
        let lu = Px::layout_unit();
        let origin = left.value() / lu;
        let unit = |u: Units| {
            let (n, d) = u.parts();
            Q::new(n, d)
        };
        let step_q = Q::from_integer(i128::from(step));
        let first = ((origin + unit(start)) / step_q).ceil().to_integer();
        let last = ((origin + unit(end)) / step_q).floor().to_integer() - 1;
        if last < first {
            return;
        }
        let xs: Vec<Px> = (first..=last)
            .map(|n| Px::from(step_q * Q::from_integer(n) * lu))
            .collect();
        let span = Span {
            path: self.run_path(k),
            chars: vec![ch; xs.len()],
            end: Px::from(step_q * Q::from_integer(last + 1) * lu),
            xs,
            y: self.job.line.baseline - offset_px(format.position) - offset,
            face: Arc::clone(&format.face),
            bold: format.bold,
            italic: format.italic,
            half_points: drawn,
            color: colour(props.get("color").and_then(Value::str)),
            underline: None,
            strike: false,
            double_strike: false,
            highlight: None,
            shading: None,
            kind: SpanKind::Leader,
            line_baseline: self.job.line.baseline,
            vertical_align: format.vertical_align.clone(),
            effects: TextEffects::of(&props),
        };
        let key = SpanKey {
            run: None,
            kind: SpanKind::Leader,
            face: Arc::clone(&span.face),
            bold: span.bold,
            italic: span.italic,
            half_points: drawn,
            y: span.y,
        };
        self.spans.push((key, span));
    }
}

/// The glyph Word draws each tab leader with.
const LEADER_GLYPHS: [(&str, char); 5] = [
    ("dot", '.'),
    ("hyphen", '-'),
    ("underscore", '_'),
    ("heavy", '_'),
    ("middleDot", '\u{b7}'),
];
