//! Owns: what the text engine is given for one paragraph of the Word model -- its runs with
//! every character's format by its slot, its marks, its list label, its hyphenation
//! ([`text_paragraph`]) -- and the geometry its lines are broken at ([`geometry`]):
//! docx2svg's former `linebreak.pieces`' and `label_pieces`' inputs and
//! `linebreak.geometry`.
//!
//! Must not: measure or break lines (`ooxml-text` does), resolve styles (the model has),
//! or know where the paragraph is placed.

use crate::sections::SectionGeometry;
use ooxml_text::{
    twips, BreakKind, CharFormat, FaceKey, Geometry, Hyphenation, Label, LabelAlign, LabelBody, LabelSuffix, Mark,
    Paragraph as TextParagraph, Run as TextRun, RunFormats, TabAlign, Units, VerticalAlign, WordRules,
};
use ooxml_word_model::model::{Paragraph, Run};
use ooxml_word_model::resolve::{character_format, complex_script, slot_for, CharacterFormat, Slot};
use ooxml_word_model::value::{Props, Value};
use ooxml_word_model::{ResolvedParagraph, WordDocument};
use std::collections::HashMap;
use std::sync::Arc;

/// A run's characters' formats, found once per slot (a run's characters of one slot share
/// one format; docx2svg's `character_format` keeps them the same way).
pub struct RunFormatter<'a> {
    document: &'a WordDocument,
    resolved: &'a Props,
    complex: bool,
    hint: Option<&'a str>,
    east_asian: Option<&'a str>,
    by_slot: HashMap<Slot, CharacterFormat>,
}

impl<'a> RunFormatter<'a> {
    /// The formatter of a run whose properties are `resolved`.
    pub fn new(document: &'a WordDocument, resolved: &'a Props) -> RunFormatter<'a> {
        RunFormatter {
            document,
            resolved,
            complex: complex_script(resolved),
            hint: resolved.get("rFonts.hint").and_then(Value::str),
            east_asian: resolved.get("lang.eastAsia").and_then(Value::str),
            by_slot: HashMap::new(),
        }
    }

    /// The format `ch` is measured in.
    pub fn format(&mut self, ch: char) -> &CharacterFormat {
        let slot = slot_for(ch, self.complex, self.hint, self.east_asian);
        let (document, resolved) = (self.document, self.resolved);
        self.by_slot.entry(slot).or_insert_with(|| {
            character_format(
                resolved,
                slot,
                document.context.font_scheme.as_deref(),
                &document.context.settings,
            )
        })
    }
}

/// The text engine's format of a model's character format.
pub fn char_format(format: &CharacterFormat) -> CharFormat {
    CharFormat {
        face: Some(FaceKey {
            family: Arc::from(&*format.face),
            bold: format.bold,
            italic: format.italic,
        }),
        half_points: u32::try_from(format.half_points).unwrap_or(0),
        vertical_align: match format.vertical_align.as_deref() {
            Some("superscript") => Some(VerticalAlign::Superscript),
            Some("subscript") => Some(VerticalAlign::Subscript),
            _ => None,
        },
    }
}

/// A run mark (`Run::breaks`) as the text engine reads it.
pub fn mark(kind: &str) -> Mark {
    match kind {
        "tab" => Mark::Tab,
        "br" | "textWrapping" | "cr" => Mark::Break(BreakKind::Line),
        "page" => Mark::Break(BreakKind::Page),
        "column" => Mark::Break(BreakKind::Column),
        "softHyphen" => Mark::SoftHyphen,
        "noBreakHyphen" => Mark::NoBreakHyphen,
        "drawing" => Mark::UnmeasuredDrawing,
        "separator" => Mark::Separator,
        "continuationSeparator" => Mark::ContinuationSeparator,
        other => match other.strip_prefix("drawing:") {
            Some(rest) => {
                let v: Vec<i64> = rest.split(':').map(|s| s.parse().unwrap_or_default()).collect();
                let at = |k: usize| v.get(k).copied().unwrap_or_default();
                Mark::Drawing {
                    cx: at(0),
                    cy: at(1),
                    effect: [at(2), at(3), at(4), at(5)],
                }
            }
            None => Mark::Other,
        },
    }
}

