//! Owns: spacing around punctuation as Word's proofing of the text's language expects, as
//! docx-agent's `edit/typography.py` has it (measured in Word there): the rules by language
//! ([`rule_of`]), a run's language, proofing and code-ness through the styles ([`Cascade`]),
//! and the places in a paragraph's text where the rule is not met ([`find`], [`Spot`]).
//!
//! French (`fr`, `fr-FR`, `fr-CH`, ...) wants a space before `: ; ! ?` and inside
//! guillemets; Canadian French inside guillemets only. A mark between two words (`12:30`)
//! is left alone. The languages that want no space are `check`'s alone (`check` here).
//!
//! Must not: change a tree (the edit crate writes the spacing), or know which text an edit
//! wrote.

use ooxml_xml::Element;
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::sync::Arc;

const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";

/// The no-break space the spacing is written as (what Word's AutoFormat writes).
pub const NBSP: char = '\u{a0}';
/// The narrow no-break space (passes Word's proofing too).
pub const NNBSP: char = '\u{202f}';
/// The marks French puts a space before.
const MARKS: [char; 4] = [':', ';', '!', '?'];
/// The guillemets.
const OPEN: char = '«';
const CLOSE: char = '»';
/// The spaces `check` counts as a space a language that wants none has.
const SPACES: [char; 3] = [' ', NBSP, NNBSP];
/// What may end the word a mark follows (a letter or digit too).
const WORD_END: &str = "»)]}”’\"'%.…°€$£";
/// What may follow a mark (white space and the end too).
const AFTER_MARK: &str = "»”’\"'.,…";
/// Languages whose proofing flags any space before `: ; ! ?` (`check` only).
const NO_SPACE_BEFORE: [&str; 6] = ["en", "de", "nl", "es", "it", "pt"];
/// ... and inside guillemets.
const NO_SPACE_INSIDE: [&str; 3] = ["es", "it", "pt"];
/// The names of the code styles (lower case).
const CODE_STYLES: [&str; 4] = ["html code", "verbatim char", "html preformatted", "source code"];

/// What a language's rule writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rule {
    /// Before `: ; ! ?` and inside guillemets (French).
    Full,
    /// Inside guillemets only (Canadian French).
    Guillemets,
}

/// The rule `language` (a `w:lang` value) writes: its tag's, else its primary subtag's.
#[must_use]
pub fn rule_of(language: Option<&str>) -> Option<Rule> {
    let tag = language?.to_lowercase();
    let of = |t: &str| match t {
        "fr" => Some(Rule::Full),
        "fr-ca" => Some(Rule::Guillemets),
        _ => None,
    };
    of(&tag).or_else(|| of(tag.split('-').next().unwrap_or("")))
}

fn primary(language: Option<&str>) -> String {
    language
        .unwrap_or("")
        .to_lowercase()
        .split('-')
        .next()
        .unwrap_or("")
        .to_owned()
}

/// Python's `str.isspace` for one character.
fn space(c: char) -> bool {
    c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c)
}

/// What a run's text is to the rule: its language, whether Word proofs it, whether it is
/// code.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RunLanguage {
    /// The `w:lang` value that applies, if any.
    pub language: Option<String>,
    /// Whether Word proofs it (no `w:noProof` on).
    pub proofed: bool,
    /// Whether it is in a code style.
    pub code: bool,
    /// Where `language` comes from: `run` (the run's own `w:lang`), `style:<name>` (a
    /// style's, by name, else id) or `default` (`w:docDefaults`'); `None` with no language.
    pub source: Option<String>,
}

/// The styles a run's language comes through (its paragraph's, table's and character
/// style's ids): what [`Cascade::styled`] reads, the same for many runs.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct StyleKey {
    /// The paragraph's style (its default where it names none).
    pub paragraph: Option<String>,
    /// The table's style, where the paragraph is in a table.
    pub table: Option<String>,
    /// The run's character style.
    pub character: Option<String>,
}

