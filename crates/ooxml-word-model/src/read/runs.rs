//! Owns: a `w:r` as the runs it contributes (its drawn text and marks, less what lies in a
//! field's instruction or a computed field's cached result, and the computed field an `end`
//! in it closes), `w:fldSimple`, and the run-level containers a paragraph's runs are found
//! in (`w:hyperlink`, `w:ins`, `w:moveTo`, `w:sdt`...; a deletion and a move's source are
//! not drawn) -- docx2svg's `_runs`, `_simple_field` and `_inline`.
//!
//! Must not: read a paragraph's own properties (`paragraph`), or a drawing's content
//! (`drawing`).

use super::fields::{keyword, merge_format, Fields, Open, Part};
use super::{Reader, Steps};
use crate::model::{Anchor, Bookmark, Field, InlineObject, Run, RunProps};
use crate::value::{text, Text};
use crate::w;
use crate::xmlread::{attr, child, children, elements, local, text as element_text};
use ooxml_xml::Element;
use std::collections::HashMap;
use std::sync::Arc;

const RUN_MARKS: &[&str] = &["tab", "br", "cr", "noBreakHyphen", "softHyphen"];
const RUN_OBJECTS: &[&str] = &["drawing", "pict", "object", "AlternateContent"];
const RUN_CONTAINERS: &[&str] = &[
    "hyperlink",
    "ins",
    "moveTo",
    "smartTag",
    "customXml",
    "sdt",
    "dir",
    "bdo",
];

/// The run being gathered: text and marks since the last flush.
#[derive(Default)]
struct Pending {
    text: String,
    length: usize,
    marks: Vec<(usize, Text)>,
    drawings: Vec<InlineObject>,
    anchors: Vec<Anchor>,
    started: bool,
}

struct RunContext<'e> {
    properties: Option<Arc<RunProps>>,
    path: Text,
    out: Vec<Run>,
    pending: Pending,
    rpr: Option<&'e Element>,
}

impl RunContext<'_> {
    fn flush(&mut self) {
        let p = std::mem::take(&mut self.pending);
        if p.started || !p.marks.is_empty() {
            self.out.push(Run {
                text: text(&p.text),
                properties: self.properties.clone(),
                breaks: Arc::from(p.marks),
                path: self.path.clone(),
                drawings: Arc::from(p.drawings),
                anchors: Arc::from(p.anchors),
                field: None,
                toc_link: false,
            });
        }
    }

    fn mark(&mut self, mark: String) {
        self.pending.marks.push((self.pending.length, text(&mark)));
    }
}

fn custom(node: &Element) -> &'static str {
    if matches!(attr(node, "customMarkFollows"), Some("1" | "true" | "on")) {
        ":custom"
    } else {
        ""
    }
}

