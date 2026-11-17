//! Owns: `STYLEREF` (docx2svg's `fields.style_reference`, `find_style`, `names_language`,
//! `style_reference_target`, `style_occurrences`, `style_reference_text` and the layout's
//! `_style_references`, `_story_reference`; measured in round 21): an instruction read
//! ([`style_reference`]), the style it names and what it finds when there is none
//! ([`Target`]), the stretches of the body in that style ([`occurrences`]), and its text --
//! in the body the last occurrence before it in the document, else the first after
//! ([`style_references`]); in a header or footer the first on its page (`\l`: the last),
//! else the last before the page, else the first after ([`story_reference`]).
//!
//! Must not: find a page (the caller gives each occurrence's pages), or lay anything out.

use super::format::Uncomputable;
use super::{substitute, Rebuild, Substitutions};
use crate::flow::body_paragraphs;
use ooxml_base::Locale;
use ooxml_word_model::model::{Block, Paragraph, Run};
use ooxml_word_model::styles::{StyleKind, StyleSheet};
use ooxml_word_model::value::{Text, Value};
use ooxml_word_model::WordDocument;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

fn uncomputable(why: impl Into<String>) -> Uncomputable {
    Uncomputable(why.into())
}

/// Word's text for a `STYLEREF` it cannot resolve, in the application's language
/// (`STYLEREF_ERRORS`, measured with Word for Mac 16 in English and in Dutch): `none` --
/// no paragraph of the body has the style; `undefined` -- no style of that name;
/// `unnamed` -- the instruction names none. A text not measured in a language is not
/// computed.
fn error_text(language: &str, which: &str, name: &str) -> Option<String> {
    match (language, which) {
        ("en-US", "none") => Some("Error! No text of specified style in document.".into()),
        ("en-US", "undefined") => Some(format!(
            "Error! Use the Home tab to apply {name} to the text that you want to appear here."
        )),
        ("en-US", "unnamed") => Some("Error! No style name given.".into()),
        ("nl-NL", "none") => Some("Fout! Geen tekst met de opgegeven stijl in het document.".into()),
        _ => None,
    }
}

/// A `STYLEREF` instruction read: the style `name` as written (quotes removed, not
/// trimmed), whether `\l` searches a page from its foot up, and whether `\p` asks where
/// the text is (`above` / `below` in the body).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StyleReference {
    /// The style's name as written.
    pub name: String,
    /// `\l`.
    pub last: bool,
    /// `\p`.
    pub position: bool,
}

/// The tokens of an instruction as `STYLEREF` reads them (`"[^"]*"|\\\S|[^\s"\\]+`).
fn tokens(instruction: &str) -> Vec<String> {
    let chars: Vec<char> = instruction.chars().collect();
    let mut out = Vec::new();
    let mut k = 0;
    while let Some(&c) = chars.get(k) {
        if c == '"' {
            match chars.iter().skip(k + 1).position(|d| *d == '"') {
                Some(close) => {
                    out.push(chars.get(k..=k + 1 + close).unwrap_or_default().iter().collect());
                    k += close + 2;
                }
                None => k += 1,
            }
        } else if c == '\\' {
            match chars.get(k + 1) {
                Some(d) if !d.is_whitespace() => {
                    out.push(format!("\\{d}"));
                    k += 2;
                }
                _ => k += 1,
            }
        } else if c.is_whitespace() {
            k += 1;
        } else {
            let end = chars
                .iter()
                .skip(k)
                .position(|d| d.is_whitespace() || *d == '"' || *d == '\\')
                .map_or(chars.len(), |p| k + p);
            out.push(chars.get(k..end).unwrap_or_default().iter().collect());
            k = end;
        }
    }
    out
}

