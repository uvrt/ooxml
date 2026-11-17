//! Owns: a run's and a paragraph's effective formatting as docx-agent's `edit/effective.py`
//! asks it of docx2svg's resolver: the layout model's cascade (`crate::resolve`) given the
//! paragraph's style, its innermost table's style, the run's character style and its direct
//! properties -- bold and italic for the slot of its first character, strike, hidden, and a
//! paragraph's alignment.
//!
//! Must not: keep anything between versions, or know Markdown. A cell's table-style
//! regions are not given (they change a run and its paragraph's baseline alike, which is
//! what the Markdown reader compares).

use crate::document::Context;
use crate::docx::{find_path, Docx};
use crate::ids::W;
use crate::props::{paragraph_properties, run_properties};
use crate::resolve::{
    character_format, complex_script, resolve_paragraph, resolve_run, slot_for, ParagraphInput, RunInput, Sources,
};
use crate::value::Props;
use ooxml_xml::Element;

/// A run's formatting as the Markdown reader reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectiveRun {
    /// Bold, for its first character's slot.
    pub bold: bool,
    /// Italic, likewise.
    pub italic: bool,
    /// `w:strike`.
    pub strike: bool,
    /// `w:dstrike`.
    pub double_strike: bool,
    /// `w:vanish`.
    pub hidden: bool,
    /// Its character style's id.
    pub style: Option<String>,
}

/// What formatting resolves against, read once for a call.
pub struct Formatting {
    context: Context,
}

fn related_tree(docx: &Docx, kind: &str) -> Option<ooxml_xml::Tree> {
    let rel = format!("http://schemas.openxmlformats.org/officeDocument/2006/relationships/{kind}");
    let part = crate::parts::related(&docx.package, &docx.main, &rel).ok()??;
    docx.package.tree(&part).ok()?
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
        }
    }

    /// The numbering part as the layout model reads it (list labels count from it).
    #[must_use]
    pub fn numbering(&self) -> &crate::numbering::Numbering {
        &self.context.numbering
    }

    fn sources(&self) -> Sources<'_> {
        Sources {
            styles: self.context.styles.as_deref(),
            numbering: &self.context.numbering,
            settings: &self.context.settings,
        }
    }

    /// `run`'s formatting in `paragraph` (`None`: a run with no properties of its own --
    /// the paragraph's baseline), `first` its first character, `table_style` the innermost
    /// table's `w:tblStyle` (`Some("")` for a table naming none).
    #[must_use]
    pub fn run(
        &self,
        paragraph: &Element,
        run: Option<&Element>,
        first: char,
        table_style: Option<&str>,
    ) -> EffectiveRun {
        let colours = &self.context.colours;
        let properties = run.and_then(|r| r.find(Some(W), "rPr")).map(|e| &**e);
        let style = properties
            .and_then(|p| p.find(Some(W), "rStyle"))
            .and_then(|s| s.attribute(Some(W), "val"))
            .map(str::to_owned);
        let direct: Props = if properties.is_some() {
            run_properties(properties, colours)
        } else {
            Props::new()
        };
        let paragraph_style = find_path(paragraph, &["pPr", "pStyle"]).and_then(|s| s.attribute(Some(W), "val"));
        let input = RunInput {
            paragraph_style,
            table_style,
            conditions: &[],
            character_style: style.as_deref(),
            toc_link: false,
            direct: &direct,
        };
        let resolved = resolve_run(&self.sources(), &input);
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
        let truthy = |key: &str| resolved.get(key).is_some_and(crate::value::Value::truthy);
        EffectiveRun {
            bold: format.bold,
            italic: format.italic,
            strike: truthy("strike"),
            double_strike: truthy("dstrike"),
            hidden: truthy("vanish"),
            style,
        }
    }

    /// A paragraph's alignment as docx-agent names it (`left`, `center`, `right`,
    /// `justify`, `distribute`, or the value as written).
    #[must_use]
    pub fn alignment(&self, paragraph: &Element, table_style: Option<&str>) -> Option<String> {
        let direct = paragraph_properties(paragraph.find(Some(W), "pPr").map(|e| &**e), &self.context.colours);
        let style_id = find_path(paragraph, &["pPr", "pStyle"]).and_then(|s| s.attribute(Some(W), "val"));
        let input = ParagraphInput {
            style_id,
            table_style,
            conditions: &[],
            direct: &direct,
        };
        let resolved = resolve_paragraph(&self.sources(), &input);
        let jc = resolved.get("jc").and_then(|v| v.str().map(str::to_owned));
        let named = match jc.as_deref().unwrap_or("left") {
            "left" | "start" => Some("left"),
            "center" => Some("center"),
            "right" | "end" => Some("right"),
            "both" => Some("justify"),
            "distribute" => Some("distribute"),
            _ => None,
        };
        named.map(str::to_owned).or(jc)
    }
}
