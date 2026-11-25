//! Owns: [`InsertShape`], a text box or a DrawingML shape anchored at a position, as
//! docx-agent's `_insert_shape` writes one (as Word's Insert Text Box and `make new shape`
//! do): in a run of its own there (formatted as text there would be, `w:noProof`, put
//! outside a content control it falls in), an `mc:AlternateContent` whose choice is the
//! `wps` drawing -- floating against the column and the paragraph, a step above every
//! other drawing, Word's insets, a text box's 0.5 pt black line, a shape's theme style --
//! and whose fallback is the VML shape where VML can draw it; its text a paragraph a line
//! (a shape's centred), each with its own paraId and textId, copied to the fallback.
//! Tracked, the run is an insertion and so is the text inside.
//!
//! Must not: write but through the sink.

use super::{all_heights, emu, height_above, points_text, DrawingEdited, A, EMU, POLYGON, SIDE_DISTANCE, V, W10, WPS};
use crate::controls::{rewrite, InParagraph, Rewrite};
use crate::marks::{insert_in_order, insert_paragraph_content};
use crate::names::{MC, W, W14, WP, WP14};
use crate::policy::RevisionPolicy;
use crate::ranges::renamed_id;
use crate::review::xml::make;
use crate::sink::{EditSink, TextWrite};
use crate::stamping::stamp_document;
use crate::text::atoms;
use crate::text::write::{check_text, make_run};
use crate::text_range::{range, range_id, Span};
use crate::work::{scope_above, Id, Work};
use crate::{EditError, Result, WordDoc};
use ooxml_base::Limits;
use ooxml_store::{Command, Txn};
use ooxml_word_model::docx::Docx;

/// VML's Office extensions.
const O: &str = "urn:schemas-microsoft-com:office:office";

/// A text box or a shape inserted at a position.
#[derive(Debug, Clone)]
pub struct InsertShape {
    /// Where: a position, or a range (at its start).
    pub at: String,
    /// A text box (`txBox`, Word's line and insets) rather than a shape.
    pub text_box: bool,
    /// The preset geometry (`rect` for a text box).
    pub preset: String,
    /// Its text, a paragraph a line (a text box always has one).
    pub text: Option<String>,
    /// Points.
    pub width: f64,
    /// Points.
    pub height: f64,
    /// Across from the column, points.
    pub x: f64,
    /// Down from the paragraph, points.
    pub y: f64,
    /// `square`, `tight`, `through`, `top_and_bottom`, `front`, `none` or `behind`.
    pub wrap: String,
    /// Untracked or tracked.
    pub policy: RevisionPolicy,
}

/// Where the shape goes.
struct Place {
    id: String,
    offset: usize,
}

impl InsertShape {
    /// The text's lines (`None`: a shape without text).
    fn lines(&self) -> Option<Vec<&str>> {
        let text = self.text.as_deref();
        match text {
            Some(t) if !t.is_empty() || self.text_box => Some(t.split('\n').collect()),
            None if self.text_box => Some(vec![""]),
            _ => None,
        }
    }

    fn check(&self, doc: &WordDoc, docx: &Docx) -> Result<Place> {
        if ooxml_drawingml::presets::preset(&self.preset).is_none() {
            return Err(EditError::Refused(format!(
                "{} is not a preset geometry",
                ooxml_base::pyfmt::repr_str(&self.preset)
            )));
        }
        for line in self.lines().unwrap_or_default() {
            check_text(line)?;
        }
        let found = range(doc, docx, &self.at)?;
        let span = Span::of(doc, docx, &found)?;
        let paragraph = span.first().ok_or_else(|| EditError::no_element(span.part.clone()))?;
        if crate::textbox::text_box_anchor(docx, &span.part, &paragraph.path).is_some() {
            return Err(EditError::Refused(
                "a text box or shape is not anchored in a text box".to_owned(),
            ));
        }
        let work = Work::of(&paragraph.element, Vec::new());
        if span.start > atoms(&work, work.root()).len() {
            return Err(EditError::Refused(format!(
                "{} is outside its paragraph's text",
                range_id(&found)
            )));
        }
        Ok(Place {
            id: paragraph.id.clone(),
            offset: span.start,
        })
    }
}