fn int(props: &Props, key: &str) -> Option<i64> {
    props.get(key).and_then(Value::int)
}

fn flag(props: &Props, key: &str) -> bool {
    props.get(key).is_some_and(Value::truthy)
}

/// One run as the text engine reads it.
pub fn text_run(document: &WordDocument, run: &Run, resolved: &Props) -> TextRun {
    let marks = run.breaks.iter().map(|(at, kind)| (*at, mark(kind))).collect();
    let blank = CharFormat {
        face: None,
        half_points: 0,
        vertical_align: None,
    };
    if flag(resolved, "vanish") {
        return TextRun {
            text: run.text.to_string(),
            marks,
            vanish: true,
            formats: RunFormats::Uniform(blank.clone()),
            hyphen_format: blank,
            tracking: 0,
            scale: 100,
            kern: 0,
            caps: false,
            note_number: false,
            lang: None,
            no_proof: false,
        };
    }
    let mut formatter = RunFormatter::new(document, resolved);
    let mut formats: Vec<CharFormat> = run.text.chars().map(|c| char_format(formatter.format(c))).collect();
    // One format for every character where there is exactly one distinct format (as the
    // differential's records write it); an empty run's list stays empty.
    let uniform = !formats.is_empty() && formats.windows(2).all(|w| w.first() == w.get(1));
    let formats = match formats.pop() {
        Some(one) if uniform => RunFormats::Uniform(one),
        Some(last) => {
            formats.push(last);
            RunFormats::PerChar(formats)
        }
        None => RunFormats::PerChar(formats),
    };
    let small = |key: &str, default: i64| int(resolved, key).filter(|v| *v != 0).unwrap_or(default);
    TextRun {
        text: run.text.to_string(),
        marks,
        vanish: false,
        formats,
        hyphen_format: char_format(formatter.format('-')),
        tracking: i32::try_from(small("spacing", 0)).unwrap_or(0),
        scale: u32::try_from(small("w", 100)).unwrap_or(100),
        kern: u32::try_from(small("kern", 0)).unwrap_or(0),
        caps: flag(resolved, "caps"),
        note_number: run.note_number,
        lang: resolved.get("lang.val").and_then(Value::str).map(str::to_owned),
        no_proof: flag(resolved, "noProof"),
    }
}

/// The list level a resolved paragraph is an item of, if any.
pub fn list_level<'a>(document: &'a WordDocument, pp: &Props) -> Option<&'a Arc<ooxml_word_model::numbering::Level>> {
    let num_id = int(pp, "numPr.numId").filter(|n| *n != 0)?;
    let ilvl = int(pp, "numPr.ilvl").unwrap_or(0);
    document.context.numbering.level(num_id, ilvl)
}

/// A list label's format: the mark's properties with the level's `w:rPr` on top.
pub fn label_props(mark: &Props, level: &ooxml_word_model::numbering::Level) -> Props {
    let mut values = mark.clone();
    values.extend(level.run.iter().map(|(k, v)| (k.clone(), v.clone())));
    values
}

/// The text engine's label: `text` (its numbers substituted) in the level's format.
pub fn text_label(document: &WordDocument, resolved: &ResolvedParagraph, text: &str) -> Option<Label> {
    let level = list_level(document, &resolved.paragraph)?;
    let props = label_props(&resolved.mark, level);
    let mut formatter = RunFormatter::new(document, &props);
    let space = char_format(formatter.format(' '));
    let body = match level.picture_bullet {
        Some(id) => match document.picture_bullets.get(&id).and_then(|b| b.pixels) {
            Some((_, height)) => LabelBody::Picture {
                half_points: space.half_points,
                pixel_height: height,
            },
            None => LabelBody::UnreadPicture,
        },
        None => LabelBody::Text(text.chars().map(|c| (c, char_format(formatter.format(c)))).collect()),
    };
    Some(Label {
        body,
        caps: flag(&props, "caps"),
        alignment: match level.alignment.as_deref() {
            Some("center") => LabelAlign::Center,
            Some("right" | "end") => LabelAlign::Right,
            _ => LabelAlign::Left,
        },
        suffix: match level.suffix.as_deref().unwrap_or("tab") {
            "tab" => LabelSuffix::Tab,
            "space" => LabelSuffix::Space,
            _ => LabelSuffix::Nothing,
        },
        space_format: space,
    })
}