/// A run's language, proofing and code-ness through the styles (docx-agent's `Cascade`):
/// its own `w:lang`, else its character style's, its paragraph style's, its table style's,
/// `w:docDefaults`'.
#[derive(Debug)]
pub struct Cascade {
    styles: BTreeMap<String, Arc<Element>>,
    default_paragraph: Option<String>,
    default_table: Option<String>,
    defaults: Option<Arc<Element>>,
    code: HashSet<String>,
}

/// The styles a paragraph's run is under: the paragraph's style as written (`None`: the
/// default), its table's, and the run's direct properties.
#[derive(Debug, Clone, Copy, Default)]
pub struct RunContext<'a> {
    /// The paragraph's `w:pPr`.
    pub paragraph_properties: Option<&'a Element>,
    /// The `w:tblPr` of the innermost table the paragraph is in, and whether it is in one.
    pub table: Option<Option<&'a Element>>,
    /// The run's `w:rPr`.
    pub run_properties: Option<&'a Element>,
}

fn child<'e>(element: &'e Element, local: &str) -> Option<&'e Arc<Element>> {
    element.find(Some(W), local)
}

fn val(element: &Element) -> Option<&str> {
    element.attribute(Some(W), "val")
}

fn off(element: &Element) -> bool {
    matches!(val(element), Some("0" | "false" | "off"))
}

impl Cascade {
    /// The cascade of a styles part's `w:style` elements (in order: the last of an id
    /// counts), with `defaults` its `w:docDefaults/w:rPrDefault/w:rPr`.
    #[must_use]
    pub fn new(declared: &[Arc<Element>], defaults: Option<Arc<Element>>) -> Cascade {
        let mut styles = BTreeMap::new();
        let (mut default_paragraph, mut default_table) = (None, None);
        for style in declared {
            let Some(id) = style.attribute(Some(W), "styleId") else {
                continue;
            };
            styles.insert(id.to_owned(), Arc::clone(style));
            if matches!(style.attribute(Some(W), "default"), Some("1" | "true" | "on")) {
                match style.attribute(Some(W), "type") {
                    Some("paragraph") => default_paragraph = Some(id.to_owned()),
                    Some("table") => default_table = Some(id.to_owned()),
                    _ => {}
                }
            }
        }
        let code = styles
            .iter()
            .filter(|(_, s)| {
                child(s, "name")
                    .and_then(|n| val(n))
                    .is_some_and(|n| CODE_STYLES.contains(&n.to_lowercase().as_str()))
            })
            .map(|(id, _)| id.clone())
            .collect();
        Cascade {
            styles,
            default_paragraph,
            default_table,
            defaults,
            code,
        }
    }

    /// `id`'s `w:basedOn` chain, its own first.
    fn chain(&self, id: Option<&str>) -> Vec<&Arc<Element>> {
        let mut out = Vec::new();
        let mut seen = BTreeSet::new();
        let mut at = id.map(str::to_owned);
        while let Some(id) = at.take() {
            let Some(style) = self.styles.get(&id).filter(|_| seen.insert(id.clone())) else {
                break;
            };
            out.push(style);
            at = child(style, "basedOn").and_then(|b| val(b)).map(str::to_owned);
        }
        out
    }

    /// The language, proofing and code-ness of a run under `context`, and where its language
    /// comes from.
    #[must_use]
    pub fn of(&self, context: &RunContext<'_>) -> RunLanguage {
        self.styled(&self.key(context)).with_direct(context.run_properties)
    }

    /// The styles a run under `context` takes its language through.
    #[must_use]
    pub fn key(&self, context: &RunContext<'_>) -> StyleKey {
        let paragraph = match context.paragraph_properties.and_then(|p| child(p, "pStyle")) {
            Some(node) => val(node).map(str::to_owned),
            None => self.default_paragraph.clone(),
        };
        let table = context
            .table
            .and_then(|table| match table.and_then(|t| child(t, "tblStyle")) {
                Some(node) => val(node).map(str::to_owned),
                None => self.default_table.clone(),
            });
        let character = context
            .run_properties
            .and_then(|r| child(r, "rStyle"))
            .and_then(|n| val(n))
            .map(str::to_owned);
        StyleKey {
            paragraph,
            table,
            character,
        }
    }

