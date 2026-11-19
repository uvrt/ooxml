//! Owns: the language of an existing document (docx-agent's `edit/language.py`, round 35,
//! measured in Word 16 for Mac): a tag read as Word writes it and the `w:lang` attribute it
//! goes in (`word_tag`), the template's own languages (`own_languages`), where the document
//! states languages (`languages`), and `set_language` -- the defaults (`w:docDefaults`,
//! `w:themeFontLang`) and the styles in the template's own language, or one style alone.
//!
//! Must not: rewrite a run's own language (the tool never asks), or invent a tag: the
//! tags are the ones Word writes, measured.

use super::sheet::{make, Sheet};
use super::{invalid, not_found, StyleStep};
use crate::doc::Key;
use crate::names::W;
use crate::review::space::Space;
use crate::sink::EditSink;
use crate::work::{Id, Work};
use crate::{EditError, Result};
use ooxml_base::pyfmt::repr_str;
use ooxml_word_model::docx::Docx;
use ooxml_xml::Element;
use std::sync::{Arc, OnceLock};

/// The `w:lang` attributes, in docx-agent's order.
pub const WORD_TAG_SLOTS: [&str; 3] = ["val", "eastAsia", "bidi"];

/// The tags Word writes, by the attribute it writes each in (measured: every language of
/// Word's dictionary set on text and saved; docx-agent's `WORD_TAGS`).
const WORD_TAGS: [(&str, &str); 3] = [
    (
        "val",
        "af-ZA am-ET az-Cyrl-AZ az-Latn-AZ be-BY bg-BG bin-NG ca-ES chr-Cher-US cs-CZ cy-GB da-DK de-AT de-CH \
         de-DE de-LI de-LU el-GR en-029 en-AU en-BZ en-CA en-GB en-ID en-IE en-JM en-NZ en-PH en-TT en-US \
         en-ZA en-ZW es-AR es-BO es-CL es-CO es-CR es-DO es-EC es-ES es-ES_tradnl es-GT es-HN es-MX es-NI \
         es-PA es-PE es-PR es-PY es-SV es-UY es-VE et-EE eu-ES ff-NG fi-FI fil-PH fo-FO fr-029 fr-BE fr-CA \
         fr-CD fr-CH fr-CI fr-CM fr-FR fr-HT fr-LU fr-MA fr-MC fr-ML fr-RE fr-SN fy-NL ga-IE gl-ES gn-PY \
         ha-Latn-NG haw-US hr-HR hsb-DE hu-HU hy-AM ibb-NG id-ID ig-NG is-IS it-CH it-IT iu-Cans-CA ka-GE \
         kk-KZ kr-NG ky-KG la-Latn lt-LT lv-LV mk-MK mn-MN ms-BN ms-MY mt-MT nb-NO nl-BE nl-NL nn-NO om-ET \
         pl-PL pt-BR pt-PT rm-CH ro-MD ro-RO ru-MD ru-RU se-NO sk-SK sl-SI so-SO sq-AL sr-Cyrl-CS sr-Latn-CS \
         st-ZA sv-FI sv-SE sw-KE tg-Cyrl-TJ ti-ER ti-ET tk-TM tn-ZA tr-TR ts-ZA tt-RU tzm-Latn-DZ uk-UA \
         uz-Cyrl-UZ uz-Latn-UZ ve-ZA vi-VN xh-ZA yo-NG zu-ZA",
    ),
    ("eastAsia", "ii-CN ja-JP ko-KR zh-CN zh-HK zh-MO zh-SG zh-TW"),
    (
        "bidi",
        "ar-AE ar-BH ar-DZ ar-EG ar-IQ ar-JO ar-KW ar-LB ar-LY ar-MA ar-OM ar-QA ar-SA ar-SY ar-TN ar-YE \
         as-IN bn-IN bo-CN dv-MV fa-IR gu-IN he-IL hi-IN km-KH kn-IN kok-IN ks-Arab lo-LA ml-IN mni-IN \
         mr-IN my-MM ne-NP or-IN pa-IN ps-AF sa-IN sd-Arab-PK sd-Deva-IN si-LK syr-SY ta-IN te-IN th-TH \
         tzm-Arab-MA ur-PK yi-Hebr",
    ),
];

