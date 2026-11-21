//! Owns: a run's and a paragraph's effective formatting as docx-agent's `edit/effective.py`
//! asks it of docx2svg's resolver: the layout model's cascade (`crate::resolve`) given the
//! paragraph's style, its cell's table style and regions ([`Cell`], from the layout
//! model's reading of its table, [`super::cells`]), the run's character style and its
//! direct properties -- every field `word_inspect` shows, the face and size for the slot
//! of the run's first character.
//!
//! Resolutions are memoised for the version (design section 7: formatting resolved once per
//! style and property set): a run's by its paragraph style, cell, character style and the
//! content of its own properties; a paragraph's likewise.
//!
//! Must not: keep anything between versions, or know Markdown.

use crate::digest;
use crate::document::Context;
use crate::docx::{find_path, Docx};
use crate::ids::W;
use crate::props::{paragraph_properties, run_properties};
use crate::resolve::{
    character_format, complex_script, resolve_paragraph, resolve_paragraph_traced, resolve_run, resolve_run_traced,
    slot_for, Origins, ParagraphInput, RunInput, Sources,
};
use crate::value::{Props, Text, Value};
use ooxml_base::{Digest, DigestKey};
use ooxml_xml::Element;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

/// The memo's digests: a content key within one process, not a secret.
const KEY: DigestKey = DigestKey::new(0x5031_6272_6561_6473, 0x666f_726d_6174_7321);

/// Where a paragraph stands in a table: the innermost table's style (`""`: none named) and
/// its cell's regions, as the layout model reads them.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct Cell {
    /// The table style.
    pub style: Text,
    /// The cell's regions (band positions unnamed).
    pub conditions: Arc<[Text]>,
}

/// A run's formatting as Word applies it (docx-agent's `EffectiveRun`).
#[derive(Debug, Clone, PartialEq)]
pub struct EffectiveRun {
    /// Bold, for its first character's slot.
    pub bold: bool,
    /// Italic, likewise.
    pub italic: bool,
    /// `w:u` (`None` for none).
    pub underline: Option<String>,
    /// `w:strike`.
    pub strike: bool,
    /// `w:dstrike`.
    pub double_strike: bool,
    /// `w:caps`.
    pub caps: bool,
    /// `w:smallCaps`.
    pub small_caps: bool,
    /// Points.
    pub size: f64,
    /// The face its first character is drawn in.
    pub font: Option<String>,
    /// `RRGGBB` or `auto`.
    pub color: String,
    /// `w:highlight`.
    pub highlight: Option<String>,
    /// `w:vertAlign`.
    pub vertical_align: Option<String>,
    /// `w:vanish`.
    pub hidden: bool,
    /// Its character style's id.
    pub style: Option<String>,
    /// What the cascade resolved (a cell's text colour reads its shading).
    pub resolved: Arc<Props>,
}

/// A paragraph's formatting as Word applies it (docx-agent's `EffectiveParagraph`).
#[derive(Debug, Clone, PartialEq)]
pub struct EffectiveParagraph {
    /// `left`, `center`, `right`, `justify`, `distribute`, or `w:jc` as written.
    pub alignment: Option<String>,
    /// Points.
    pub indent_left: f64,
    /// Points.
    pub indent_right: f64,
    /// Points.
    pub first_line: f64,
    /// Points.
    pub hanging: f64,
    /// Points.
    pub space_before: f64,
    /// Points.
    pub space_after: f64,
    /// A multiple (`auto`) or points.
    pub line_spacing: Option<f64>,
    /// `auto`, `exact`, `atLeast`.
    pub line_rule: String,
    /// `w:keepNext`.
    pub keep_with_next: bool,
    /// `w:keepLines`.
    pub keep_together: bool,
    /// `w:pageBreakBefore`.
    pub page_break_before: bool,
    /// `w:outlineLvl`.
    pub outline_level: Option<i64>,
    /// What the cascade resolved.
    pub resolved: Arc<Props>,
}

/// What a paragraph of a style gets with no direct formatting, and a run in it with
/// none, each value with where it came from ([`Formatting::style_look`]).
#[derive(Debug, Clone)]
pub struct StyleLook {
    /// The paragraph's formatting.
    pub paragraph: EffectiveParagraph,
    /// Where each of its resolved values came from.
    pub paragraph_origins: Origins,
    /// The run's.
    pub run: EffectiveRun,
    /// Where each of its resolved values came from.
    pub run_origins: Origins,
}

type RunKey = (Option<String>, Option<Cell>, Option<String>, Digest);
type ParagraphKey = (Option<String>, Option<Cell>, Digest);

/// What formatting resolves against, read once for a version, with its memos.
pub struct Formatting {
    context: Context,
    runs: Mutex<HashMap<RunKey, Arc<Props>>>,
    paragraphs: Mutex<HashMap<ParagraphKey, Arc<Props>>>,
}

