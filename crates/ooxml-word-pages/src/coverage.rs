//! Owns: how much of a document a version laid out covers, as docx2svg's Rust layout says
//! it (`docx2svg.rust.RustLayout._coverage_and_warnings`, its `Coverage`): the faces laid
//! out with a substitute (in the order the body's lines draw them where two or more are,
//! `coverage.in_layout_order`) and the ones missing, the places laid out by a rule no probe
//! measured (the layout's approximations, then each glyph measured with another face), and
//! every warning in docx2svg's order -- the model's reading warnings, the substituted faces,
//! the glyph fallbacks grouped by face, then what the layout said ([`crate::warnings`]).
//!
//! A version laid out here is never partial in docx2svg's sense but by a missing face: a
//! layout that stops is a fallback ([`crate::Fallback`]), and nothing is drawn as a
//! placeholder.
//!
//! Must not: lay out, or word the report as a tool does (`check` compacts it).

use crate::engine::PageEngine;
use crate::version::LaidVersion;
use crate::warnings::{Approximation, LayoutWarning};
use ooxml_fonts::{Grade, SubstitutionKind};
use ooxml_word_model::value::Text;

/// What docx2svg calls a recorded face's substitute (`fonts.RECORDED`).
pub const RECORDED: &str = "recorded metrics";

/// docx2svg's message for a reading warning it names (`_PARSER_WARNINGS`), else its default.
fn reading_message(code: &str) -> &'static str {
    match code {
        "styles-part-missing" => "the package has no styles part; Word's application defaults apply",
        "document-no-body" => "the document has no body",
        _ => "a body element is not read",
    }
}

/// A face laid out with another (docx2svg's `Substitution`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Substituted {
    /// The family the document names.
    pub family: String,
    /// The face laid out in its place ([`RECORDED`] for a recorded face).
    pub substitute: String,
    /// The two measured to the same advances and line metrics.
    pub metric_compatible: bool,
    /// How the substitute was chosen: `named`, `default` or `generic`.
    pub kind: &'static str,
    /// For a generic one, the class it was chosen by.
    pub class: Option<&'static str>,
}

impl Substituted {
    /// docx2svg's `Substitution.message`.
    #[must_use]
    pub fn message(&self) -> String {
        let family = repr(&self.family);
        let substitute = repr(&self.substitute);
        if self.substitute == RECORDED {
            return format!(
                "{family} is not installed: laid out with the metrics recorded from Word's copy \
                 (docx2svg.recorded: the same advances and line metrics), and its symbols drawn as their \
                 Unicode equivalents in a generic face"
            );
        }
        if self.metric_compatible {
            return format!(
                "{family} is not installed: laid out and drawn with {substitute}, its open metric \
                 compatible substitute (the same advances and line metrics; outlines, underline geometry \
                 and kerning may differ)"
            );
        }
        if self.kind == "default" || self.kind == "generic" {
            let why = if self.kind == "default" {
                "the face PowerPoint rendering uses for it".to_owned()
            } else {
                format!("the first installed {} face", self.class.unwrap_or("sans-serif"))
            };
            return format!(
                "{family} is not installed and no substitute was named: laid out and drawn with \
                 {substitute}, {why}, as an approximation (its advances and line metrics are not the absent \
                 face's: line breaks and page breaks may differ; name a substitute in \
                 ConvertOptions.font_substitutes, or install the face, to change it)"
            );
        }
        format!(
            "{family} is not installed: laid out and drawn with {substitute}, an approximate substitute \
             the caller named (its advances and line metrics are not Word's: line breaks and page breaks \
             may differ)"
        )
    }
}

/// How much of a document a version covers (docx2svg's `Coverage` of a Rust layout).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Coverage {
    /// No face is missing.
    pub complete: bool,
    /// Its pages (all laid out).
    pub pages: usize,
    /// The body's top-level blocks (all laid out).
    pub blocks: usize,
    /// The faces laid out with another, in docx2svg's order.
    pub substituted: Vec<Substituted>,
    /// The faces found nowhere, sorted.
    pub missing: Vec<String>,
    /// Every place laid out by a rule no probe measured.
    pub approximations: Vec<Approximation>,
    /// Every warning, in docx2svg's order.
    pub warnings: Vec<LayoutWarning>,
}

fn repr(text: &str) -> String {
    ooxml_base::pyfmt::repr_str(text)
}

/// What a glyph measured with `used` was measured with, as docx2svg says it.
fn measured_with(used: &str, missing: &'static str) -> String {
    if used == ".notdef" {
        missing.to_owned()
    } else {
        repr(used)
    }
}

/// The faces of `found` laid out with another, in the order first looked up, and those
/// missing, sorted.
fn graded(found: &ooxml_word_layout::faces::Coverage) -> (Vec<Substituted>, Vec<String>) {
    let mut substituted = Vec::new();
    let mut missing = Vec::new();
    for family in &found.order {
        match found.grades.get(family) {
            Some(Grade::Substituted(s)) => substituted.push(Substituted {
                family: family.clone(),
                substitute: s.substitute.clone(),
                metric_compatible: s.same_metrics,
                kind: match s.kind {
                    SubstitutionKind::Named => "named",
                    SubstitutionKind::Default => "default",
                    SubstitutionKind::Generic => "generic",
                },
                class: s.class.map(ooxml_fonts::FaceClass::name),
            }),
            Some(Grade::Recorded) => substituted.push(Substituted {
                family: family.clone(),
                substitute: RECORDED.to_owned(),
                metric_compatible: true,
                kind: "named",
                class: None,
            }),
            Some(Grade::Missing) => missing.push(family.clone()),
            _ => {}
        }
    }
    missing.sort();
    (substituted, missing)
}