/// Tags offered when a refused one is like none Word knows.
const COMMON: [&str; 7] = ["fr-FR", "fr-BE", "nl-BE", "nl-NL", "en-US", "en-GB", "de-DE"];

/// Every tag, lower-cased, with its spelling and attribute (docx-agent's `_BY_KEY`).
fn by_key() -> &'static [(String, &'static str, &'static str)] {
    static TABLE: OnceLock<Vec<(String, &'static str, &'static str)>> = OnceLock::new();
    TABLE.get_or_init(|| {
        WORD_TAGS
            .iter()
            .flat_map(|(slot, tags)| tags.split_whitespace().map(move |t| (t.to_lowercase(), t, *slot)))
            .collect()
    })
}

fn lookup(key: &str) -> Option<(&'static str, &'static str)> {
    by_key().iter().find(|(k, _, _)| k == key).map(|(_, t, s)| (*t, *s))
}

fn language_error(message: String, options: Vec<String>) -> EditError {
    invalid(message, Some("language"), options)
}

/// `difflib.get_close_matches(word, possibilities, n, cutoff)`'s ratio for two strings
/// (Ratcliff/Obershelp, as `SequenceMatcher.ratio`; no junk heuristic below 200 items).
fn ratio(a: &str, b: &str) -> f64 {
    fn matches(a: &[char], b: &[char]) -> usize {
        // the longest common block, then the same on each side of it
        let (mut best, mut at_a, mut at_b) = (0, 0, 0);
        for i in 0..a.len() {
            for j in 0..b.len() {
                let mut k = 0;
                while a.get(i + k).is_some_and(|c| b.get(j + k) == Some(c)) {
                    k += 1;
                }
                if k > best {
                    (best, at_a, at_b) = (k, i, j);
                }
            }
        }
        if best == 0 {
            return 0;
        }
        best + matches(a.get(..at_a).unwrap_or(&[]), b.get(..at_b).unwrap_or(&[]))
            + matches(a.get(at_a + best..).unwrap_or(&[]), b.get(at_b + best..).unwrap_or(&[]))
    }
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let total = a.len() + b.len();
    if total == 0 {
        return 1.0;
    }
    2.0 * matches(&a, &b) as f64 / total as f64
}

/// `(tag, attribute, note)`: the tag Word writes for `language` and the `w:lang` attribute
/// it goes in; the note when the tag was read as another spelling of it (`word_tag`).
///
/// # Errors
/// [`EditError::Style`] with the tags that would do, for what Word does not know.
pub fn word_tag(language: &str) -> Result<(&'static str, &'static str, Option<String>)> {
    let given = language.trim();
    if given.is_empty() {
        return Err(language_error(
            format!("language is a tag such as 'fr-BE', 'nl-BE' or 'en-US', not {}", repr_str(language)),
            COMMON.iter().map(|t| (*t).to_owned()).collect(),
        ));
    }
    let dashed = given.replace('_', "-");
    let mut found = lookup(&given.to_lowercase()).or_else(|| lookup(&dashed.to_lowercase()));
    if found.is_none() {
        let parts: Vec<&str> = dashed.split('-').collect();
        if let [first, script, region] = parts.as_slice() {
            if script.chars().count() == 4 {
                found = lookup(&format!("{first}-{region}").to_lowercase());
            }
        }
    }
    if let Some((tag, slot)) = found {
        let note = (tag != given).then(|| format!("{} is written as Word writes it: {tag}", repr_str(given)));
        return Ok((tag, slot, note));
    }
    let primary = dashed.split('-').next().unwrap_or("").to_lowercase();
    let mut same: Vec<&'static str> = by_key()
        .iter()
        .filter(|(_, t, _)| t.to_lowercase().split('-').next() == Some(primary.as_str()))
        .map(|(_, t, _)| *t)
        .collect();
    let own = format!("{primary}-{primary}");
    same.sort_by_key(|t| (t.to_lowercase() != own, *t));
    if !same.is_empty() {
        let what = if dashed.contains('-') {
            "not a region Word knows for it"
        } else {
            "a language without a region"
        };
        let shown: Vec<&str> = same.iter().take(30).copied().collect();
        let more = if same.len() > 30 { " ..." } else { "" };
        return Err(language_error(
            format!(
                "{} is {what}: Word proofs a language with its region, one of {}{more}",
                repr_str(given),
                shown.join(", ")
            ),
            same.iter().map(|t| (*t).to_owned()).collect(),
        ));
    }
    let word = given.to_lowercase();
    let mut close: Vec<(f64, usize)> = by_key()
        .iter()
        .enumerate()
        .map(|(k, (key, _, _))| (ratio(&word, key), k))
        .filter(|(r, _)| *r >= 0.5)
        .collect();
    // difflib: best first, ties by the possibility (the later wins in its heap: by key)
    close.sort_by(|a, b| {
        b.0.partial_cmp(&a.0)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| by_key().get(b.1).map(|x| &x.0).cmp(&by_key().get(a.1).map(|x| &x.0)))
    });
    let mut options: Vec<String> = close
        .iter()
        .take(6)
        .filter_map(|(_, k)| by_key().get(*k).map(|(_, t, _)| (*t).to_owned()))
        .collect();
    for tag in COMMON {
        if !options.iter().any(|o| o == tag) {
            options.push(tag.to_owned());
        }
    }
    Err(language_error(
        format!(
            "{} is not a language Word knows (it would proof the text in its own editing language); a tag such as {}",
            repr_str(given),
            options.join(", ")
        ),
        options,
    ))
}

