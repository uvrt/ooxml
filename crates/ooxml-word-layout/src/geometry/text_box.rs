//! Owns: a text box's text (`w:txbxContent`) laid out as a story of its own, as docx2svg's
//! `_Placer._text_box` lays it out: its blocks the body of one section of one column as wide
//! as the box's text area in whole twips, its floating drawings that text does not wrap
//! around taken out ([`story_blocks`]), its list items counted as the document's text boxes
//! count them -- one story of their own, from box to box in the order of their anchors
//! ([`text_box_labels`]) -- and its lines stacked from the top, middle or foot of its room
//! ([`lay_out_text_box`]); with what docx2svg says of it.
//!
//! Where the box's text area is -- its shape's text rectangle less half its outline -- and
//! what colour its automatic text takes are its painter's ([`TextBoxFrame`]).
//!
//! Must not: paint (`crate::paint::text_box` does), or read a drawing's geometry.

use super::stories::{StoryJob, StoryPage};
use super::walk::{walk, WalkFrom};
use crate::flow::{body_paragraphs, FlowContext, FlowItem, FlowMaker, Labels};
use crate::px::Px;
use crate::sections::Sections;
use crate::stories::{stack_height, story_section};
use ooxml_base::Q;
use ooxml_word_model::model::{Block, BodyValue, Graphic, InlineObject, Paragraph, Run, Table};
use ooxml_word_model::value::{Text, Value};
use ooxml_word_model::{Model, WordDocument};
use std::collections::HashMap;
use std::sync::Arc;

/// EMU per twip.
const EMU_PER_TWIP: i128 = 635;

/// Device px per twip.
fn twip_px() -> Q {
    Q::new(300, 1440)
}

/// A text box as its painter found it: the shape holding the text, its box (exact px from
/// the page's top left), and what only the painter knows -- how far in from each side of the
/// box its text area starts before `wps:bodyPr`'s insets, and what its automatic text is
/// drawn in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextBoxFrame {
    /// The shape: its blocks and `wps:bodyPr`.
    pub graphic: Arc<Graphic>,
    /// Its box's left edge.
    pub x: Px,
    /// Its top.
    pub y: Px,
    /// Its width.
    pub width: Px,
    /// Its height.
    pub height: Px,
    /// How far in from the box's left, top, right and bottom its text area starts, EMU:
    /// its geometry's text rectangle less half its outline's width
    /// (docx2svg's `drawing.text_area_edges`).
    pub edges: [Q; 4],
    /// What its text whose colour is automatic is drawn in (`RRGGBB`; `None`: black).
    pub colour: Option<String>,
}

/// A text box laid out: its story placed on the page, and what docx2svg says of it.
#[derive(Debug, Clone)]
pub struct TextBoxStory {
    /// Its story, stacked where its text goes (`None`: its text is not laid out).
    pub story: Option<StoryPage>,
    /// What its automatic text is drawn in.
    pub colour: Option<String>,
    /// The width of its text column (the box less its left and right insets), px.
    pub width: Px,
    /// docx2svg's warnings of it, `(code, message)`, in the order said.
    pub said: Vec<(String, String)>,
}

