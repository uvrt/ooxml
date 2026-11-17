//! Owns: which face a rasteriser that files faces as resvg's database does draws each text
//! of an SVG with, where two faces of different files answer to one name -- docx2svg's
//! `drawing_plan`: the faces of the files the rasteriser is handed ([`FacePool`], in its
//! load order, by the names it files them under), the queries the SVG's texts make
//! ([`text_queries`]), and the passes docx2svg draws them in, each leaving out the files
//! whose faces would win a tie a text's own face loses ([`DrawingPlan`]). A text whose face
//! is the first of a tie whose legacy family (name ID 1) is the name the document gives
//! first is drawn in that face; a tie no file left out undoes stays as it is.
//!
//! Must not: draw, read a font beyond its names and CSS keys, or read the machine's
//! fonts itself (the caller hands the files, in load order).

use crate::drawing::{css, css_match, rasteriser_families, CssFace};
use crate::sfnt::{face_offsets, Face};
use crate::store::FontStore;
use std::collections::{BTreeMap, BTreeSet};

/// A face handed to the rasteriser, as a plan reads it (`_PoolFace`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PoolFace {
    /// Its file.
    pub source: String,
    /// Its number in the file.
    pub number: usize,
    css: CssFace,
    families: Vec<String>,
    legacy: BTreeSet<String>,
}

/// The faces of the files handed to a rasteriser, by each name it files them under, in its
/// load order.
#[derive(Debug, Clone, Default)]
pub struct FacePool {
    by_name: BTreeMap<String, Vec<PoolFace>>,
}

impl FacePool {
    /// The faces of `files` (in load order, each once) read through `store`; `None` where
    /// no two faces of different files tie (one name, weight, width and style): nothing to
    /// plan, the common case.
    pub fn read(store: &dyn FontStore, files: &[String]) -> Option<FacePool> {
        let mut seen_files = BTreeSet::new();
        let mut faces = Vec::new();
        for path in files {
            if !seen_files.insert(path.clone()) {
                continue;
            }
            let Ok(data) = store.read(path) else { continue };
            for (number, offset) in face_offsets(&data).into_iter().enumerate() {
                let Ok(face) = Face::at_offset(std::sync::Arc::clone(&data), offset, path.clone()) else {
                    continue;
                };
                let legacy = face
                    .names()
                    .into_iter()
                    .filter(|r| r.name_id == 1)
                    .map(|r| r.text.to_lowercase())
                    .collect();
                faces.push(PoolFace {
                    source: path.clone(),
                    number,
                    css: CssFace {
                        source: path.clone(),
                        offset,
                        css: css(&face),
                    },
                    families: rasteriser_families(&face).iter().map(|f| f.to_lowercase()).collect(),
                    legacy,
                });
            }
        }
        let mut seen: BTreeMap<(String, (u16, u16, &str)), &str> = BTreeMap::new();
        let mut tie = false;
        for face in &faces {
            for name in &face.families {
                let other = seen.entry((name.clone(), face.css.css)).or_insert(&face.source);
                tie = tie || *other != face.source;
            }
        }
        if !tie {
            return None;
        }
        let mut by_name: BTreeMap<String, Vec<PoolFace>> = BTreeMap::new();
        for face in &faces {
            let mut names = face.families.clone();
            names.dedup();
            let mut once = Vec::new();
            for name in names {
                if !once.contains(&name) {
                    once.push(name);
                }
            }
            for name in once {
                by_name.entry(name).or_default().push(face.clone());
            }
        }
        Some(FacePool { by_name })
    }
}

/// What a text asks a rasteriser for (`_text_queries`): its named families (generic ones
/// left out), `usWidthClass`, style and weight.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TextQuery {
    /// The families, as named.
    pub families: Vec<String>,
    /// The width class, 1 to 9.
    pub stretch: u16,
    /// `normal`, `italic` or `oblique`.
    pub style: String,
    /// The weight.
    pub weight: u16,
}

const GENERIC: [&str; 5] = ["serif", "sans-serif", "monospace", "cursive", "fantasy"];
const STRETCH: [&str; 9] = [
    "ultra-condensed",
    "extra-condensed",
    "condensed",
    "semi-condensed",
    "normal",
    "semi-expanded",
    "expanded",
    "extra-expanded",
    "ultra-expanded",
];

/// A `font-family` value's names as a text query reads them.
pub fn family_names(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(|n| {
            n.trim()
                .trim_matches(|c| c == '\'' || c == '"')
                .replace("\\'", "'")
                .replace("&apos;", "'")
                .replace("&amp;", "&")
        })
        .filter(|n| !n.is_empty() && !GENERIC.contains(&n.to_lowercase().as_str()))
        .collect()
}