fn related_tree(docx: &Docx, kind: &str) -> Option<ooxml_xml::Tree> {
    let rel = format!("http://schemas.openxmlformats.org/officeDocument/2006/relationships/{kind}");
    let part = crate::parts::related(&docx.package, &docx.main, &rel).ok()??;
    docx.package.tree(&part).ok()?
}

fn element_key(element: Option<&Element>) -> Digest {
    element.map_or_else(Digest::default, |e| digest::element(KEY, e))
}

fn paragraph_style(paragraph: &Element) -> Option<String> {
    find_path(paragraph, &["pPr", "pStyle"])
        .and_then(|s| s.attribute(Some(W), "val"))
        .map(str::to_owned)
}

impl Formatting {
    /// The styles, numbering, settings and theme of `docx`.
    #[must_use]
    pub fn read(docx: &Docx) -> Formatting {
        let theme = related_tree(docx, "theme");
        let settings = related_tree(docx, "settings");
        let styles = related_tree(docx, "styles");
        let numbering = related_tree(docx, "numbering");
        Formatting {
            context: crate::read_context(theme.as_ref(), settings.as_ref(), styles.as_ref(), numbering.as_ref()),
            runs: Mutex::new(HashMap::new()),
            paragraphs: Mutex::new(HashMap::new()),
        }
    }

    /// What blocks resolve against (styles, numbering, settings, theme).
    #[must_use]
    pub fn context(&self) -> &Context {
        &self.context
    }

    /// The numbering part as the layout model reads it (list labels count from it).
    #[must_use]
    pub fn numbering(&self) -> &crate::numbering::Numbering {
        &self.context.numbering
    }