impl Reader<'_> {
    fn computed_run(&self, open: &Open, chosen: Option<Arc<RunProps>>, path: Text, instruction: &str) -> Run {
        Run {
            text: text(&open.result),
            properties: chosen,
            path,
            field: Some(Arc::new(Field {
                instruction: text(instruction.trim()),
                keyword: text(&keyword(instruction)),
                cached: Arc::from(open.cached.clone()),
            })),
            ..Run::default()
        }
    }

    fn field_char(&self, node: &Element, fields: &mut Fields, cx: &mut RunContext<'_>) {
        match w::FldChar::new(node).kind() {
            Some("begin") => {
                let visible = fields.visible();
                fields.stack.push(Open {
                    visible,
                    path: cx.path.clone(),
                    instruction: String::new(),
                    part: Part::Instruction,
                    computed: false,
                    instr: None,
                    result_properties: None,
                    result: String::new(),
                    cached: Vec::new(),
                });
            }
            Some("separate") => {
                let computed = fields.stack.last().is_some_and(|top| fields.computes(&top.instruction));
                if let Some(top) = fields.stack.last_mut() {
                    top.part = Part::Result;
                    top.computed = computed;
                }
            }
            Some("end") => {
                let Some(top) = fields.stack.pop() else { return };
                let instruction = top.instruction.clone();
                let computed = if top.part == Part::Result {
                    top.computed
                } else {
                    fields.computes(&instruction)
                };
                let drawn = top.visible && fields.visible();
                if drawn && !computed {
                    fields.note_cached(&instruction);
                }
                if drawn && computed {
                    cx.flush();
                    let (instr_props, instr_path) = top.instr.clone().unzip();
                    let mut chosen = instr_props.flatten();
                    if merge_format(&instruction) && top.result_properties.is_some() {
                        chosen = top.result_properties.clone();
                    }
                    let path = instr_path.filter(|p| !p.is_empty()).unwrap_or_else(|| top.path.clone());
                    let run = self.computed_run(&top, chosen, path, &instruction);
                    cx.out.push(run);
                }
            }
            _ => {}
        }
    }

    /// Text inside a computed field's result, gathered for it; anything else hidden.
    fn hidden(&self, node: &Element, name: &str, fields: &mut Fields, cx: &RunContext<'_>) {
        let outer_visible = fields
            .stack
            .split_last()
            .is_some_and(|(_, rest)| rest.iter().all(|f| f.part == Part::Result && !f.computed));
        let props = self.run_props(cx.rpr);
        let Some(top) = fields.stack.last_mut() else { return };
        if top.computed && top.part == Part::Result && outer_visible && matches!(name, "t" | "delText") {
            let piece = element_text(node);
            top.result.push_str(&piece);
            top.cached.push((text(&piece), props.clone()));
            if top.result_properties.is_none() {
                top.result_properties = Some(props.unwrap_or_default());
            }
        }
    }

    fn content(
        &self,
        node: &Element,
        name: &str,
        fields: &mut Fields,
        cx: &mut RunContext<'_>,
        seen: &mut HashMap<String, usize>,
    ) {
        match name {
            "t" | "delText" => {
                let piece = element_text(node);
                cx.pending.length += piece.chars().count();
                cx.pending.text.push_str(&piece);
                cx.pending.started = true;
            }
            _ if RUN_MARKS.contains(&name) => {
                let mark = attr(node, "type").filter(|t| !t.is_empty()).unwrap_or(name).to_owned();
                cx.mark(mark);
            }
            "footnoteReference" | "endnoteReference" => {
                let id = attr(node, "id").map_or_else(|| "None".to_owned(), str::to_owned);
                cx.mark(format!("{name}:{id}{}", custom(node)));
            }
            "endnoteRef" | "footnoteRef" | "separator" | "continuationSeparator" => cx.mark(name.to_owned()),
            _ if RUN_OBJECTS.contains(&name) => self.object(node, name, fields, cx, seen),
            _ => {}
        }
    }

    fn object(
        &self,
        node: &Element,
        name: &str,
        fields: &mut Fields,
        cx: &mut RunContext<'_>,
        seen: &mut HashMap<String, usize>,
    ) {
        let drawn_anchors = cx.pending.anchors.iter().filter(|a| !a.vml).count();
        let anchor_path = format!("{}/wp:anchor[{}]", cx.path, drawn_anchors + 1);
        if let Some(anchor) = self.anchor_of(node, &anchor_path, fields) {
            cx.mark(format!("anchor:{}", cx.pending.anchors.len()));
            cx.pending.anchors.push(anchor);
            return;
        }
        let count = seen.entry(name.to_owned()).or_insert(0);
        *count += 1;
        if super::drawing::drawing_element(node).is_none() {
            let prefix = if name == "AlternateContent" { "mc" } else { "w" };
            let vml_path = format!("{}/{prefix}:{name}[{count}]", cx.path);
            if let Some(found) = self.vml(node, &vml_path, fields) {
                match found {
                    super::vml::Vml::Anchor(anchor) => {
                        cx.mark(format!("anchor:{}", cx.pending.anchors.len()));
                        cx.pending.anchors.push(*anchor);
                    }
                    super::vml::Vml::Inline(mark, object) => {
                        cx.mark(mark);
                        cx.pending.drawings.push(object);
                    }
                }
                return;
            }
        }
        let mark = super::drawing::object_mark(node);
        let inline = mark.starts_with("drawing:");
        cx.mark(mark);
        if inline {
            let path = format!("{}/wp:inline[{}]", cx.path, cx.pending.drawings.len() + 1);
            let object = self.inline_object(node, &path, fields);
            cx.pending.drawings.push(object);
        }
    }

    /// The runs a `w:r` contributes.
    pub(crate) fn runs(&self, element: &Element, path: Text, fields: &mut Fields) -> Vec<Run> {
        let rpr = child(element, "rPr");
        let mut cx = RunContext {
            properties: self.run_props(rpr),
            path,
            out: Vec::new(),
            pending: Pending::default(),
            rpr,
        };
        let plain = fields.visible();
        let mut seen = HashMap::new();
        for node in elements(element) {
            let name = local(node);
            if name == "fldChar" {
                self.field_char(node, fields, &mut cx);
                continue;
            }
            if name == "instrText" {
                let props = self.run_props(cx.rpr);
                if let Some(top) = fields.stack.last_mut().filter(|t| t.part == Part::Instruction) {
                    top.instruction.push_str(&element_text(node));
                    if top.instr.is_none() {
                        top.instr = Some((props, cx.path.clone()));
                    }
                }
                continue;
            }
            if !fields.visible() {
                self.hidden(node, name, fields, &cx);
                continue;
            }
            self.content(node, name, fields, &mut cx, &mut seen);
        }
        cx.flush();
        let fielded = elements(element).any(|n| matches!(local(n), "fldChar" | "instrText"));
        if cx.out.is_empty() && plain && fields.visible() && !fielded {
            cx.out.push(Run {
                properties: cx.properties.clone(),
                path: cx.path.clone(),
                ..Run::default()
            });
        }
        cx.out
    }

    fn simple_field(&self, node: &Element, steps: &mut Steps, fields: &mut Fields) -> Vec<Run> {
        let here = steps.step(node);
        let mut inner_steps = Steps::new(&here);
        let instruction = w::FldSimple::new(node).instruction().unwrap_or("");
        let inner: Vec<&Element> = children(node, "r").collect();
        if fields.computes(instruction) {
            if !fields.visible() {
                return Vec::new();
            }
            let mut open = Open {
                visible: true,
                path: here.clone(),
                instruction: instruction.to_owned(),
                part: Part::Result,
                computed: true,
                instr: None,
                result_properties: None,
                result: String::new(),
                cached: Vec::new(),
            };
            for run in &inner {
                let props = self.run_props(child(run, "rPr"));
                for t in children(run, "t") {
                    let piece = element_text(t);
                    open.result.push_str(&piece);
                    open.cached.push((text(&piece), props.clone()));
                }
            }
            let chosen = match inner.first() {
                Some(first) if merge_format(instruction) => self.run_props(child(first, "rPr")),
                _ => None,
            };
            return vec![self.computed_run(&open, chosen, here, instruction)];
        }
        if fields.visible() {
            fields.note_cached(instruction);
        }
        let mut out = Vec::new();
        for run in inner {
            let path = inner_steps.step(run);
            out.extend(self.runs(run, path, fields));
        }
        out
    }

    fn bookmark(node: &Element, fields: &mut Fields, runs: usize) {
        let visible = fields.visible();
        if let Some(marks) = fields.bookmarks.as_mut().filter(|_| visible) {
            let view = w::Bookmark::new(node);
            marks.push(Bookmark {
                start: local(node) == "bookmarkStart",
                id: view.id().map(text),
                name: view.name().map(text),
                run: runs,
            });
        }
    }

    /// `element`'s runs into `runs`, through the run-level containers.
    pub(crate) fn inline(&self, element: &Element, steps: &mut Steps, fields: &mut Fields, runs: &mut Vec<Run>) {
        for node in elements(element) {
            let name = local(node);
            match name {
                "r" => {
                    let path = steps.step(node);
                    runs.extend(self.runs(node, path, fields));
                }
                "fldSimple" => runs.extend(self.simple_field(node, steps, fields)),
                "bookmarkStart" | "bookmarkEnd" => {
                    steps.step(node);
                    Self::bookmark(node, fields, runs.len());
                }
                _ if RUN_CONTAINERS.contains(&name) => {
                    let here = steps.step(node);
                    if name == "sdt" {
                        if let Some(content) = child(node, "sdtContent") {
                            self.inline(
                                content,
                                &mut Steps::new(&format!("{here}/w:sdtContent[1]")),
                                fields,
                                runs,
                            );
                        }
                    } else if name == "hyperlink" && fields.in_toc_result() {
                        let start = runs.len();
                        self.inline(node, &mut Steps::new(&here), fields, runs);
                        for run in runs.iter_mut().skip(start) {
                            run.toc_link = true;
                        }
                    } else {
                        self.inline(node, &mut Steps::new(&here), fields, runs);
                    }
                }
                _ => {
                    steps.step(node);
                }
            }
        }
    }
}