impl Command<WordDoc> for InsertShape {
    type Out = DrawingEdited;
    type Error = EditError;

    fn validate(&self, doc: &WordDoc) -> Result<()> {
        let docx = doc.docx()?;
        self.check(doc, &docx).map(|_| ())
    }

    fn prepare(&self, tx: &mut Txn<'_, WordDoc>) -> Result<DrawingEdited> {
        let mut sink = EditSink::new(tx, self.policy.clone());
        let out = self.edit(&mut sink)?;
        crate::after::after_edit(&mut sink)?;
        Ok(out)
    }
}

impl InsertShape {
    fn edit(&self, sink: &mut EditSink<'_, '_>) -> Result<DrawingEdited> {
        let docx = sink.docx()?;
        let place = self.check(sink.doc(), &docx)?;
        let renamed = stamp_document(sink, &docx)?;
        let heights = all_heights(sink)?;
        let docx = sink.docx()?;
        let id = renamed_id(&renamed, &place.id);
        let found = crate::set_text::find(sink.doc(), &docx, &id)?;
        let part = found.part.clone();
        let number = sink.draw_drawings(1)?.first().copied().unwrap_or(1);
        let ids = sink.draw(&format!("{part}\0shape"), 2)?;
        let lines = self.lines();
        let mut boxes = Vec::new();
        for _ in lines.iter().flatten() {
            boxes.push(sink.draw(&format!("{part}\0box"), 2)?);
        }
        let tree = sink
            .doc()
            .tree(&part)?
            .ok_or_else(|| EditError::no_part(part.clone()))?;
        let mut scope = scope_above(tree.root(), &found.path);
        scope.extend_from_slice(found.element.namespaces());
        let label = if self.text_box {
            format!("Text Box {number}")
        } else {
            format!("Shape {number}")
        };
        let frame = Frame {
            number,
            label: &label,
            height: height_above(&heights),
            anchor_id: ids.first().map_or("", String::as_str),
            edit_id: ids.get(1).map_or("", String::as_str),
            content: lines.is_some(),
        };
        let xml = self.xml(&frame);
        let parsed =
            ooxml_xml::parse(xml.as_bytes(), &Limits::default().xml).map_err(|e| EditError::Refused(e.to_string()))?;
        let holder = crate::namespaces::strip_redundant(parsed.root(), &mut scope);
        let at = InParagraph {
            part: part.clone(),
            path: found.path.clone(),
            id: found.id.clone(),
        };
        let how = Rewrite {
            renew: true,
            ensure_w14: false,
        };
        let offset = place.offset;
        let lines = lines.unwrap_or_default();
        rewrite(sink, &at, how, |sink, work| {
            let holder = work.adopt(&holder).ok_or_else(|| EditError::no_element(part.clone()))?;
            let wrote = self.place(sink, work, (holder, offset), &lines, &boxes)?;
            Ok(((), wrote))
        })?;
        // after the tracking's own declarations, as docx-agent declares them (with `wp14`)
        sink.ensure_w14(&part, true, &[("wps", WPS)])?;
        // docx-agent's created names the drawing only: the text box's paragraphs it looks
        // up are the ones it wrote out as text, not those in the document
        let created = vec![format!("d:{number}")];
        Ok(DrawingEdited {
            id: format!("d:{number}"),
            changed: true,
            created,
            renamed,
            warnings: Vec::new(),
        })
    }