fn same(a: Option<&str>, b: Option<&str>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => key(a) == key(b),
        _ => false,
    }
}

fn key(tag: &str) -> String {
    tag.replace('_', "-").to_lowercase()
}

/// A `w:lang`'s attributes that are set, in slot order (`_attributes`).
fn attributes_of(lang: Option<&Element>) -> Vec<(&'static str, String)> {
    let Some(lang) = lang else { return Vec::new() };
    WORD_TAG_SLOTS
        .iter()
        .filter_map(|s| {
            lang.attribute(Some(W), s)
                .filter(|v| !v.is_empty())
                .map(|v| (*s, v.to_owned()))
        })
        .collect()
}

fn slot_of<'a>(found: &'a [(&'static str, String)], slot: &str) -> Option<&'a str> {
    found.iter().find(|(s, _)| *s == slot).map(|(_, v)| v.as_str())
}

fn lang_in(holder: Option<&Element>) -> Option<&Arc<Element>> {
    holder?.find(Some(W), "lang")
}

fn default_lang(root: &Element) -> Option<&Arc<Element>> {
    let defaults = root.find(Some(W), "docDefaults")?;
    let run = defaults.find(Some(W), "rPrDefault")?;
    lang_in(run.find(Some(W), "rPr").map(|r| &**r))
}

fn styles_of(root: &Element) -> impl Iterator<Item = &Arc<Element>> {
    root.child_elements().filter(|c| c.name().is(Some(W), "style"))
}

/// The default paragraph style (`w:default` on, else the one with id `Normal`).
fn normal(root: &Element) -> Option<&Arc<Element>> {
    let mut fallback = None;
    for style in styles_of(root) {
        if style.attribute(Some(W), "type").unwrap_or("paragraph") != "paragraph" {
            continue;
        }
        if matches!(style.attribute(Some(W), "default"), Some("1" | "true" | "on")) {
            return Some(style);
        }
        if fallback.is_none() && style.attribute(Some(W), "styleId") == Some("Normal") {
            fallback = Some(style);
        }
    }
    fallback
}

fn own_lang(style: &Element) -> Option<&Arc<Element>> {
    lang_in(style.find(Some(W), "rPr").map(|r| &**r))
}

/// The template's own languages in `slot` (`own_languages`): `(tag, why)`, why one of
/// `default`, `Normal`, `most styles`.
fn own_languages(root: &Element, slot: &str) -> Vec<(String, &'static str)> {
    let mut out = Vec::new();
    let default = slot_of(&attributes_of(default_lang(root).map(|l| &**l)), slot).map(str::to_owned);
    if let Some(d) = &default {
        out.push((d.clone(), "default"));
    }
    let own = normal(root)
        .and_then(|n| slot_of(&attributes_of(own_lang(n).map(|l| &**l)), slot).map(str::to_owned));
    if let Some(own) = own.filter(|o| !same(Some(o), default.as_deref())) {
        out.push((own, "Normal"));
    }
    if !out.is_empty() {
        return out;
    }
    let mut counts: Vec<(String, usize, String)> = Vec::new();
    for style in styles_of(root) {
        if let Some(value) = slot_of(&attributes_of(own_lang(style).map(|l| &**l)), slot) {
            match counts.iter_mut().find(|(k, _, _)| *k == key(value)) {
                Some(c) => c.1 += 1,
                None => counts.push((key(value), 1, value.to_owned())),
            }
        }
    }
    let stating: usize = counts.iter().map(|(_, n, _)| n).sum();
    counts
        .into_iter()
        .find(|(_, n, _)| *n >= 2 && 2 * n > stating)
        .map(|(_, _, spelling)| vec![(spelling, "most styles")])
        .unwrap_or_default()
}

/// Where the document states languages (`Document.languages`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Languages {
    /// `w:docDefaults`' `w:lang`, by attribute.
    pub default: Vec<(&'static str, String)>,
    /// `w:themeFontLang`'s.
    pub theme: Vec<(&'static str, String)>,
    /// Each style with its own, by name.
    pub styles: Vec<(String, Vec<(&'static str, String)>)>,
    /// Each tag runs and paragraph marks carry themselves, and how many.
    pub runs: Vec<(String, usize)>,
}

/// Every run's and paragraph mark's own `w:lang` (`_run_langs`), in every story.
fn run_langs(docx: &Docx) -> Vec<Arc<Element>> {
    fn walk(element: &Element, out: &mut Vec<Arc<Element>>) {
        let owner = element.name().namespace() == Some(W) && matches!(element.name().local(), "r" | "pPr");
        for child in element.child_elements() {
            if owner && child.name().is(Some(W), "rPr") {
                out.extend(lang_in(Some(child)).cloned());
            }
            walk(child, out);
        }
    }
    let mut out = Vec::new();
    for part in &docx.parts {
        walk(part.root(), &mut out);
    }
    out
}

/// Where `docx` states languages.
#[must_use]
pub fn languages(docx: &Docx) -> Languages {
    let mut out = Languages::default();
    let styles = ooxml_word_model::parts::related(&docx.package, &docx.main, super::sheet::REL_STYLES)
        .ok()
        .flatten();
    if let Some(tree) = styles.and_then(|p| docx.package.tree(&p).ok().flatten()) {
        out.default = attributes_of(default_lang(tree.root()).map(|l| &**l));
        for style in styles_of(tree.root()) {
            let found = attributes_of(own_lang(style).map(|l| &**l));
            if !found.is_empty() {
                let name = ooxml_word_model::docx::style_name(style)
                    .unwrap_or_else(|| ooxml_word_model::docx::style_id(style))
                    .to_owned();
                match out.styles.iter_mut().find(|(n, _)| *n == name) {
                    Some(slot) => slot.1 = found,
                    None => out.styles.push((name, found)),
                }
            }
        }
    }
    let settings = ooxml_word_model::parts::related(&docx.package, &docx.main, ooxml_word_model::parts::REL_SETTINGS)
        .ok()
        .flatten();
    if let Some(tree) = settings.and_then(|p| docx.package.tree(&p).ok().flatten()) {
        out.theme = attributes_of(tree.root().find(Some(W), "themeFontLang").map(|l| &**l));
    }
    for lang in run_langs(docx) {
        for (_, value) in attributes_of(Some(&lang)) {
            match out.runs.iter_mut().find(|(t, _)| *t == value) {
                Some(slot) => slot.1 += 1,
                None => out.runs.push((value, 1)),
            }
        }
    }
    out
}

/// `tag` in `holder`'s `w:lang` `slot` (made where missing); whether it changed (`_set`).
fn set(work: &mut Work, holder: Id, slot: &str, tag: &str, orders: &crate::validate::Orders) -> bool {
    let lang = work.find(holder, W, "lang").unwrap_or_else(|| {
        let made = make(work, "lang", &[]);
        crate::marks::insert_in_order(work, holder, made, orders);
        made
    });
    if work.get(lang, Some(W), slot) == Some(tag) {
        return false;
    }
    crate::review::xml::set_attr(work, lang, slot, tag);
    true
}

fn child_or_made(work: &mut Work, parent: Id, local: &str, ordered: bool, orders: &crate::validate::Orders) -> Id {
    if let Some(found) = work.find(parent, W, local) {
        return found;
    }
    let made = make(work, local, &[]);
    if ordered {
        crate::marks::insert_in_order(work, parent, made, orders);
    } else {
        work.append(parent, made);
    }
    made
}

/// The defaults' language (with `default`) and the styles' -- the one named, else those in
/// the template's own (`old`) -- written; what changed, and the styles keeping another.
fn write_styles(
    work: &mut Work,
    sheet: &Sheet,
    (slot, tag): (&str, &str),
    old: &[String],
    named_id: Option<&str>,
    default: bool,
) -> (Vec<String>, Vec<(String, String)>) {
    let is_old = |value: Option<&str>| old.iter().any(|o| same(value, Some(o)));
    let mut changed: Vec<String> = Vec::new();
    let mut kept: Vec<(String, String)> = Vec::new();
    let orders = sheet.orders();
    let root = sheet.root;
    if default {
        let defaults = child_or_made(work, root, "docDefaults", true, orders);
        let run_defaults = child_or_made(work, defaults, "rPrDefault", true, orders);
        let properties = child_or_made(work, run_defaults, "rPr", false, orders);
        if set(work, properties, slot, tag, orders) {
            changed.push("default".to_owned());
        }
    }
    for node in work.iter(root, W, &["style"]) {
        let properties = work.find(node, W, "rPr");
        let own = properties
            .and_then(|p| work.find(p, W, "lang"))
            .and_then(|l| work.get(l, Some(W), slot))
            .map(str::to_owned);
        let name = super::sheet::child_val(work, node, "name")
            .or_else(|| work.get(node, Some(W), "styleId"))
            .unwrap_or("")
            .to_owned();
        let wanted = match named_id {
            Some(id) => work.get(node, Some(W), "styleId") == Some(id),
            None => is_old(own.as_deref()),
        };
        if wanted {
            let properties = properties.unwrap_or_else(|| {
                let made = make(work, "rPr", &[]);
                crate::marks::insert_in_order(work, node, made, orders);
                made
            });
            if set(work, properties, slot, tag, orders) {
                changed.push(format!("style {name}"));
            }
        } else if let Some(own) = own.filter(|o| !same(Some(o), Some(tag))) {
            match kept.iter_mut().find(|(n, _)| *n == name) {
                Some(slot) => slot.1 = own,
                None => kept.push((name, own)),
            }
        }
    }
    (changed, kept)
}

/// `set_language` (docx-agent's `Document.set_language`): the document's (`style` none,
/// styles in the template's own language following) or `style`'s alone.
pub(crate) fn set_language(
    sink: &mut EditSink<'_, '_>,
    space: &mut Space,
    language: &str,
    style: Option<&str>,
) -> Result<StyleStep> {
    // the warnings name the runs' own languages: the whole document is read
    sink.read_everything();
    let (tag, slot, note) = word_tag(language)?;
    let docx = sink.docx()?;
    let named_id = match style {
        Some(name) => Some(
            super::sheet::find_in(&docx, name, None)
                .map(|s| ooxml_word_model::docx::style_id(&s).to_owned())
                .ok_or_else(|| not_found(format!("the document has no style {}", repr_str(name))))?,
        ),
        None => None,
    };
    let default = style.is_none();
    let Some(sheet) = Sheet::open(sink, space)? else {
        return Err(EditError::Refused(
            "the document has no style sheet (styles.xml) to hold its default language".to_owned(),
        ));
    };
    let styles_tree = sink.doc().tree(&sheet.part)?.ok_or_else(|| EditError::NoPart(sheet.part.clone()))?;
    let own_set = own_languages(styles_tree.root(), slot);
    let old: Vec<String> = own_set.iter().map(|(t, _)| t.clone()).collect();
    let mut warnings: Vec<String> = note.into_iter().collect();
    let (mut changed, kept) = write_styles(sheet.work(space)?, &sheet, (slot, tag), &old, named_id.as_deref(), default);
    if default {
        let part = super::default::settings_part(sink)?;
        sink.read(Key::Part(Arc::from(part.as_str())));
        let held = sink.doc().clone();
        let work = space.work(&held, &part)?.ok_or_else(|| EditError::NoPart(part.clone()))?;
        let root = work.root();
        let theme = child_or_made(work, root, "themeFontLang", true, sheet.orders());
        if work.get(theme, Some(W), slot) != Some(tag) {
            crate::review::xml::set_attr(work, theme, slot, tag);
            changed.push("theme".to_owned());
        }
    }
    let mut own_runs: Vec<(String, usize)> = Vec::new();
    for lang in run_langs(&docx) {
        let Some(value) = lang.attribute(Some(W), slot) else { continue };
        if !same(Some(value), Some(tag)) {
            match own_runs.iter_mut().find(|(t, _)| t == value) {
                Some(slot) => slot.1 += 1,
                None => own_runs.push((value.to_owned(), 1)),
            }
        }
    }
    warnings.extend(said(&own_set, &kept, &own_runs, &old, default));
    if changed.is_empty() {
        warnings.insert(
            0,
            if default {
                format!("the document's language was {tag} already")
            } else {
                format!("the styles named were in {tag} already")
            },
        );
        return Ok(StyleStep {
            changed: false,
            warnings,
            ..StyleStep::default()
        });
    }
    space.mark_dirty(&sheet.part);
    Ok(StyleStep {
        warnings,
        ..StyleStep::of(None)
    })
}

/// The warnings `set_language` gives: the template's own language and why, the styles
/// keeping theirs, the runs carrying their own.
fn said(
    own_set: &[(String, &'static str)],
    kept: &[(String, String)],
    own_runs: &[(String, usize)],
    old: &[String],
    matching: bool,
) -> Vec<String> {
    let why = |w: &str| match w {
        "default" => "the default",
        "Normal" => "Normal's",
        _ => "most styles'",
    };
    let mut out = Vec::new();
    if matching && own_set.iter().any(|(_, w)| *w != "default") {
        let listed: Vec<String> = own_set.iter().map(|(t, w)| format!("{t} ({})", why(w))).collect();
        let stated = if own_set.iter().any(|(_, w)| *w == "default") {
            ""
        } else {
            "; the document stated no default"
        };
        out.push(format!("the template's own language: {}{stated}", listed.join(", ")));
    } else if matching && own_set.is_empty() && !kept.is_empty() {
        out.push(
            "the template states no language of its own (no default, none on Normal, none most of its styles share): every style keeps its own; styles=\"all\" sets them too"
                .to_owned(),
        );
    }
    if !kept.is_empty() {
        let mut shown = kept.to_vec();
        shown.sort();
        let listed: Vec<String> = shown.iter().take(12).map(|(n, t)| format!("{n} ({t})")).collect();
        let more = if shown.len() > 12 {
            format!(" and {} more", shown.len() - 12)
        } else {
            String::new()
        };
        out.push(format!("styles keeping their own language: {}{more}", listed.join(", ")));
    }
    if !own_runs.is_empty() {
        let stale: usize = own_runs
            .iter()
            .filter(|(t, _)| old.iter().any(|o| same(Some(t), Some(o))))
            .map(|(_, n)| n)
            .sum();
        let mut by_count = own_runs.to_vec();
        by_count.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
        let listed: Vec<String> = by_count.iter().take(8).map(|(t, n)| format!("{t} ({n})")).collect();
        let tail = if stale > 0 {
            format!("; runs=True also sets the {stale} in {}", old.join(" or "))
        } else {
            String::new()
        };
        out.push(format!("runs keeping their own language tag: {}{tail}", listed.join(", ")));
    }
    out
}