/// Every list label of the document's text boxes anchored in its body, counted as Word
/// counts them: the text boxes one story of their own, their lists going on from box to
/// box in the order of their anchors (docx2svg's `linebreak.text_box_labels`). Keyed by
/// the paths the paragraphs have once laid out in their box: a top-level block's under
/// its anchor's paragraph's path.
pub fn text_box_labels(model: &mut Model, document: &WordDocument) -> HashMap<String, Option<String>> {
    struct Count<'d, 'm> {
        model: &'m mut Model,
        document: &'d WordDocument,
        counters: ooxml_word_model::labels::Counters,
        out: HashMap<String, Option<String>>,
    }
    impl Count<'_, '_> {
        fn blocks(&mut self, blocks: &[Block], prefix: Option<&str>) {
            for block in blocks {
                match block {
                    Block::Table(t) => {
                        for cell in t.rows.iter().flat_map(|row| row.iter()) {
                            self.blocks(cell, None);
                        }
                    }
                    Block::Paragraph(p) => {
                        let path = prefix.map_or_else(|| p.path.to_string(), |at| format!("{at}/{}", p.path));
                        let label = self.label(p);
                        self.out.insert(path, label);
                    }
                }
            }
        }
        fn label(&mut self, paragraph: &Paragraph) -> Option<String> {
            let pp = self.model.resolve_paragraph_props(self.document, paragraph);
            let num_id = pp.get("numPr.numId").and_then(Value::int).unwrap_or(0);
            if num_id == 0 {
                return None;
            }
            let ilvl = pp.get("numPr.ilvl").and_then(Value::int).unwrap_or(0);
            self.counters.item(&self.document.context.numbering, num_id, ilvl)
        }
        fn graphic(&mut self, graphic: &Graphic, at: &str) {
            if !graphic.text.is_empty() {
                let text = Arc::clone(&graphic.text);
                self.blocks(&text, Some(at));
            }
            for inner in graphic.children.iter() {
                self.graphic(inner, at);
            }
        }
        fn paragraphs(&mut self, blocks: &[Block]) {
            for block in blocks {
                match block {
                    Block::Table(t) => {
                        for cell in t.rows.iter().flat_map(|row| row.iter()) {
                            self.paragraphs(cell);
                        }
                    }
                    Block::Paragraph(p) => {
                        for run in p.runs.iter() {
                            for found in run.drawings.iter() {
                                if let InlineObject::Graphic(g) = found {
                                    self.graphic(g, &p.path);
                                }
                            }
                            for anchor in run.anchors.iter() {
                                if let Some(g) = anchor.graphic.as_deref() {
                                    self.graphic(g, &p.path);
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    let mut count = Count {
        model,
        document,
        counters: ooxml_word_model::labels::Counters::default(),
        out: HashMap::new(),
    };
    count.paragraphs(&document.body);
    count.out
}

/// A text box's blocks as laid out in it: each top-level block's path under its anchor's
/// paragraph's, and the mark of every floating drawing text does not wrap around taken
/// out (`paginate.floating_marks`); whether any was.
pub fn story_blocks(blocks: &[Block], paragraph_path: &str) -> (Vec<Block>, bool) {
    let mut taken = false;
    let out = blocks
        .iter()
        .map(|block| {
            let block = unmarked(block, &mut taken);
            match block {
                Block::Paragraph(p) => Block::Paragraph(Arc::new(Paragraph {
                    path: Text::from(format!("{paragraph_path}/{}", p.path).as_str()),
                    ..(*p).clone()
                })),
                Block::Table(t) => Block::Table(Arc::new(Table {
                    path: Text::from(format!("{paragraph_path}/{}", t.path).as_str()),
                    ..(*t).clone()
                })),
            }
        })
        .collect();
    (out, taken)
}

/// `block` with the marks of its floating drawings that text does not wrap around taken
/// out, cells' included.
fn unmarked(block: &Block, taken: &mut bool) -> Block {
    match block {
        Block::Paragraph(p) => {
            let runs: Vec<Run> = p.runs.iter().map(|run| unmarked_run(run, taken)).collect();
            Block::Paragraph(Arc::new(Paragraph {
                runs: runs.into(),
                ..(**p).clone()
            }))
        }
        Block::Table(t) => {
            let rows = t
                .rows
                .iter()
                .map(|row| {
                    row.iter()
                        .map(|cell| cell.iter().map(|b| unmarked(b, taken)).collect())
                        .collect()
                })
                .collect();
            Block::Table(Arc::new(Table { rows, ..(**t).clone() }))
        }
    }
}

fn unmarked_run(run: &Run, taken: &mut bool) -> Run {
    let kept = |mark: &Text| -> bool {
        let Some(k) = mark.strip_prefix("anchor:").and_then(|k| k.parse::<usize>().ok()) else {
            return true;
        };
        run.anchors.get(k).is_none_or(crate::floating::moves_text)
    };
    if run.breaks.iter().all(|(_, mark)| kept(mark)) {
        return run.clone();
    }
    *taken = true;
    Run {
        breaks: run.breaks.iter().filter(|(_, mark)| kept(mark)).cloned().collect(),
        ..run.clone()
    }
}

/// Whether a text box's blocks hold what this layout does not lay out in one: a floating
/// drawing text wraps around (docx2svg stops the box's text there), or an inline drawing
/// that is not a picture (docx2svg paints it with the box).
pub fn unsupported_in(blocks: &[Block]) -> Option<&'static str> {
    for paragraph in body_paragraphs(blocks).0 {
        for run in paragraph.runs.iter() {
            if run.anchors.iter().any(crate::floating::moves_text) {
                return Some("a floating drawing text wraps around in a text box");
            }
            if run.drawings.iter().any(|d| matches!(d, InlineObject::Graphic(_))) {
                return Some("a drawing that is not a picture in a text box");
            }
        }
    }
    None
}

/// A `wps:bodyPr` setting.
fn body<'g>(graphic: &'g Graphic, name: &str) -> Option<&'g BodyValue> {
    graphic.body.iter().find(|(k, _)| &**k == name).map(|(_, v)| v)
}

fn body_text<'g>(graphic: &'g Graphic, name: &str, default: &'g str) -> &'g str {
    match body(graphic, name) {
        Some(BodyValue::Str(s)) => s,
        _ => default,
    }
}

/// An inset, twips, exact.
fn inset(graphic: &Graphic, name: &str, default: i64) -> Q {
    let emu = match body(graphic, name) {
        Some(BodyValue::Int(Some(v))) => *v,
        _ => default,
    };
    Q::new(i128::from(emu), EMU_PER_TWIP)
}

fn half_up(x: Q) -> Q {
    (x + Q::new(1, 2)).floor()
}

/// Where a text box's lines are walked from, and the foot of its room.
struct Room {
    top: Px,
    left: Px,
    bottom: Px,
}

/// What the layout needs of one text box beyond the story: its anchor's paragraph's path,
/// the drawing's path, and the page's layout.
pub struct TextBoxJob<'a> {
    /// The drawing's path (`.../wp:anchor[1]`), as docx2svg names it in what it says.
    pub path: &'a str,
    /// Its anchor's paragraph's path.
    pub paragraph_path: &'a str,
    /// The text boxes' list labels ([`text_box_labels`]).
    pub labels: &'a HashMap<String, Option<String>>,
}

/// Lay a text box's text out in its frame (`_text_box`).
pub fn lay_out_text_box(story: &mut StoryJob<'_, '_>, job: &TextBoxJob<'_>, frame: &TextBoxFrame) -> TextBoxStory {
    let graphic = &frame.graphic;
    let mut said = Vec::new();
    let mut out = TextBoxStory {
        story: None,
        colour: frame.colour.clone(),
        width: Px::ZERO,
        said: Vec::new(),
    };
    if body_text(graphic, "vert", "horz") != "horz" || body_text(graphic, "rot", "0") != "0" {
        said.push((
            "drawing-not-drawn".to_owned(),
            format!("a text box's vertical or rotated text is not laid out ({})", job.path),
        ));
        out.said = said;
        return out;
    }
    let document = story.cx.document;
    let mode15 = crate::input::rules(document).mode15();
    let stated = [
        inset(graphic, "lIns", 91_440),
        inset(graphic, "tIns", 45_720),
        inset(graphic, "rIns", 91_440),
        inset(graphic, "bIns", 45_720),
    ];
    // Each side's inset plus its distance in from the shape's edge, to the nearest twip, a
    // half up (`make_text_box_top_probe.py`).
    let insets: Vec<Q> = stated
        .iter()
        .zip(frame.edges)
        .map(|(inset, edge)| half_up(*inset + edge / Q::from_integer(EMU_PER_TWIP)))
        .collect();
    let side = |k: usize| insets.get(k).copied().unwrap_or_default();
    let inner = frame.width.value() / twip_px() - side(0) - side(2);
    let width_twips = i64::try_from(inner.floor().to_integer()).unwrap_or(1).max(1);
    let (blocks, nested) = story_blocks(&graphic.text, job.paragraph_path);
    let Some(flow) = box_flow(story, job, &blocks, width_twips) else {
        out.said = said;
        return out;
    };
    let content = stack_height(&flow.items, mode15);
    if nested {
        said.push((
            "drawing-not-drawn".to_owned(),
            format!("a floating drawing inside a text box is not drawn ({})", job.path),
        ));
    }
    out.width = frame.width - Px::from((side(0) + side(2)) * twip_px());
    let room = room(frame, (&insets, content, mode15), body_text(graphic, "anchor", "t"));
    let walked = walk(
        &flow.items,
        &WalkFrom {
            top: room.top,
            left: room.left,
            mode15,
            bottom: Some(room.bottom),
        },
    );
    out.story = Some(StoryPage {
        kind: "text box",
        part: Text::from(job.path),
        blocks: blocks.into(),
        flow,
        walked,
        clip: None,
        broken: crate::paginate::LineStore::new(),
    });
    out.said = said;
    out
}

/// A text box's blocks laid out as the body of the document's first section, one column
/// `width` twips wide with no left or right margin (`story_document` at the box's width).
fn box_flow(
    story: &mut StoryJob<'_, '_>,
    job: &TextBoxJob<'_>,
    blocks: &[Block],
    width: i64,
) -> Option<crate::flow::Flow> {
    let document = story.cx.document;
    let section = document.sections.first()?;
    let mut props = (*section.properties).clone();
    props.page_size.width = width;
    props.margins.left = 0;
    props.margins.right = 0;
    props.margins.gutter = 0;
    let (paragraphs, _) = body_paragraphs(blocks);
    let labels = box_labels(story.model, document, &paragraphs, job.labels);
    let sections = Sections::new(&[story_section(&props, paragraphs.len())], false);
    let fcx = FlowContext {
        document,
        blocks,
        sections: &sections,
        labels: Labels::Given(&labels),
        faces: story.cx.faces,
        locale: story.cx.locale,
        tables: story.cx.tables,
        roles: &[],
    };
    Some(FlowMaker::new(&fcx, story.model, story.state).build())
}

/// Where a text box's text starts and the foot of its room (`make_text_box_top_probe.py`,
/// ROADMAP F.14): its corner on the nearest twip, plus the insets, in the nearest layout
/// unit; the content stacked down from the room's top, or centred, or at its foot where
/// it fits; below mode 15 the room a twip shorter, truncated to the unit.
fn room(frame: &TextBoxFrame, (insets, content, mode15): (&[Q], Px, bool), anchor: &str) -> Room {
    let side = |k: usize| insets.get(k).copied().unwrap_or_default();
    let px = twip_px();
    let origin_x = half_up(frame.x.value() / px) + side(0);
    let origin_y = half_up(frame.y.value() / px) + side(1);
    let mut top = Px::quantise_q(origin_y * px);
    let mut room = Px::from((frame.height.value() / px - side(1) - side(3)) * px);
    if !mode15 {
        room -= Px::from(px).quantise_down();
    }
    let offset = match anchor {
        "ctr" if content < room => Px::from((room - content).value() / Q::from_integer(2)),
        "b" if content < room => room - content,
        _ => Px::ZERO,
    };
    let mut bottom = top + room;
    let moved = (top + offset).quantise();
    bottom += moved - top;
    top = moved;
    let pixel = Q::from_integer(i128::from(Px::quantise_q(origin_x * px).round_half_up()));
    let unit = Px::layout_unit();
    let left = Px::from(half_up(pixel / unit) * unit);
    Room { top, left, bottom }
}

/// A text box's paragraphs' labels: as the document's text boxes count them, else counted
/// in the box (a header's text box).
fn box_labels(
    model: &mut Model,
    document: &WordDocument,
    paragraphs: &[Arc<Paragraph>],
    counted: &HashMap<String, Option<String>>,
) -> Vec<Option<String>> {
    let mut counters = ooxml_word_model::labels::Counters::default();
    paragraphs
        .iter()
        .map(|p| {
            if let Some(found) = counted.get(&*p.path) {
                return found.clone();
            }
            let pp = model.resolve_paragraph_props(document, p);
            let num_id = pp.get("numPr.numId").and_then(Value::int).unwrap_or(0);
            if num_id == 0 {
                return None;
            }
            let ilvl = pp.get("numPr.ilvl").and_then(Value::int).unwrap_or(0);
            counters.item(&document.context.numbering, num_id, ilvl)
        })
        .collect()
}

/// Whether a flow item stops the box's text (an obstacle).
pub fn stopped(story: &StoryPage) -> bool {
    story.flow.items.iter().any(|i| matches!(i, FlowItem::Stop(_)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ooxml_word_model::model::Anchor;

    fn frame(height: Px, anchor: &str) -> TextBoxFrame {
        let body: Vec<(Text, BodyValue)> = vec![(Text::from("anchor"), BodyValue::Str(Text::from(anchor)))];
        TextBoxFrame {
            graphic: Arc::new(Graphic {
                body: body.into(),
                ..Graphic::default()
            }),
            x: Px::from(Q::new(1001, 2)),
            y: Px::int(600),
            width: Px::int(400),
            height,
            edges: [Q::from_integer(0); 4],
            colour: None,
        }
    }

    #[test]
    fn room_rounds_the_corner_to_twips_and_aligns_the_content() {
        let insets = [
            Q::from_integer(144),
            Q::from_integer(72),
            Q::from_integer(144),
            Q::from_integer(72),
        ];
        let content = Px::int(50);
        let top = room(&frame(Px::int(300), "t"), (&insets, content, true), "t");
        // 600 px is 2,880 twips; 72 twips in: 2,952 twips, 615 px in the nearest unit.
        let start = Px::int(615).quantise();
        assert_eq!(top.top, start);
        // 300 px less two insets of 72 twips (15 px each): a room of 270 px.
        assert_eq!(top.bottom, start + Px::int(270));
        // The corner 500.5 px is 2,402.4 twips, 2,402 to the nearest; 144 twips in.
        assert_eq!(top.left.round_half_up(), 530);
        let foot = room(&frame(Px::int(300), "b"), (&insets, content, true), "b");
        assert_eq!(foot.top, (start + Px::int(220)).quantise());
        assert_eq!(foot.bottom, foot.top + Px::int(270));
        let centred = room(&frame(Px::int(300), "ctr"), (&insets, content, true), "ctr");
        assert_eq!(centred.top, (start + Px::int(110)).quantise());
        // Taller than its room: from the top.
        let tall = room(&frame(Px::int(300), "b"), (&insets, Px::int(500), true), "b");
        assert_eq!(tall.top, start);
        // Below mode 15 the room is a twip shorter, truncated to the layout unit.
        let older = room(&frame(Px::int(300), "t"), (&insets, content, false), "t");
        assert!(older.bottom < top.bottom);
    }

    #[test]
    fn story_blocks_take_out_drawings_text_does_not_wrap_around() {
        let anchors: Vec<Anchor> = vec![
            Anchor {
                wrap: Some(Text::from("wrapNone")),
                ..Anchor::default()
            },
            Anchor {
                wrap: Some(Text::from("wrapSquare")),
                ..Anchor::default()
            },
        ];
        let run = Run {
            text: Text::from("ab"),
            breaks: vec![(0, Text::from("anchor:0")), (1, Text::from("anchor:1"))].into(),
            anchors: anchors.into(),
            ..Run::default()
        };
        let paragraph = Paragraph {
            path: Text::from("w:p[1]"),
            runs: vec![run].into(),
            ..Paragraph::default()
        };
        let (blocks, taken) = story_blocks(&[Block::Paragraph(Arc::new(paragraph))], "w:body/w:p[3]");
        assert!(taken);
        let Some(Block::Paragraph(p)) = blocks.first() else {
            panic!("a paragraph");
        };
        assert_eq!(&*p.path, "w:body/w:p[3]/w:p[1]");
        let marks: Vec<&str> = p.runs.iter().flat_map(|r| r.breaks.iter().map(|(_, m)| &**m)).collect();
        assert_eq!(marks, ["anchor:1"]);
        assert_eq!(
            unsupported_in(&blocks),
            Some("a floating drawing text wraps around in a text box")
        );
    }
}