    /// What the styles `key` names give a run, before its own properties
    /// ([`RunLanguage::with_direct`]): `w:docDefaults`', then each of the table's, the
    /// paragraph's and the character style's `w:basedOn` chain, its root first.
    #[must_use]
    pub fn styled(&self, key: &StyleKey) -> RunLanguage {
        let mut out = RunLanguage {
            language: None,
            proofed: true,
            code: false,
            source: None,
        };
        out.apply(self.defaults.as_deref(), || "default".to_owned());
        for id in [&key.table, &key.paragraph, &key.character] {
            for style in self.chain(id.as_deref()).into_iter().rev() {
                let name = || {
                    let shown = child(style, "name")
                        .and_then(|n| val(n))
                        .filter(|n| !n.is_empty())
                        .or_else(|| style.attribute(Some(W), "styleId"));
                    format!("style:{}", shown.unwrap_or(""))
                };
                out.apply(child(style, "rPr").map(|r| &**r), name);
            }
        }
        let ids = |id: &Option<String>| -> Vec<String> {
            self.chain(id.as_deref())
                .iter()
                .filter_map(|s| s.attribute(Some(W), "styleId").map(str::to_owned))
                .collect()
        };
        out.code = ids(&key.paragraph)
            .iter()
            .chain(ids(&key.character).iter())
            .any(|id| self.code.contains(id));
        out
    }
}

impl RunLanguage {
    /// What a run whose styles give `self` has with its own `w:rPr` (`direct`).
    #[must_use]
    pub fn with_direct(mut self, direct: Option<&Element>) -> RunLanguage {
        self.apply(direct, || "run".to_owned());
        self
    }

    /// `level`'s `w:lang` (not empty) and `w:noProof` over what is found so far, `source`
    /// where the language comes from.
    fn apply(&mut self, level: Option<&Element>, source: impl FnOnce() -> String) {
        let Some(level) = level else { return };
        if let Some(language) = child(level, "lang").and_then(|l| val(l)).filter(|v| !v.is_empty()) {
            self.language = Some(language.to_owned());
            self.source = Some(source());
        }
        if let Some(node) = child(level, "noProof") {
            self.proofed = off(node);
        }
    }
}

/// What a [`Spot`] is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpotKind {
    /// An ordinary space where the rule wants U+00A0.
    Breakable,
    /// No space where the rule wants one.
    Missing,
    /// A space before a mark the language does not want (`check` only).
    SpaceBefore,
    /// A space inside guillemets the language does not want (`check` only).
    SpaceInside,
}

/// What fixes a [`Spot`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// The character at `at` becomes U+00A0.
    Replace,
    /// U+00A0 goes in before position `at`.
    Insert,
    /// The character at `at` goes.
    Delete,
}

/// One place the rule is not met: `at` where the fix acts, `mark` the punctuation's place.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Spot {
    /// What it is.
    pub kind: SpotKind,
    /// Its fix.
    pub action: Action,
    /// Where the fix acts.
    pub at: usize,
    /// The punctuation's place.
    pub mark: usize,
}

fn wordish(c: char) -> bool {
    c.is_alphanumeric() || WORD_END.contains(c)
}

fn ends_mark(text: &[char], j: usize) -> bool {
    text.get(j).is_none_or(|c| space(*c) || AFTER_MARK.contains(*c))
}