    /// The run holding `holder` put at `offset` of the paragraph (`work`'s root), its text
    /// in the choice's text box, tracked when tracking, and copied to the fallback.
    fn place(
        &self,
        sink: &mut EditSink<'_, '_>,
        work: &mut Work,
        (holder, offset): (Id, usize),
        lines: &[&str],
        boxes: &[Vec<String>],
    ) -> Result<TextWrite> {
        let run = crate::inline::new_run(work, work.root(), offset, &[holder])?;
        crate::controls::hoist(work, run);
        let paragraphs = self.paragraphs(work, lines, boxes);
        let content = choice_content(work, holder);
        if let Some(content) = content {
            for p in &paragraphs {
                work.append(content, *p);
            }
        }
        no_proof(sink, work, run);
        let wrote = sink.track(work, |work, stamp, _| {
            if let (Some(parent), Some(index)) = (work.parent(run), work.index(run)) {
                work.remove(run);
                crate::track::insert_nodes(work, parent, index, &[run], stamp);
            }
            for p in &paragraphs {
                insert_paragraph_content(work, *p, stamp);
            }
            Vec::new()
        })?;
        if let (Some(content), Some(copy)) = (content, fallback_content(work, holder)) {
            for p in work.children(content).to_vec() {
                let made = work.deep_copy(p);
                work.append(copy, made);
            }
        }
        Ok(TextWrite { changed: true, ..wrote })
    }

    /// The text's paragraphs, made in `work` (detached): a shape's centred.
    fn paragraphs(&self, work: &mut Work, lines: &[&str], ids: &[Vec<String>]) -> Vec<Id> {
        let mut out = Vec::new();
        for (line, pair) in lines.iter().zip(ids) {
            let paragraph = make(work, "p", &[]);
            if !self.text_box {
                let properties = make(work, "pPr", &[]);
                let centred = make(work, "jc", &[("val", "center")]);
                work.append(properties, centred);
                work.append(paragraph, properties);
            }
            if !line.is_empty() {
                let run = make_run(work, line, None);
                work.append(paragraph, run);
            }
            for (local, value) in [("paraId", pair.first()), ("textId", pair.get(1))] {
                if let Some(value) = value {
                    let name = work.qname(W14, local, "w14");
                    work.set(paragraph, name, value);
                }
            }
            out.push(paragraph);
        }
        out
    }
}

/// `w:noProof` in the run's properties (made first when it has none).
fn no_proof(sink: &EditSink<'_, '_>, work: &mut Work, run: Id) {
    let shared = sink.doc().shared_handle();
    let properties = match work.find(run, W, "rPr") {
        Some(p) => p,
        None => {
            let p = make(work, "rPr", &[]);
            work.insert(run, 0, p);
            p
        }
    };
    let node = make(work, "noProof", &[]);
    insert_in_order(work, properties, node, shared.facts.orders());
}

fn first_below(work: &Work, node: Id, namespace: &str, local: &str) -> Option<Id> {
    work.descendants(node)
        .into_iter()
        .find(|n| work.is(*n, namespace, local))
}

/// The `w:txbxContent` of the holder's choice.
fn choice_content(work: &Work, holder: Id) -> Option<Id> {
    let choice = work.find(holder, MC, "Choice")?;
    first_below(work, choice, W, "txbxContent")
}

/// The `w:txbxContent` of the holder's fallback.
fn fallback_content(work: &Work, holder: Id) -> Option<Id> {
    let fallback = work.find(holder, MC, "Fallback")?;
    first_below(work, fallback, W, "txbxContent")
}

/// What the shape's XML takes besides the call's arguments.
struct Frame<'a> {
    number: u64,
    label: &'a str,
    height: i64,
    anchor_id: &'a str,
    edit_id: &'a str,
    /// Whether it holds text (an empty `w:txbxContent` is made for it).
    content: bool,
}

/// `text` as an XML attribute's value, double-quoted.
fn quoted(text: &str) -> String {
    let escaped = text
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;");
    format!("\"{escaped}\"")
}