/// A `STYLEREF` instruction (`style_reference`); `Uncomputable` for a switch not computed:
/// the paragraph number ones (`\n` `\r` `\w` `\s`), a `\*` format other than `Upper`,
/// `Lower`, `FirstCap`, `Caps`, `MERGEFORMAT` and `CHARFORMAT`, an unknown one, or a name
/// not written first.
pub fn style_reference(instruction: &str) -> Result<StyleReference, Uncomputable> {
    let tokens = tokens(instruction);
    let mut k = usize::from(tokens.first().is_some_and(|t| t.to_uppercase() == "STYLEREF"));
    let (mut name, mut last, mut position, mut first) = (None::<String>, false, false, true);
    while let Some(token) = tokens.get(k) {
        k += 1;
        let Some(key) = token.strip_prefix('\\').and_then(|r| r.chars().next()) else {
            if name.is_some() || !first {
                return Err(uncomputable(
                    "its style name is not the first thing after STYLEREF, or is not one word",
                ));
            }
            let quoted = token.len() > 1 && token.starts_with('"') && token.ends_with('"');
            let chars: Vec<char> = token.chars().collect();
            name = Some(if quoted {
                chars.get(1..chars.len() - 1).unwrap_or_default().iter().collect()
            } else {
                token.clone()
            });
            first = false;
            continue;
        };
        first = false;
        match key.to_ascii_lowercase() {
            '*' => {
                let argument = tokens
                    .get(k)
                    .map_or(String::new(), |a| a.trim_matches('"').to_lowercase());
                k += 1;
                if !matches!(
                    argument.as_str(),
                    "mergeformat" | "charformat" | "upper" | "lower" | "firstcap" | "caps"
                ) {
                    return Err(uncomputable(format!(
                        "the switch \\* {argument} is not measured on STYLEREF"
                    )));
                }
            }
            'l' => last = true,
            'p' => position = true,
            't' => {}
            'n' | 'r' | 'w' | 's' => {
                return Err(uncomputable(format!(
                    "the paragraph number switch \\{key} is not computed"
                )))
            }
            _ => return Err(uncomputable(format!("the switch \\{key} is not measured on STYLEREF"))),
        }
    }
    Ok(StyleReference {
        name: name.unwrap_or_default(),
        last,
        position,
    })
}

/// What a `STYLEREF` searches for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// Text in a paragraph style (whole paragraphs) or a character style (runs), by ids.
    Style {
        /// Whether it is a paragraph style.
        paragraph: bool,
        /// The style ids.
        ids: BTreeSet<Text>,
    },
    /// Word's error text: nothing is searched.
    Error(String),
}

/// The style a `STYLEREF` names (`find_style`): whether it is a paragraph style, and its
/// ids; `None` where no style has that name. A number 1 to 9 is the built-in `heading N`;
/// names are found by `ooxml_style_names::find_style`, not trimmed.
pub fn find_style(name: &str, styles: Option<&StyleSheet>, language: Option<&str>) -> Option<(bool, BTreeSet<Text>)> {
    let numbered = !name.is_empty() && name.chars().all(|c| c.is_ascii_digit()) && {
        let n: u32 = name.parse().unwrap_or(0);
        (1..=9).contains(&n)
    };
    let name = if numbered {
        format!("heading {}", name.parse::<u32>().unwrap_or(0))
    } else {
        name.to_owned()
    };
    let entries: Vec<(StyleKind, Text, Text)> = styles
        .into_iter()
        .flat_map(|sheet| sheet.order.iter().filter_map(|id| sheet.get(id)))
        .filter(|s| matches!(s.kind, StyleKind::Paragraph | StyleKind::Character))
        .filter_map(|s| s.name.clone().map(|n| (s.kind, s.style_id.clone(), n)))
        .collect();
    let mut names: Vec<(&str, &str)> = Vec::new();
    for (_, _, n) in &entries {
        if !names.iter().any(|(k, _)| *k == &**n) {
            names.push((n, n));
        }
    }
    let Some(found) = ooxml_style_names::find_style(&name, &names, language, false) else {
        return numbered.then(|| (true, BTreeSet::new()));
    };
    let wanted = ooxml_style_names::fold(found);
    let matching = |kind: StyleKind| -> BTreeSet<Text> {
        entries
            .iter()
            .filter(|(k, _, n)| *k == kind && ooxml_style_names::fold(n) == wanted)
            .map(|(_, id, _)| id.clone())
            .collect()
    };
    let paragraphs = matching(StyleKind::Paragraph);
    if paragraphs.is_empty() {
        Some((false, matching(StyleKind::Character)))
    } else {
        Some((true, paragraphs))
    }
}