/// The places in `text` where the rule of the language at each mark is not met;
/// `language_at(i)` the language of character `i`, `looked_at(i)` whether the rule applies
/// to it. With `check` also the spaces the languages that want none have.
pub fn find(
    text: &[char],
    language_at: &mut dyn FnMut(usize) -> Option<String>,
    looked_at: &mut dyn FnMut(usize) -> bool,
    check: bool,
) -> Vec<Spot> {
    let mut spots = Vec::new();
    let opened = text.iter().position(|c| *c == OPEN);
    let closed = text.iter().rposition(|c| *c == CLOSE);
    for (i, &c) in text.iter().enumerate() {
        if !MARKS.contains(&c) && c != OPEN && c != CLOSE {
            continue;
        }
        if !looked_at(i) {
            continue;
        }
        let language = language_at(i);
        let rule = rule_of(language.as_deref());
        let primary = primary(language.as_deref());
        let spot = if MARKS.contains(&c) {
            let wants_none = check && rule.is_none() && NO_SPACE_BEFORE.contains(&primary.as_str());
            mark_spot(text, i, rule, wants_none, looked_at)
        } else {
            let wants_none = check && rule.is_none() && NO_SPACE_INSIDE.contains(&primary.as_str());
            let paired = if c == OPEN {
                closed.is_some_and(|k| k > i)
            } else {
                opened.is_some_and(|k| k < i)
            };
            paired
                .then(|| guillemet_spot(text, i, c == OPEN, rule, wants_none, looked_at))
                .flatten()
        };
        spots.extend(spot);
    }
    spots
}

/// The spot of the mark `: ; ! ?` at `i`, if any.
fn mark_spot(
    text: &[char],
    i: usize,
    rule: Option<Rule>,
    wants_none: bool,
    looked_at: &mut dyn FnMut(usize) -> bool,
) -> Option<Spot> {
    let at = |k: usize| text.get(k).copied();
    let before = at(i.checked_sub(1)?)?;
    if MARKS.contains(&before) {
        return None; // the first of ?! or !!! only
    }
    let mut j = i;
    while at(j).is_some_and(|c| MARKS.contains(&c)) {
        j += 1;
    }
    if !ends_mark(text, j) {
        return None; // 12:30, x?y, http://
    }
    let spot = |kind, action, at| {
        Some(Spot {
            kind,
            action,
            at,
            mark: i,
        })
    };
    let word_before = i >= 2 && at(i - 2).is_some_and(wordish);
    if rule == Some(Rule::Full) {
        if before == ' ' && word_before && looked_at(i - 1) {
            return spot(SpotKind::Breakable, Action::Replace, i - 1);
        }
        if wordish(before) {
            return spot(SpotKind::Missing, Action::Insert, i);
        }
    } else if wants_none && SPACES.contains(&before) && word_before && looked_at(i - 1) {
        return spot(SpotKind::SpaceBefore, Action::Delete, i - 1);
    }
    None
}

/// The spot of the guillemet at `i` (`opening` or closing, paired in the paragraph).
fn guillemet_spot(
    text: &[char],
    i: usize,
    opening: bool,
    rule: Option<Rule>,
    wants_none: bool,
    looked_at: &mut dyn FnMut(usize) -> bool,
) -> Option<Spot> {
    let at = |k: usize| text.get(k).copied();
    // the space's place, the character beyond it (inward), and where a missing one goes
    let (inside, beyond, insert) = if opening {
        (i + 1, at(i + 2), i + 1)
    } else {
        (i.checked_sub(1)?, i.checked_sub(2).and_then(at), i)
    };
    let next = at(inside)?;
    let other = if opening { CLOSE } else { OPEN };
    let spot = |kind, action, at| {
        Some(Spot {
            kind,
            action,
            at,
            mark: i,
        })
    };
    if rule.is_some() {
        if next == ' ' && beyond.is_some_and(|b| !space(b) && b != other) && looked_at(inside) {
            return spot(SpotKind::Breakable, Action::Replace, inside);
        }
        if !space(next) && next != other {
            return spot(SpotKind::Missing, Action::Insert, insert);
        }
    } else if wants_none && SPACES.contains(&next) && looked_at(inside) {
        return spot(SpotKind::SpaceInside, Action::Delete, inside);
    }
    None
}

