//! Owns: `check`'s `punctuation_spacing` (docx-agent's `typography.problems`, round 35
//! R21): per paragraph of every story, the places the spacing rule of the text's language
//! is not met (`ooxml_word_model::typography::find`), with the `replace_text` calls that fix
//! them; spaces a language that wants none has (French spacing in text tagged English...)
//! as a language mismatch of their own, with the `set_language` that retags the text.
//!
//! Must not: lay out, cap a paragraph's marks or word an entry (the tool layer does).

use super::proofing::{any_text, paragraphs_where, Placed, Proofing};
use super::py_space;
use crate::names::W;
use crate::styles::own_languages;
use crate::text::atoms;
use crate::work::{Id, Work};
use crate::{Result, WordDoc};
use ooxml_word_model::typography::{find, has_marks, Action, RunLanguage, Spot, SpotKind, NBSP};
use std::collections::HashMap;

/// Characters a fix's literal find stops at: an object, breaks.
const STOPS: [char; 4] = ['\u{fffc}', '\u{b}', '\u{c}', '\n'];

/// One place the rule is not met.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpacingMark {
    /// The 0-based place of the space (or of the mark a space is missing before; after
    /// `«`, of the character after it) in the paragraph's text.
    pub at: usize,
    /// What it is.
    pub kind: SpotKind,
    /// The language of the mark's run.
    pub language: Option<String>,
}

/// A `replace_text` call that fixes some of a paragraph's marks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplaceCall {
    /// The paragraph.
    pub range: String,
    /// The text found (once in the paragraph).
    pub find: String,
    /// What it becomes.
    pub replace: String,
}

/// The `set_language` that retags French spacing's text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetLanguage {
    /// The French tag it sets.
    pub language: String,
    /// The style it sets it on; `None`: the document's (its default and the styles in the
    /// template's own language).
    pub style: Option<String>,
}

/// A paragraph's problems of one kind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpacingProblem {
    /// Spacing the language wants, missing or breakable.
    Spacing(Spacing),
    /// Spaces a language that wants none has.
    LanguageMismatch(LanguageMismatch),
}

/// A paragraph's spacing problems (`breakable`, `missing`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Spacing {
    /// The paragraph.
    pub para: String,
    /// The language of its first mark.
    pub language: Option<String>,
    /// Every place, in order.
    pub marks: Vec<SpacingMark>,
    text: Vec<char>,
    spots: Vec<Spot>,
}

impl Spacing {
    /// The `replace_text` calls that fix the first `first` marks (docx-agent's
    /// `_fix_calls`): each the shortest stretch around them, widened to whole words, that
    /// occurs once in the paragraph; marks whose stretches touch share one call.
    #[must_use]
    pub fn calls(&self, first: usize) -> Vec<ReplaceCall> {
        let mut held: Vec<&Spot> = self.spots.iter().take(first).collect();
        held.sort_by_key(|s| s.at);
        let text = &self.text;
        let mut windows: Vec<(usize, usize, Vec<&Spot>)> = Vec::new();
        for spot in held {
            let (start, end) = window(text, spot);
            match windows.last_mut() {
                Some(last) if start <= last.1 => {
                    last.1 = last.1.max(end);
                    last.2.push(spot);
                }
                _ => windows.push((start, end, vec![spot])),
            }
        }
        let stop = |k: usize| text.get(k).is_none_or(|c| STOPS.contains(c));
        let mut calls = Vec::new();
        for (mut start, mut end, mut spots) in windows {
            while !unique(text, start, end) {
                let mut grew = false;
                if start > 0 && !stop(start - 1) {
                    start -= 1;
                    grew = true;
                }
                if end < text.len() && !stop(end) {
                    end += 1;
                    grew = true;
                }
                if !grew {
                    break;
                }
            }
            let found: Vec<char> = text.get(start..end).unwrap_or_default().to_vec();
            let mut piece = found.clone();
            spots.sort_by_key(|s| std::cmp::Reverse(s.at));
            for spot in spots {
                let at = spot.at - start;
                match spot.action {
                    Action::Replace => {
                        if let Some(c) = piece.get_mut(at) {
                            *c = NBSP;
                        }
                    }
                    Action::Insert => piece.insert(at.min(piece.len()), NBSP),
                    Action::Delete => {
                        if at < piece.len() {
                            piece.remove(at);
                        }
                    }
                }
            }
            calls.push(ReplaceCall {
                range: self.para.clone(),
                find: found.into_iter().collect(),
                replace: piece.into_iter().collect(),
            });
        }
        calls
    }
}

/// A paragraph's spaces a language that wants none has (`space_before`, `space_inside`):
/// most often French text under the wrong tag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LanguageMismatch {
    /// The paragraph.
    pub para: String,
    /// The language of its first mark.
    pub language: Option<String>,
    /// Every place, in order.
    pub marks: Vec<SpacingMark>,
    /// Where the first mark's tag comes from: `default`, `style:<name>` or `run`.
    pub source: String,
    /// What retags it; `None` where the runs carry the tag themselves.
    pub fix: Option<SetLanguage>,
}

/// The stretch a fix of `spot` rewrites: from the start of the word before to the mark.
fn window(text: &[char], spot: &Spot) -> (usize, usize) {
    let mut start = spot.at.min(spot.mark);
    let end = (spot.at + usize::from(spot.action != Action::Insert)).max(spot.mark + 1);
    while start > 0 && text.get(start - 1).is_some_and(|c| !STOPS.contains(c) && !py_space(*c)) {
        start -= 1;
    }
    (start, end)
}