/// Each `font-family` value of the SVG's `<text>` elements, in the order met, with the
/// queries its texts make (`_text_queries`).
pub fn text_queries(svg: &str) -> Vec<(String, BTreeSet<TextQuery>)> {
    let mut out: Vec<(String, BTreeSet<TextQuery>)> = Vec::new();
    let mut rest = svg;
    while let Some(at) = rest.find("<text") {
        let after = rest.get(at + 5..).unwrap_or("");
        rest = after;
        if !after.starts_with(|c: char| c.is_whitespace() || c == '>' || c == '/') {
            continue;
        }
        let Some(end) = after.find('>') else { break };
        let tag = after.get(..end).unwrap_or("");
        let attributes = tag_attributes(tag);
        let Some(value) = attributes.get("font-family") else { continue };
        let families = family_names(value);
        if families.is_empty() {
            continue;
        }
        let weight = match attributes.get("font-weight").map_or("normal", String::as_str) {
            "bold" => 700,
            "normal" => 400,
            w if !w.is_empty() && w.bytes().all(|b| b.is_ascii_digit()) => w.parse().unwrap_or(400),
            _ => 400,
        };
        let stretch = attributes
            .get("font-stretch")
            .and_then(|s| STRETCH.iter().position(|k| k == s))
            .map_or(5, |k| u16::try_from(k + 1).unwrap_or(5));
        let query = TextQuery {
            families,
            stretch,
            style: attributes.get("font-style").cloned().unwrap_or_else(|| "normal".into()),
            weight,
        };
        match out.iter_mut().find(|(v, _)| v == value) {
            Some((_, queries)) => {
                queries.insert(query);
            }
            None => out.push((value.clone(), BTreeSet::from([query]))),
        }
    }
    out
}

/// The four font attributes of a `<text ...>` tag, as `\s(name)="value"` finds them: the
/// last of each name met.
fn tag_attributes(tag: &str) -> BTreeMap<&'static str, String> {
    let mut out = BTreeMap::new();
    for name in ["font-family", "font-weight", "font-stretch", "font-style"] {
        let needle = format!("{name}=\"");
        let mut from = 0;
        while let Some(found) = tag.get(from..).and_then(|t| t.find(&needle)) {
            let start = from + found;
            let preceded = tag.get(..start).and_then(|t| t.chars().last()).is_some_and(char::is_whitespace);
            let value_at = start + needle.len();
            let Some(close) = tag.get(value_at..).and_then(|t| t.find('"')) else { break };
            if preceded {
                out.insert(name, tag.get(value_at..value_at + close).unwrap_or("").to_owned());
            }
            from = value_at + close + 1;
        }
    }
    out
}

/// The passes a drawing is drawn in (`DrawingPlan`): each pass's `font-family` values (the
/// first pass's also every value no later pass takes) and the files it leaves out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DrawingPlan {
    /// Per pass, its values and the files it leaves out.
    pub passes: Vec<(Vec<String>, BTreeSet<String>)>,
}

/// One query's wish: the face it is to be drawn in, and the files to leave out for it.
type Want<'p> = (TextQuery, Option<&'p PoolFace>, BTreeSet<String>);

impl FacePool {
    fn members(&self, family: &str) -> Option<&Vec<PoolFace>> {
        self.by_name.get(&family.to_lowercase())
    }

    /// The face the rasteriser draws `query` in with `withheld` left out (`drawn`).
    pub fn drawn(&self, query: &TextQuery, withheld: &BTreeSet<String>) -> Option<&PoolFace> {
        for family in &query.families {
            let Some(members) = self.members(family) else { continue };
            let members: Vec<&PoolFace> = members.iter().filter(|f| !withheld.contains(&f.source)).collect();
            if members.is_empty() {
                continue;
            }
            return first_match(&members, query);
        }
        None
    }