/// The document's line rules (its compatibility mode).
pub fn rules(document: &WordDocument) -> WordRules {
    WordRules {
        compatibility_mode: u32::try_from(document.context.settings.compatibility_mode.unwrap_or(0)).unwrap_or(0),
    }
}

/// The paragraph as the text engine measures it: `label` its list label's text.
pub fn text_paragraph(
    document: &WordDocument,
    paragraph: &Paragraph,
    resolved: &ResolvedParagraph,
    label: Option<&str>,
) -> TextParagraph {
    let runs = paragraph
        .runs
        .iter()
        .zip(resolved.runs.iter())
        .map(|(run, props)| text_run(document, run, props))
        .collect();
    let hyphenation = document
        .context
        .settings
        .hyphenation
        .as_ref()
        .filter(|_| !flag(&resolved.paragraph, "suppressAutoHyphens"))
        .map(|h| Hyphenation {
            mode15: rules(document).mode15(),
            no_caps: h.no_caps,
        });
    TextParagraph {
        runs,
        label: label.and_then(|text| text_label(document, resolved, text)),
        hyphenation,
    }
}

/// `linebreak.twips_to_units` of a twip length.
fn units(value: i64) -> Units {
    twips(value)
}

/// Where the paragraph's lines start and end in a column `width` twips wide
/// (`linebreak.geometry`).
pub fn geometry(document: &WordDocument, pp: &Props, width: i64) -> Geometry {
    let left = int(pp, "ind.left").unwrap_or(0);
    let right = int(pp, "ind.right").unwrap_or(0);
    let first = int(pp, "ind.firstLine").unwrap_or(0);
    let hanging = int(pp, "ind.hanging").unwrap_or(0);
    let tabs = match pp.get("tabs") {
        Some(Value::Tabs(stops)) => stops
            .iter()
            .filter(|s| !matches!(&*s.alignment, "clear" | "bar"))
            .map(|s| {
                (
                    units(s.position),
                    TabAlign::parse(&s.alignment).unwrap_or(TabAlign::Left),
                )
            })
            .collect(),
        _ => Vec::new(),
    };
    let rules = rules(document);
    let settings = &document.context.settings;
    let jc = pp.get("jc").and_then(Value::str);
    let hyphen = settings
        .hyphenation
        .as_ref()
        .filter(|_| !flag(pp, "suppressAutoHyphens"))
        .map(|h| {
            rules.hyphenation(
                h.zone,
                u32::try_from(h.limit).unwrap_or(0),
                matches!(jc, Some("both" | "distribute")),
            )
        })
        .unwrap_or_default();
    Geometry {
        first_start: units(left + first - hanging),
        start: units(left),
        right: units(width - right),
        tabs,
        default_tab: rules.default_tab(settings.default_tab_stop),
        hanging_stop: (hanging != 0).then(|| units(left)),
        squeeze: rules.squeeze(jc == Some("both")),
        tab_origin: Units::ZERO,
        hyphen_zone: hyphen.zone,
        hyphen_next: hyphen.next,
        hyphen_gap: hyphen.gap,
        hyphen_limit: hyphen.limit,
    }
}

/// [`geometry`] in its section's first column.
pub fn section_geometry(document: &WordDocument, pp: &Props, section: &SectionGeometry) -> Geometry {
    geometry(document, pp, section.column_width(0))
}