/// The code of a glyph measured with another face, an approximation the coverage adds to
/// the layout's (docx2svg's `coverage_of`).
pub const GLYPH: &str = "layout-approximate:glyph";

/// The approximation of `family`'s `character` measured with `used` (`.notdef`: the face's
/// missing glyph), as docx2svg's coverage says it.
#[must_use]
pub fn glyph_approximation(family: &str, character: char, used: &str) -> Approximation {
    Approximation {
        code: GLYPH.to_owned(),
        message: format!(
            "{} has no glyph for {}: measured with {}",
            repr(family),
            repr(&character.to_string()),
            measured_with(used, "its missing glyph"),
        ),
        page: None,
        path: None,
    }
}

impl Approximation {
    /// Its `(reason, message)` as docx2svg's coverage lists it (`StopInfo`): the reason the
    /// part of its code after the colon (the whole code where it has none) and its message,
    /// but for a glyph measured with another face ([`GLYPH`]), which docx2svg's
    /// `coverage_of` records with its text as the reason and no message.
    #[must_use]
    pub fn listed(&self) -> (&str, &str) {
        if self.code == GLYPH {
            return (&self.message, "");
        }
        let reason = self.code.split_once(':').map_or(self.code.as_str(), |(_, r)| r);
        (reason, &self.message)
    }
}

/// Each glyph measured with another face: `(family, character, face used)`, in the order met.
fn fallbacks(found: &ooxml_word_layout::faces::Coverage) -> Vec<(String, char, String)> {
    found
        .fallback_order
        .iter()
        .filter_map(|key| Some((key.0.clone(), key.1, found.glyph_fallbacks.get(key)?.clone())))
        .collect()
}

/// docx2svg's warning per face and face used (`font-glyph-fallback`), in the order first met.
fn fallback_warnings(fallbacks: &[(String, char, String)]) -> Vec<LayoutWarning> {
    let mut grouped: Vec<((&str, &str), Vec<char>)> = Vec::new();
    for (family, c, used) in fallbacks {
        let key = (family.as_str(), used.as_str());
        match grouped.iter_mut().find(|(k, _)| *k == key) {
            Some((_, chars)) => chars.push(*c),
            None => grouped.push((key, vec![*c])),
        }
    }
    grouped
        .into_iter()
        .map(|((family, used), chars)| {
            let first = chars.first().copied().unwrap_or(' ');
            let more = if chars.len() > 1 {
                format!(" and {} more character(s)", chars.len() - 1)
            } else {
                String::new()
            };
            LayoutWarning {
                code: "font-glyph-fallback".to_owned(),
                message: format!(
                    "{} has no glyph for {} (U+{:04X}){more}: measured with {}, as an approximation (Word's \
                     own font fallback is not measured)",
                    repr(family),
                    repr(&first.to_string()),
                    u32::from(first),
                    measured_with(used, "its missing glyph (.notdef)"),
                ),
                page: None,
            }
        })
        .collect()
}

impl PageEngine {
    /// `version`'s coverage; `None` where it is not of this engine's generation. Where two
    /// or more faces are substituted, every page is painted (not kept) to read the order
    /// the body's lines draw them in, as docx2svg does.
    pub fn coverage(&mut self, version: &LaidVersion) -> Option<Coverage> {
        let found = version.glyphs().coverage();
        let (mut substituted, missing) = graded(&found);
        if substituted.len() >= 2 {
            let faces = self.body_faces(version)?;
            let (mut first, rest): (Vec<_>, Vec<_>) = substituted
                .into_iter()
                .partition(|s| faces.iter().any(|f| **f == *s.family));
            first.sort_by_key(|s| faces.iter().position(|f| **f == *s.family));
            first.extend(rest);
            substituted = first;
        }
        let mut warnings: Vec<LayoutWarning> = version
            .layout()
            .document
            .warnings
            .iter()
            .map(|code| LayoutWarning {
                code: code.to_string(),
                message: reading_message(code).to_owned(),
                page: None,
            })
            .collect();
        warnings.extend(substituted.iter().map(|s| LayoutWarning {
            code: "font-substituted".to_owned(),
            message: s.message(),
            page: None,
        }));
        let fallbacks = fallbacks(&found);
        warnings.extend(fallback_warnings(&fallbacks));
        let said = version.said();
        let mut approximations = said.approximations.clone();
        approximations.extend(
            fallbacks
                .iter()
                .map(|(family, c, used)| glyph_approximation(family, *c, used)),
        );
        warnings.extend(said.warnings.iter().cloned());
        Some(Coverage {
            complete: missing.is_empty(),
            pages: version.page_count(),
            blocks: version.layout().roles.iter().filter(|role| role.is_none()).count(),
            substituted,
            missing,
            approximations,
            warnings,
        })
    }

    /// The faces `version`'s body lines draw text in, each once, in the order drawn page by
    /// page (every page painted, not kept); `None` where it is not of this generation.
    fn body_faces(&mut self, version: &LaidVersion) -> Option<Vec<Text>> {
        let mut faces: Vec<Text> = Vec::new();
        for page in 0..version.page_count() {
            let paint = self.painted_page(version, page, false)?;
            for line in paint.lines.iter().filter(|l| l.story.is_none()) {
                for span in &line.spans {
                    if !faces.contains(&span.face) {
                        faces.push(span.face.clone());
                    }
                }
            }
        }
        Some(faces)
    }
}