/// Whether `text` holds a character the rule looks at (cheaper than reading its runs).
#[must_use]
pub fn has_marks(text: &str) -> bool {
    text.chars().any(|c| MARKS.contains(&c) || c == OPEN || c == CLOSE)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spots(text: &str, language: &str) -> Vec<(SpotKind, usize)> {
        let chars: Vec<char> = text.chars().collect();
        find(&chars, &mut |_| Some(language.to_owned()), &mut |_| true, false)
            .into_iter()
            .map(|s| (s.kind, s.at))
            .collect()
    }

    #[test]
    fn french_wants_spaces_before_marks_and_inside_guillemets() {
        assert_eq!(spots("Bonjour:", "fr-FR"), vec![(SpotKind::Missing, 7)]);
        assert_eq!(spots("Fin ?", "fr"), vec![(SpotKind::Breakable, 3)]);
        assert_eq!(spots("Quoi?!", "fr-CH"), vec![(SpotKind::Missing, 4)]);
        assert_eq!(
            spots("«oui»", "fr-FR"),
            vec![(SpotKind::Missing, 1), (SpotKind::Missing, 4)]
        );
        assert_eq!(
            spots("«oui»!", "fr-CA"),
            vec![(SpotKind::Missing, 1), (SpotKind::Missing, 4)]
        );
    }

    #[test]
    fn marks_between_words_and_other_languages_are_left_alone() {
        assert!(spots("12:30 x?y", "fr-FR").is_empty());
        assert!(spots("Hello: there!", "en-US").is_empty());
        assert_eq!(rule_of(Some("fr-BE")), Some(Rule::Full));
        assert_eq!(rule_of(Some("FR-ca")), Some(Rule::Guillemets));
        assert_eq!(rule_of(None), None);
    }

    #[test]
    fn check_reports_spaces_a_language_does_not_want() {
        let chars: Vec<char> = "Hello : «x »".chars().collect();
        let found = find(&chars, &mut |_| Some("it-IT".to_owned()), &mut |_| true, true);
        let kinds: Vec<SpotKind> = found.iter().map(|s| s.kind).collect();
        assert_eq!(kinds, vec![SpotKind::SpaceBefore, SpotKind::SpaceInside]);
    }

    #[test]
    fn the_cascade_says_where_a_language_comes_from() {
        #![allow(clippy::unwrap_used)]
        let xml = format!(
            r#"<w:styles xmlns:w="{W}"><w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/></w:style><w:style w:type="character" w:styleId="De"><w:name w:val=""/><w:rPr><w:lang w:val="de-DE"/></w:rPr></w:style><w:docDefaults><w:rPrDefault><w:rPr><w:lang w:val="en-US"/></w:rPr></w:rPrDefault></w:docDefaults><w:rPr><w:rStyle w:val="De"/></w:rPr><w:rPr><w:lang w:val="fr-FR"/><w:noProof/></w:rPr></w:styles>"#
        );
        let tree = ooxml_xml::parse(xml.as_bytes(), &ooxml_xml::Limits::default()).unwrap();
        let root = tree.root();
        let styles: Vec<Arc<Element>> = root
            .child_elements()
            .filter(|e| e.name().local() == "style")
            .cloned()
            .collect();
        let defaults = root
            .find(Some(W), "docDefaults")
            .and_then(|d| d.find(Some(W), "rPrDefault"))
            .and_then(|d| d.find(Some(W), "rPr"))
            .cloned();
        let cascade = Cascade::new(&styles, defaults);
        let direct: Vec<&Arc<Element>> = root.child_elements().filter(|e| e.name().local() == "rPr").collect();
        let of = |run: Option<&Element>| {
            cascade.of(&RunContext {
                run_properties: run,
                ..RunContext::default()
            })
        };
        let plain = of(None);
        assert_eq!(
            (plain.language.as_deref(), plain.source.as_deref()),
            (Some("en-US"), Some("default"))
        );
        let styled = of(direct.first().map(|r| &***r));
        // a style named "" is said by its id
        assert_eq!(
            (styled.language.as_deref(), styled.source.as_deref()),
            (Some("de-DE"), Some("style:De"))
        );
        let own = of(direct.get(1).map(|r| &***r));
        assert_eq!(
            (own.language.as_deref(), own.source.as_deref(), own.proofed),
            (Some("fr-FR"), Some("run"), false)
        );
    }
}