/// A locale's tag in BCP 47's conventional case (`en-US`): what `Locale.language` is in
/// docx2svg.
fn canonical(locale: &Locale) -> String {
    locale
        .tag()
        .split('-')
        .enumerate()
        .map(|(k, part)| match (k, part.len()) {
            (0, _) => part.to_ascii_lowercase(),
            (_, 2) => part.to_ascii_uppercase(),
            (_, 4) => {
                let mut c = part.chars();
                c.next().map_or(String::new(), |f| {
                    f.to_ascii_uppercase().to_string() + &c.as_str().to_ascii_lowercase()
                })
            }
            _ => part.to_owned(),
        })
        .collect::<Vec<_>>()
        .join("-")
}

/// The language a field's style names are read in (`names_language`): the conversion's
/// when the caller gave one other than en-US, else the document's (`w:themeFontLang`,
/// else the default run language); `None` when none says (English only).
pub fn names_language(document: &WordDocument, locale: &Locale) -> Option<String> {
    let tag = canonical(locale);
    if tag != "en-US" {
        return Some(tag);
    }
    let stated = document
        .context
        .settings
        .theme_font_lang
        .get("val")
        .map(|v| v.to_string())
        .filter(|v| !v.is_empty());
    stated
        .or_else(|| {
            let styles = document.context.styles.as_ref()?;
            styles
                .run_defaults
                .get("lang.val")
                .and_then(Value::str)
                .map(str::to_owned)
        })
        .filter(|v| !v.is_empty())
}

/// What a `STYLEREF` searches for, or Word's error text where it finds no style
/// (`style_reference_target`); `Uncomputable` for an error text not measured in the
/// language and the default character style.
pub fn target(
    reference: &StyleReference,
    styles: Option<&StyleSheet>,
    language: &str,
    names_language: Option<&str>,
) -> Result<Target, Uncomputable> {
    if reference.name.is_empty() {
        return error_text(language, "unnamed", "").map(Target::Error).ok_or_else(|| {
            uncomputable("the instruction names no style: Word's error text, in the application's language")
        });
    }
    let Some((paragraph, ids)) = find_style(&reference.name, styles, Some(names_language.unwrap_or(language))) else {
        return error_text(language, "undefined", &reference.name)
            .map(Target::Error)
            .ok_or_else(|| {
                uncomputable(
                    "no style has that name in the languages measured: Word's error text, in the application's language",
                )
            });
    };
    let default = |id: &Text| styles.and_then(|s| s.get(id)).is_some_and(|s| s.default);
    if !paragraph && ids.iter().any(default) {
        return Err(uncomputable("STYLEREF to the default character style is not measured"));
    }
    Ok(Target::Style { paragraph, ids })
}

/// Text in the style a `STYLEREF` searches for: its paragraph (document order, cells
/// included), the character it starts at, and its text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Occurrence {
    /// The paragraph.
    pub block: usize,
    /// The character it starts at.
    pub offset: usize,
    /// Its text, or why it cannot be shown.
    pub text: Result<String, Uncomputable>,
}

/// Fields whose text depends on the page they are drawn on.
const PAGED: [&str; 5] = ["PAGE", "NUMPAGES", "SECTIONPAGES", "PAGEREF", "STYLEREF"];

/// The text of `runs` as `STYLEREF` shows it (`_occurrence_text`): a page break is a space,
/// a floating drawing's anchor nothing; a tab, another break or an object is not measured.
fn occurrence_text<'r>(runs: impl Iterator<Item = &'r Run>) -> Result<String, Uncomputable> {
    let mut out = String::new();
    for run in runs {
        if run.field.as_ref().is_some_and(|f| PAGED.contains(&&*f.keyword)) {
            return Err(uncomputable("its text holds a page-number field"));
        }
        if run
            .properties
            .as_ref()
            .is_some_and(|p| p.declared.get("vanish").is_some_and(Value::truthy))
        {
            return Err(uncomputable("its text holds hidden text"));
        }
        let mut text: Vec<char> = run.text.chars().collect();
        let mut marks: Vec<&(usize, Text)> = run.breaks.iter().collect();
        marks.sort_by(|a, b| b.cmp(a));
        for (position, kind) in marks {
            if kind.starts_with("anchor:") {
                continue;
            }
            if &**kind != "page" {
                return Err(uncomputable("its text holds a tab, a break or an object"));
            }
            let at = (*position).min(text.len());
            text.insert(at, ' ');
        }
        out.extend(text);
    }
    Ok(out)
}