    /// The sources the cascade reads.
    #[must_use]
    pub fn sources(&self) -> Sources<'_> {
        Sources {
            styles: self.context.styles.as_deref(),
            numbering: &self.context.numbering,
            settings: &self.context.settings,
        }
    }

    /// `run`'s formatting in `paragraph` (`None`: a run with no properties of its own --
    /// the paragraph's baseline), `first` its first character, `cell` where the paragraph
    /// stands in a table.
    #[must_use]
    pub fn run(&self, paragraph: &Element, run: Option<&Element>, first: char, cell: Option<&Cell>) -> EffectiveRun {
        let properties = run.and_then(|r| r.find(Some(W), "rPr")).map(|e| &**e);
        self.run_properties(paragraph, properties, first, cell)
    }

    /// Formatting of a run whose `w:rPr` is `properties`, in `paragraph`.
    #[must_use]
    pub fn run_properties(
        &self,
        paragraph: &Element,
        properties: Option<&Element>,
        first: char,
        cell: Option<&Cell>,
    ) -> EffectiveRun {
        let style = properties
            .and_then(|p| p.find(Some(W), "rStyle"))
            .and_then(|s| s.attribute(Some(W), "val"))
            .map(str::to_owned);
        self.resolved_run(paragraph, properties, style, first, cell)
    }

    /// What a run in the character style `style` gets in `paragraph` with none of its
    /// direct formatting: what its styles give it (docx-agent's `effective.run`, `direct`
    /// false).
    #[must_use]
    pub fn run_styled(&self, paragraph: &Element, style: Option<&str>, first: char, cell: Option<&Cell>) -> EffectiveRun {
        self.resolved_run(paragraph, None, style.map(str::to_owned), first, cell)
    }

    /// A run's formatting from its direct `properties` (`None`: none) and its character
    /// `style`, memoised by both.
    fn resolved_run(
        &self,
        paragraph: &Element,
        properties: Option<&Element>,
        style: Option<String>,
        first: char,
        cell: Option<&Cell>,
    ) -> EffectiveRun {
        let pstyle = paragraph_style(paragraph);
        let key: RunKey = (pstyle.clone(), cell.cloned(), style.clone(), element_key(properties));
        let known = self
            .runs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&key)
            .cloned();
        let resolved = known.unwrap_or_else(|| {
            let direct: Props = if properties.is_some() {
                run_properties(properties, &self.context.colours)
            } else {
                Props::new()
            };
            let input = RunInput {
                paragraph_style: pstyle.as_deref(),
                table_style: cell.map(|c| &*c.style),
                conditions: cell.map_or(&[], |c| &c.conditions),
                character_style: style.as_deref(),
                toc_link: false,
                direct: &direct,
            };
            let made = Arc::new(resolve_run(&self.sources(), &input));
            self.runs
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .insert(key, Arc::clone(&made));
            made
        });
        self.effective_run(resolved, first, style)
    }

    fn effective_run(&self, resolved: Arc<Props>, first: char, style: Option<String>) -> EffectiveRun {
        let text_of = |key: &str| resolved.get(key).and_then(|v| v.str().map(str::to_owned));
        let slot = slot_for(
            first,
            complex_script(&resolved),
            text_of("rFonts.hint").as_deref(),
            text_of("lang.eastAsia").as_deref(),
        );
        let format = character_format(
            &resolved,
            slot,
            self.context.font_scheme.as_deref(),
            &self.context.settings,
        );
        let truthy = |key: &str| resolved.get(key).is_some_and(Value::truthy);
        let underline = text_of("u").filter(|u| u != "none");
        EffectiveRun {
            bold: format.bold,
            italic: format.italic,
            underline,
            strike: truthy("strike"),
            double_strike: truthy("dstrike"),
            caps: truthy("caps"),
            small_caps: truthy("smallCaps"),
            size: format.half_points as f64 / 2.0,
            font: Some(format.face.to_string()),
            color: text_of("color").unwrap_or_else(|| "auto".to_owned()),
            highlight: text_of("highlight"),
            vertical_align: text_of("vertAlign"),
            hidden: truthy("vanish"),
            style,
            resolved,
        }
    }

    /// A paragraph's formatting, `cell` where it stands in a table.
    #[must_use]
    pub fn paragraph(&self, paragraph: &Element, cell: Option<&Cell>) -> EffectiveParagraph {
        let properties = paragraph.find(Some(W), "pPr").map(|e| &**e);
        let style = paragraph_style(paragraph);
        let key: ParagraphKey = (style.clone(), cell.cloned(), element_key(properties));
        let known = self
            .paragraphs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&key)
            .cloned();
        let resolved = known.unwrap_or_else(|| {
            let direct = paragraph_properties(properties, &self.context.colours);
            let input = ParagraphInput {
                style_id: style.as_deref(),
                table_style: cell.map(|c| &*c.style),
                conditions: cell.map_or(&[], |c| &c.conditions),
                direct: &direct,
            };
            let made = Arc::new(resolve_paragraph(&self.sources(), &input));
            self.paragraphs
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .insert(key, Arc::clone(&made));
            made
        });
        effective_paragraph(resolved)
    }

    /// A style's look after inheritance (docx-agent's `effective.style_look`): what a
    /// paragraph of the paragraph style `style` (`None`: the default) with no direct
    /// formatting gets, in `cell` where given, and a run in it with none, in the character
    /// style `character_style` where given -- with where each value came from. Not memoised:
    /// a design reading asks it once per style and place.
    #[must_use]
    pub fn style_look(&self, style: Option<&str>, cell: Option<&Cell>, character_style: Option<&str>) -> StyleLook {
        let direct = Props::new();
        let sources = self.sources();
        let (paragraph, paragraph_origins) = resolve_paragraph_traced::<Origins>(
            &sources,
            &ParagraphInput {
                style_id: style,
                table_style: cell.map(|c| &*c.style),
                conditions: cell.map_or(&[], |c| &c.conditions),
                direct: &direct,
            },
        );
        let (run, run_origins) = resolve_run_traced::<Origins>(
            &sources,
            &RunInput {
                paragraph_style: style,
                table_style: cell.map(|c| &*c.style),
                conditions: cell.map_or(&[], |c| &c.conditions),
                character_style,
                toc_link: false,
                direct: &direct,
            },
        );
        StyleLook {
            paragraph: effective_paragraph(Arc::new(paragraph)),
            paragraph_origins,
            run: self.effective_run(Arc::new(run), 'a', character_style.map(str::to_owned)),
            run_origins,
        }
    }

    /// A paragraph's alignment as docx-agent names it.
    #[must_use]
    pub fn alignment(&self, paragraph: &Element, cell: Option<&Cell>) -> Option<String> {
        self.paragraph(paragraph, cell).alignment
    }
}

fn effective_paragraph(resolved: Arc<Props>) -> EffectiveParagraph {
    let int = |key: &str| resolved.get(key).and_then(Value::int);
    let points = |key: &str| int(key).unwrap_or(0) as f64 / 20.0;
    let rule = resolved
        .get("spacing.lineRule")
        .and_then(Value::str)
        .map_or_else(|| "auto".to_owned(), str::to_owned);
    let line = int("spacing.line").map(|l| l as f64 / if rule == "auto" { 240.0 } else { 20.0 });
    let truthy = |key: &str| resolved.get(key).is_some_and(Value::truthy);
    let jc = resolved.get("jc").and_then(Value::str).map(str::to_owned);
    let named = match jc.as_deref().unwrap_or("left") {
        "left" | "start" => Some("left"),
        "center" => Some("center"),
        "right" | "end" => Some("right"),
        "both" => Some("justify"),
        "distribute" => Some("distribute"),
        _ => None,
    };
    EffectiveParagraph {
        alignment: named.map(str::to_owned).or(jc),
        indent_left: points("ind.left"),
        indent_right: points("ind.right"),
        first_line: points("ind.firstLine"),
        hanging: points("ind.hanging"),
        space_before: points("spacing.before"),
        space_after: points("spacing.after"),
        line_spacing: line,
        line_rule: rule,
        keep_with_next: truthy("keepNext"),
        keep_together: truthy("keepLines"),
        page_break_before: truthy("pageBreakBefore"),
        outline_level: int("outlineLvl"),
        resolved,
    }
}