    /// The face `query` is to be drawn in, and the files to leave out for it (`wanted`).
    fn wanted(&self, query: &TextQuery) -> (Option<&PoolFace>, BTreeSet<String>) {
        let first = query.families.first().map(|f| f.to_lowercase()).unwrap_or_default();
        for family in &query.families {
            let Some(members) = self.members(family) else { continue };
            let all: Vec<&PoolFace> = members.iter().collect();
            let found = matches(&all, query);
            let named: Vec<&PoolFace> = found.iter().copied().filter(|f| f.legacy.contains(&first)).collect();
            let rivals: BTreeSet<String> = found
                .iter()
                .filter(|f| !named.iter().any(|n| std::ptr::eq(*n, **f)))
                .map(|f| f.source.clone())
                .collect();
            if let (Some(&own), Some(&top)) = (named.first(), found.first()) {
                if !std::ptr::eq(own, top) && !rivals.contains(&own.source) {
                    return (Some(own), rivals);
                }
            }
            return (found.first().copied(), BTreeSet::new());
        }
        (None, BTreeSet::new())
    }

    /// The plan for the texts of `queries` (`drawing_plan`), `None` where one pass, as
    /// given, draws every text in its own face.
    pub fn plan(&self, queries: &[(String, BTreeSet<TextQuery>)]) -> Option<DrawingPlan> {
        let needs: Vec<(&str, Vec<Want<'_>>)> = queries
            .iter()
            .map(|(value, found)| {
                let wants = found
                    .iter()
                    .map(|q| {
                        let (face, rivals) = self.wanted(q);
                        (q.clone(), face, rivals)
                    })
                    .collect();
                (value.as_str(), wants)
            })
            .collect();
        if !needs.iter().any(|(_, w)| w.iter().any(|(_, _, r)| !r.is_empty())) {
            return None;
        }
        let holds = |values: &[&str], withheld: &BTreeSet<String>| {
            values.iter().all(|v| {
                needs.iter().filter(|(n, _)| n == v).all(|(_, wants)| {
                    wants.iter().all(|(q, face, _)| same(self.drawn(q, withheld), *face))
                })
            })
        };
        let mut passes: Vec<(Vec<&str>, BTreeSet<String>)> = vec![(Vec::new(), BTreeSet::new())];
        for (value, wants) in &needs {
            let rivals: BTreeSet<String> = wants.iter().flat_map(|(_, _, r)| r.iter().cloned()).collect();
            let mut placed = false;
            for (values, withheld) in &mut passes {
                let mut trial: Vec<&str> = values.clone();
                trial.push(value);
                let wider: BTreeSet<String> = withheld.union(&rivals).cloned().collect();
                if holds(&trial, &wider) {
                    values.push(value);
                    *withheld = wider;
                    placed = true;
                    break;
                }
            }
            if !placed {
                if holds(&[value], &rivals) {
                    passes.push((vec![value], rivals));
                } else if let Some(first) = passes.first_mut() {
                    first.0.push(value);
                }
            }
        }
        if passes.len() == 1 && passes.first().is_some_and(|p| p.1.is_empty()) {
            return None;
        }
        Some(DrawingPlan {
            passes: passes
                .into_iter()
                .map(|(values, withheld)| (values.into_iter().map(str::to_owned).collect(), withheld))
                .collect(),
        })
    }
}

fn same(a: Option<&PoolFace>, b: Option<&PoolFace>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => std::ptr::eq(a, b),
        (None, None) => true,
        _ => false,
    }
}

fn matches<'p>(members: &[&'p PoolFace], query: &TextQuery) -> Vec<&'p PoolFace> {
    let css: Vec<CssFace> = members.iter().map(|f| f.css.clone()).collect();
    let found = css_match(&css, query.stretch, &query.style, query.weight);
    members
        .iter()
        .copied()
        .filter(|f| found.contains(&f.css))
        .collect()
}

fn first_match<'p>(members: &[&'p PoolFace], query: &TextQuery) -> Option<&'p PoolFace> {
    matches(members, query).first().copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_text_asks_for_its_named_families_width_style_and_weight() {
        let svg = r#"<svg><text x="0" font-family="'Aptos Display', sans-serif" font-weight="bold">A</text><textPath font-family="X"/><text font-family="Calibri" font-style="italic" font-stretch="condensed">b</text><text font-family="'Aptos Display', sans-serif" font-weight="300">c</text></svg>"#;
        let found = text_queries(svg);
        assert_eq!(found.len(), 2);
        let (value, queries) = &found[0];
        assert_eq!(value, "'Aptos Display', sans-serif");
        let weights: Vec<u16> = queries.iter().map(|q| q.weight).collect();
        assert_eq!(weights.len(), 2);
        assert!(weights.contains(&700) && weights.contains(&300));
        let calibri = found[1].1.iter().next().map(|q| (q.families.clone(), q.stretch, q.style.clone()));
        assert_eq!(calibri, Some((vec!["Calibri".to_owned()], 3, "italic".to_owned())));
    }
}