/// Every stretch of `paragraphs` in the style `ids`, in document order
/// (`style_occurrences`): a paragraph of a paragraph style whole, or each run of runs of a
/// character style. `default_paragraph` is the style of a paragraph naming none; `skip`
/// the paragraphs not searched (the notes placed in the flow).
pub fn occurrences(
    paragraphs: &[Arc<Paragraph>],
    target: (bool, &BTreeSet<Text>),
    default_paragraph: Option<&str>,
    skip: &BTreeSet<usize>,
) -> Vec<Occurrence> {
    let (paragraph_style, ids) = target;
    let mut out = Vec::new();
    for (block, paragraph) in paragraphs.iter().enumerate() {
        if skip.contains(&block) {
            continue;
        }
        if paragraph_style {
            let style = paragraph
                .properties
                .as_ref()
                .and_then(|p| p.style_id.as_deref())
                .filter(|s| !s.is_empty())
                .or(default_paragraph);
            if style.is_some_and(|s| ids.contains(s)) {
                out.push(Occurrence {
                    block,
                    offset: 0,
                    text: occurrence_text(paragraph.runs.iter()),
                });
            }
            continue;
        }
        let (mut offset, mut start, mut stretch) = (0, 0, Vec::new());
        for run in paragraph.runs.iter().map(Some).chain(std::iter::once(None)) {
            let inside = run.is_some_and(|r| {
                r.properties
                    .as_ref()
                    .and_then(|p| p.style_id.as_deref())
                    .is_some_and(|s| ids.contains(s))
            });
            if inside {
                if stretch.is_empty() {
                    start = offset;
                }
                stretch.extend(run);
            } else if !stretch.is_empty() {
                out.push(Occurrence {
                    block,
                    offset: start,
                    text: occurrence_text(stretch.drain(..)),
                });
            }
            if let Some(r) = run {
                offset += r.text.chars().count();
            }
        }
    }
    out
}

/// A `STYLEREF`'s text (`style_reference_text`): the occurrence found, its case switches
/// applied; Word's error text where the target is one, or where nothing has the style.
pub fn reference_text(
    instruction: &str,
    found: Option<&Occurrence>,
    target: &Target,
    language: &str,
) -> Result<String, Uncomputable> {
    if let Target::Error(text) = target {
        return Ok(text.clone());
    }
    let Some(found) = found else {
        return error_text(language, "none", "").ok_or_else(|| {
            uncomputable("no text of the style in the document: Word's error text, in the application's language")
        });
    };
    let text = found.text.clone()?;
    Ok(super::references::apply_case(&text, instruction))
}

/// What finding a `STYLEREF`'s text reads of a document: its paragraphs, the paragraphs
/// not searched, its styles, the conversion's language and the one style names are read
/// in; each style's occurrences found once.
pub struct StyleSearch<'d> {
    /// The body's paragraphs, document order (cells included).
    pub paragraphs: Vec<Arc<Paragraph>>,
    skip: BTreeSet<usize>,
    styles: Option<&'d StyleSheet>,
    default_paragraph: Option<Text>,
    /// The conversion's language (`en-US`).
    pub language: String,
    names_language: Option<String>,
    found: BTreeMap<(bool, BTreeSet<Text>), Arc<[Occurrence]>>,
}