impl InsertShape {
    /// The `mc:AlternateContent` Word writes (docx-agent's `_shape_xml`), its text boxes
    /// empty.
    fn xml(&self, frame: &Frame<'_>) -> String {
        let (cx, cy, x, y) = (emu(self.width), emu(self.height), emu(self.x), emu(self.y));
        let tag = super::settings::wrap_tag(&self.wrap);
        let behind = if self.wrap == "behind" { "1" } else { "0" };
        let wrap = match tag {
            "wrapNone" | "wrapTopAndBottom" => format!("<wp:{tag}/>"),
            "wrapSquare" => format!("<wp:{tag} wrapText=\"bothSides\"/>"),
            _ => {
                let points: String = POLYGON
                    .iter()
                    .enumerate()
                    .map(|(k, (px, py))| {
                        let name = if k == 0 { "start" } else { "lineTo" };
                        format!("<wp:{name} x=\"{px}\" y=\"{py}\"/>")
                    })
                    .collect();
                format!("<wp:{tag} wrapText=\"bothSides\"><wp:wrapPolygon edited=\"0\">{points}</wp:wrapPolygon></wp:{tag}>")
            }
        };
        let effect = if self.text_box { "0" } else { "12700" };
        let (properties, style, anchor) = if self.text_box {
            (
                "<a:noFill/><a:ln w=\"6350\"><a:solidFill><a:prstClr val=\"black\"/></a:solidFill></a:ln>",
                "",
                "t",
            )
        } else {
            (
                "",
                "<wps:style><a:lnRef idx=\"2\"><a:schemeClr val=\"accent1\"><a:shade val=\"15000\"/></a:schemeClr>\
                 </a:lnRef><a:fillRef idx=\"1\"><a:schemeClr val=\"accent1\"/></a:fillRef><a:effectRef idx=\"0\">\
                 <a:schemeClr val=\"accent1\"/></a:effectRef><a:fontRef idx=\"minor\"><a:schemeClr val=\"lt1\"/>\
                 </a:fontRef></wps:style>",
                "ctr",
            )
        };
        let content = if frame.content { "<w:txbxContent/>" } else { "" };
        let txbx = if frame.content {
            format!("<wps:txbx>{content}</wps:txbx>")
        } else {
            String::new()
        };
        let body = format!(
            "<wps:bodyPr rot=\"0\" spcFirstLastPara=\"0\" vertOverflow=\"overflow\" horzOverflow=\"overflow\" \
             vert=\"horz\" wrap=\"square\" lIns=\"91440\" tIns=\"45720\" rIns=\"91440\" bIns=\"45720\" numCol=\"1\" \
             spcCol=\"0\" rtlCol=\"0\" fromWordArt=\"0\" anchor=\"{anchor}\" anchorCtr=\"0\" forceAA=\"0\" \
             compatLnSpc=\"1\"><a:prstTxWarp prst=\"textNoShape\"><a:avLst/></a:prstTxWarp><a:noAutofit/></wps:bodyPr>"
        );
        let (number, label, height) = (frame.number, quoted(frame.label), frame.height);
        let text_box = if self.text_box { " txBox=\"1\"" } else { "" };
        let choice = format!(
            "<mc:Choice Requires=\"wps\"><w:drawing><wp:anchor distT=\"0\" distB=\"0\" distL=\"{SIDE_DISTANCE}\" \
             distR=\"{SIDE_DISTANCE}\" simplePos=\"0\" relativeHeight=\"{height}\" behindDoc=\"{behind}\" locked=\"0\" \
             layoutInCell=\"1\" allowOverlap=\"1\" wp14:anchorId=\"{a}\" wp14:editId=\"{e}\">\
             <wp:simplePos x=\"0\" y=\"0\"/>\
             <wp:positionH relativeFrom=\"column\"><wp:posOffset>{x}</wp:posOffset></wp:positionH>\
             <wp:positionV relativeFrom=\"paragraph\"><wp:posOffset>{y}</wp:posOffset></wp:positionV>\
             <wp:extent cx=\"{cx}\" cy=\"{cy}\"/><wp:effectExtent l=\"{effect}\" t=\"{effect}\" r=\"{effect}\" b=\"{effect}\"/>\
             {wrap}<wp:docPr id=\"{number}\" name={label}/><wp:cNvGraphicFramePr/>\
             <a:graphic><a:graphicData uri=\"{WPS}\"><wps:wsp><wps:cNvSpPr{text_box}/>\
             <wps:spPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"{cx}\" cy=\"{cy}\"/></a:xfrm>\
             <a:prstGeom prst=\"{preset}\"><a:avLst/></a:prstGeom>{properties}</wps:spPr>{style}{txbx}{body}\
             </wps:wsp></a:graphicData></a:graphic></wp:anchor></w:drawing></mc:Choice>",
            a = frame.anchor_id,
            e = frame.edit_id,
            preset = self.preset,
        );
        let fallback = self.fallback(frame, (cx, cy, x, y), tag, content);
        format!(
            "<mc:AlternateContent xmlns:mc=\"{MC}\" xmlns:w=\"{W}\" xmlns:wp=\"{WP}\" xmlns:wp14=\"{WP14}\" \
             xmlns:a=\"{A}\" xmlns:wps=\"{WPS}\" xmlns:v=\"{V}\" xmlns:o=\"{O}\" xmlns:w10=\"{W10}\" xmlns:w14=\"{W14}\">\
             {choice}{fallback}</mc:AlternateContent>"
        )
    }