/// Whether `text[a..b]` occurs in `text` once, there (Python's `str.find`).
fn unique(text: &[char], a: usize, b: usize) -> bool {
    let Some(piece) = text.get(a..b) else {
        return false;
    };
    let at = |from: usize| {
        (from..=text.len().saturating_sub(piece.len())).find(|k| text.get(*k..*k + piece.len()) == Some(piece))
    };
    at(0) == Some(a) && at(a + 1).is_none()
}

/// `check`'s `punctuation_spacing` of `doc`, in document order: per paragraph its spacing
/// problems, then its language mismatch. `locale`: the session's language tag, the French
/// tag a mismatch's fix proposes where it is French.
///
/// # Errors
/// When the version cannot be read.
pub fn punctuation_spacing(doc: &WordDoc, locale: Option<&str>) -> Result<Vec<SpacingProblem>> {
    let docx = doc.docx()?;
    let mut reading = Reading {
        proofing: Proofing::new(&docx)?,
        locale,
        guess: None,
        own: None,
    };
    let mut out = Vec::new();
    // A paragraph with no mark has nothing to look at: passed over on its texts in place,
    // before its place, its table or its atoms are read.
    for placed in paragraphs_where(&docx, |p| any_text(p, &has_marks)) {
        reading.paragraph(&placed, &mut out);
    }
    Ok(out)
}

/// One reading of a document's paragraphs: what is read once for all.
struct Reading<'a> {
    proofing: Proofing,
    locale: Option<&'a str>,
    /// The French tag a mismatch's fix proposes, once found.
    guess: Option<String>,
    /// The template's own languages, once read.
    own: Option<Vec<String>>,
}

impl Reading<'_> {
    /// One paragraph's entries (docx-agent's `paragraph_problems`).
    fn paragraph(&mut self, placed: &Placed<'_>, out: &mut Vec<SpacingProblem>) {
        let work = Work::of(placed.element, Vec::new());
        let items = atoms(&work, work.root());
        let text: Vec<char> = items.iter().map(|a| a.ch).collect();
        let mut runs: HashMap<Option<Id>, RunLanguage> = HashMap::new();
        let languages: Vec<RunLanguage> = items
            .iter()
            .map(|atom| {
                runs.entry(atom.run)
                    .or_insert_with(|| {
                        let direct = atom
                            .run
                            .and_then(|r| work.find(r, W, "rPr"))
                            .and_then(|r| work.build_element(r));
                        self.proofing.of(&placed.context(direct.as_deref()))
                    })
                    .clone()
            })
            .collect();
        let spots = find(
            &text,
            &mut |i| languages.get(i).and_then(|l| l.language.clone()),
            &mut |i| items.get(i).is_some_and(|a| a.plain()) && languages.get(i).is_some_and(|l| l.proofed && !l.code),
            true,
        );
        let language_at = |i: usize| languages.get(i).and_then(|l| l.language.clone());
        let marks = |spots: &[Spot]| -> Vec<SpacingMark> {
            spots
                .iter()
                .map(|s| SpacingMark {
                    at: s.at,
                    kind: s.kind,
                    language: language_at(s.mark),
                })
                .collect()
        };
        let (mismatched, spacing): (Vec<Spot>, Vec<Spot>) = spots
            .into_iter()
            .partition(|s| matches!(s.kind, SpotKind::SpaceBefore | SpotKind::SpaceInside));
        if spacing.is_empty() && mismatched.is_empty() {
            return;
        }
        let para = placed.id();
        if let Some(first) = spacing.first() {
            out.push(SpacingProblem::Spacing(Spacing {
                para: para.clone(),
                language: language_at(first.mark),
                marks: marks(&spacing),
                text: text.clone(),
                spots: spacing.clone(),
            }));
        }
        if let Some(first) = mismatched.first() {
            let language = language_at(first.mark);
            let source = languages
                .get(first.mark)
                .and_then(|l| l.source.clone())
                .unwrap_or_else(|| "default".to_owned());
            let fix = (source != "run").then(|| {
                let style = source
                    .strip_prefix("style:")
                    .filter(|_| !self.template_own(language.as_deref()))
                    .map(str::to_owned);
                SetLanguage {
                    language: self.french_guess(),
                    style,
                }
            });
            out.push(SpacingProblem::LanguageMismatch(LanguageMismatch {
                para,
                language,
                marks: marks(&mismatched),
                source,
                fix,
            }));
        }
    }

    /// Whether `language` is the template's own (`_template_own`): a style in it is retagged
    /// with the document.
    fn template_own(&mut self, language: Option<&str>) -> bool {
        let proofing = &self.proofing;
        let own = self.own.get_or_insert_with(|| {
            proofing
                .styles
                .as_ref()
                .map(|tree| {
                    own_languages(tree.root(), "val")
                        .into_iter()
                        .map(|(tag, _)| tag)
                        .collect()
                })
                .unwrap_or_default()
        });
        let same = |a: &str, b: &str| a.replace('_', "-").to_lowercase() == b.replace('_', "-").to_lowercase();
        language.is_some_and(|l| own.iter().any(|o| same(l, o)))
    }

    /// The French tag a mismatch's fix proposes (`french_guess`): the locale's when it is
    /// French, else the first French tag with a region the styles part states, else `fr-FR`.
    fn french_guess(&mut self) -> String {
        let french = |tag: &str| tag.to_lowercase().split('-').next() == Some("fr");
        let (proofing, locale) = (&self.proofing, self.locale);
        self.guess
            .get_or_insert_with(|| {
                if let Some(locale) = locale.filter(|l| french(l)) {
                    return locale.to_owned();
                }
                proofing
                    .stated_languages()
                    .into_iter()
                    .find(|v| french(v) && v.contains('-'))
                    .unwrap_or("fr-FR")
                    .to_owned()
            })
            .clone()
    }
}