impl<'d> StyleSearch<'d> {
    /// The search over `document`'s body, the top-level blocks `placed` says are notes
    /// placed in the flow left out.
    pub fn new(document: &'d WordDocument, locale: &Locale, placed: &dyn Fn(usize) -> bool) -> StyleSearch<'d> {
        let (paragraphs, firsts) = body_paragraphs(&document.body);
        let mut skip = BTreeSet::new();
        for (body, first) in firsts.iter().enumerate() {
            if placed(body) {
                let next = firsts.get(body + 1).copied().unwrap_or(paragraphs.len());
                skip.extend(*first..next);
            }
        }
        let styles = document.context.styles.as_deref();
        StyleSearch {
            paragraphs,
            skip,
            styles,
            default_paragraph: styles
                .and_then(|s| s.default_of(StyleKind::Paragraph))
                .map(|s| s.style_id.clone()),
            language: canonical(locale),
            names_language: names_language(document, locale),
            found: BTreeMap::new(),
        }
    }

    /// What a `STYLEREF` instruction searches for and its reading; `None` where it cannot
    /// be computed.
    pub fn target(&self, instruction: &str) -> Option<(StyleReference, Target)> {
        let reference = style_reference(instruction).ok()?;
        let target = target(&reference, self.styles, &self.language, self.names_language.as_deref()).ok()?;
        Some((reference, target))
    }

    /// The occurrences of a style target, found once.
    pub fn occurrences(&mut self, paragraph: bool, ids: &BTreeSet<Text>) -> Arc<[Occurrence]> {
        let key = (paragraph, ids.clone());
        if let Some(found) = self.found.get(&key) {
            return Arc::clone(found);
        }
        let found: Arc<[Occurrence]> = occurrences(
            &self.paragraphs,
            (paragraph, ids),
            self.default_paragraph.as_deref(),
            &self.skip,
        )
        .into();
        self.found.insert(key, Arc::clone(&found));
        found
    }
}

/// Each occurrence's pages (0-based, blank pages counted), given the occurrences.
pub type OccurrencePages<'p> = dyn FnMut(&[Occurrence]) -> Arc<[BTreeSet<usize>]> + 'p;

/// A story's `STYLEREF` run (paragraph, run) and the text it was given (none: cached).
pub type StoryText = ((usize, usize), Option<String>);

/// Whether any run of `blocks` (cells included) holds a `STYLEREF`.
pub fn has_style_reference(blocks: &[Block]) -> bool {
    body_paragraphs(blocks)
        .0
        .iter()
        .any(|p| p.runs.iter().any(is_style_reference))
}

fn is_style_reference(run: &Run) -> bool {
    run.field.as_ref().is_some_and(|f| &*f.keyword == "STYLEREF")
}

/// The body with every `STYLEREF` field's text computed (`_style_references`): the last
/// text of its style before the field in the document, else the first after it, whatever
/// the pages; `\p` draws `above` or `below` (in en-US). A field that cannot be computed
/// keeps Word's cached result. Paragraphs computed alike before are taken from `kept`.
pub fn style_references(
    document: &WordDocument,
    search: &mut StyleSearch<'_>,
    kept: &mut Substitutions,
) -> WordDocument {
    let mut values: BTreeMap<(usize, usize), String> = BTreeMap::new();
    let keys: Vec<(usize, usize, Text)> = search
        .paragraphs
        .iter()
        .enumerate()
        .flat_map(|(block, p)| {
            p.runs.iter().enumerate().filter_map(move |(k, run)| {
                run.field
                    .as_ref()
                    .filter(|_| is_style_reference(run))
                    .map(|f| (block, k, f.instruction.clone()))
            })
        })
        .collect();
    if keys.is_empty() {
        kept.retain_used();
        return document.clone();
    }
    for (block, k, instruction) in keys {
        let Some((reference, target)) = search.target(&instruction) else {
            continue;
        };
        let (occurrence, before) = match &target {
            Target::Style { paragraph, ids } => {
                let offset: usize = search
                    .paragraphs
                    .get(block)
                    .map_or(0, |p| p.runs.iter().take(k).map(|r| r.text.chars().count()).sum());
                let all = search.occurrences(*paragraph, ids);
                let earlier = all.iter().rfind(|o| (o.block, o.offset) < (block, offset));
                let later = all.iter().find(|o| (o.block, o.offset) >= (block, offset));
                (earlier.or(later).cloned(), earlier.is_some())
            }
            Target::Error(_) => (None, true),
        };
        let text = if reference.position && occurrence.is_some() {
            if search.language == "en-US" {
                Ok(if before { "above" } else { "below" }.to_owned())
            } else {
                Err(uncomputable("\\p says where the text is in the application's language"))
            }
        } else {
            reference_text(&instruction, occurrence.as_ref(), &target, &search.language)
        };
        if let Ok(text) = text {
            values.insert((block, k), text);
        }
    }
    let out = substitute(document, &values, kept);
    kept.retain_used();
    out
}

/// A header's or footer's `STYLEREF` on page `page` (`_story_reference`): the first text
/// of its style on the page -- with `\l` the last -- else the last before the page, else
/// the first after it. `pages_of` gives each occurrence's pages (0-based, blank pages
/// counted). `None` where it cannot be computed (Word's cached result is drawn).
pub fn story_reference(
    search: &mut StyleSearch<'_>,
    instruction: &str,
    page: usize,
    pages_of: &mut OccurrencePages<'_>,
) -> Option<String> {
    let (reference, target) = search.target(instruction)?;
    let mut occurrence = None;
    if let Target::Style { paragraph, ids } = &target {
        let all = search.occurrences(*paragraph, ids);
        let pages = pages_of(&all);
        let on: Vec<&Occurrence> = all
            .iter()
            .zip(pages.iter())
            .filter(|(_, p)| p.contains(&page))
            .map(|(o, _)| o)
            .collect();
        occurrence = if reference.last { on.last() } else { on.first() }.copied().cloned();
        if on.is_empty() {
            let first = |p: &BTreeSet<usize>| p.iter().next().copied();
            let earlier = all
                .iter()
                .zip(pages.iter())
                .rfind(|(_, p)| first(p).is_some_and(|f| f < page))
                .map(|(o, _)| o);
            let later = all
                .iter()
                .zip(pages.iter())
                .find(|(_, p)| first(p).is_none_or(|f| f > page))
                .map(|(o, _)| o);
            occurrence = earlier.or(later).cloned();
        }
    }
    reference_text(instruction, occurrence.as_ref(), &target, &search.language).ok()
}

/// `blocks` with each `STYLEREF` run's text as `text_of(paragraph, run)` gives it (none:
/// kept), every other block shared (`substitute_blocks`); and the texts given.
pub fn with_story_references(
    blocks: &[Block],
    text_of: &mut dyn FnMut(&Run) -> Option<String>,
) -> (Arc<[Block]>, Vec<StoryText>) {
    let mut texts = Vec::new();
    let mut change = |index: usize, paragraph: &Arc<Paragraph>| -> Option<Arc<Paragraph>> {
        let mut changed = false;
        let runs: Vec<Run> = paragraph
            .runs
            .iter()
            .enumerate()
            .map(|(k, run)| {
                if !is_style_reference(run) {
                    return run.clone();
                }
                let text = text_of(run);
                texts.push(((index, k), text.clone()));
                match text {
                    Some(t) if *t != *run.text => {
                        changed = true;
                        Run {
                            text: Arc::from(t.as_str()),
                            ..run.clone()
                        }
                    }
                    _ => run.clone(),
                }
            })
            .collect();
        changed.then(|| {
            Arc::new(Paragraph {
                runs: runs.into(),
                ..(**paragraph).clone()
            })
        })
    };
    let mut walk = Rebuild {
        next: 0,
        change: &mut change,
    };
    let out: Arc<[Block]> = blocks.iter().map(|b| walk.block(b)).collect();
    (out, texts)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instructions_read_as_word_reads_them() {
        let r = style_reference(r#"STYLEREF "Chapter title" \l \* Upper"#);
        assert_eq!(
            r.ok(),
            Some(StyleReference {
                name: "Chapter title".into(),
                last: true,
                position: false
            })
        );
        assert!(style_reference(r"STYLEREF 1 \n").is_err());
        assert!(style_reference(r"STYLEREF Chapter title").is_err());
        assert!(style_reference(r"STYLEREF \l Title").is_err());
        assert_eq!(style_reference("STYLEREF").map(|r| r.name).ok().as_deref(), Some(""));
        assert_eq!(
            style_reference(r#"STYLEREF " Chapter title " \p"#)
                .map(|r| (r.name, r.position))
                .ok(),
            Some((" Chapter title ".into(), true))
        );
    }
}