    /// The VML fallback, where VML draws the preset (a text box always).
    fn fallback(&self, frame: &Frame<'_>, (cx, cy, x, y): (i64, i64, i64, i64), tag: &str, content: &str) -> String {
        let vml = if self.text_box {
            "shape"
        } else {
            match self.preset.as_str() {
                "rect" => "rect",
                "ellipse" => "oval",
                "roundRect" => "roundrect",
                _ => return String::new(),
            }
        };
        #[allow(clippy::cast_precision_loss)]
        let points = |v: i64| points_text(v as f64 / EMU);
        let css = format!(
            "position:absolute;margin-left:{};margin-top:{};width:{};height:{};z-index:{};\
             visibility:visible;mso-wrap-style:square;mso-wrap-distance-left:9pt;mso-wrap-distance-top:0;\
             mso-wrap-distance-right:9pt;mso-wrap-distance-bottom:0;mso-position-horizontal:absolute;\
             mso-position-horizontal-relative:text;mso-position-vertical:absolute;\
             mso-position-vertical-relative:text;v-text-anchor:{}",
            points(x),
            points(y),
            points(cx),
            points(cy),
            frame.height,
            if self.text_box { "top" } else { "middle" },
        );
        let textbox = if content.is_empty() {
            String::new()
        } else {
            format!("<v:textbox>{content}</v:textbox>")
        };
        let w10 = match tag {
            "wrapSquare" => "<w10:wrap type=\"square\"/>",
            "wrapTight" => "<w10:wrap type=\"tight\"/>",
            "wrapThrough" => "<w10:wrap type=\"through\"/>",
            "wrapTopAndBottom" => "<w10:wrap type=\"topAndBottom\"/>",
            _ => "",
        };
        let (label, spid) = (quoted(frame.label), 1024 + frame.number);
        let shape = if self.text_box {
            format!(
                "<v:shapetype w14:anchorId=\"{a}\" id=\"_x0000_t202\" coordsize=\"21600,21600\" o:spt=\"202\" \
                 path=\"m,l,21600r21600,l21600,xe\"><v:stroke joinstyle=\"miter\"/><v:path gradientshapeok=\"t\" \
                 o:connecttype=\"rect\"/></v:shapetype>\
                 <v:shape id={label} o:spid=\"_x0000_s{spid}\" type=\"#_x0000_t202\" style=\"{css}\" filled=\"f\" \
                 strokeweight=\".5pt\"><v:fill o:detectmouseclick=\"t\"/>{textbox}{w10}</v:shape>",
                a = frame.anchor_id,
            )
        } else {
            format!(
                "<v:{vml} w14:anchorId=\"{a}\" id={label} o:spid=\"_x0000_s{spid}\" style=\"{css}\" \
                 fillcolor=\"#4472c4 [3204]\" strokecolor=\"#09101d [484]\" strokeweight=\"1pt\">{textbox}{w10}</v:{vml}>",
                a = frame.anchor_id,
            )
        };
        format!("<mc:Fallback><w:pict>{shape}</w:pict></mc:Fallback>")
    }
}
