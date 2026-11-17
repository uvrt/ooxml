//! Owns: the numbering part as the layout counts it ([`Numbering`]): each instance's levels
//! with overrides applied, the list each instance counts in, the levels a `w:startOverride`
//! restarts, and the picture bullets -- docx2svg's `parse_numbering` and
//! `parse_picture_bullets` (measured: `make_numbering_probe.py`):
//! - abstract definitions sharing a `w:nsid` are one, the first in the part;
//! - `w:numStyleLink` takes the levels of the definition under the numbering style's
//!   `w:numId`; the instance still counts in its own definition's list;
//! - `w:lvlOverride/w:lvl` replaces the level; `w:startOverride` is the level's start in
//!   the instance and restarts the list's count at its first item there.
//!
//! Must not: count items (`crate::labels` does).

use crate::props::{paragraph_properties, run_properties};
use crate::styles::StyleSheet;
use crate::theme::ThemeColours;
use crate::value::{text, Props, Text};
use crate::w;
use crate::xmlread::{attr_on, local};
use ooxml_xml::Element;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::Arc;

/// One list level.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Level {
    /// `w:ilvl`.
    pub level: i64,
    /// `w:numFmt`.
    pub format: Option<Text>,
    /// `w:lvlText`.
    pub text: Option<Text>,
    /// The level's `w:pPr`: a cascade level for the paragraph.
    pub paragraph: Props,
    /// The level's `w:rPr`: the label's only.
    pub run: Props,
    /// `w:suff`.
    pub suffix: Option<Text>,
    /// `w:start` (an override's start where it states one).
    pub start: Option<i64>,
    /// `w:lvlRestart`.
    pub restart: Option<i64>,
    /// `w:isLgl`.
    pub legal: bool,
    /// `w:lvlJc`.
    pub alignment: Option<Text>,
    /// `w:lvlPicBulletId`.
    pub picture_bullet: Option<i64>,
    /// `w:pStyle`.
    pub style: Option<Text>,
    /// The definition is `w:multiLevelType="singleLevel"`.
    pub single: bool,
}

/// What an instance counts in: an abstract definition's id, or the instance itself when
/// it names no definition the part has.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ListKey {
    /// The abstract definition (`w:abstractNumId`).
    Abstract(i64),
    /// The instance (`w:numId`), counted on its own.
    Instance(i64),
}

/// A `w:numPicBullet`: the picture a level may show as its label.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PictureBullet {
    /// The picture's relationship id in the numbering part.
    pub relationship: Option<Text>,
    /// The numbering part's name.
    pub part: Option<Text>,
    /// The picture's pixel size, where its format is read.
    pub pixels: Option<(u32, u32)>,
}

/// `w:numbering` as the layout counts it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Numbering {
    /// numId -> ilvl -> level, overrides applied.
    pub levels: BTreeMap<i64, BTreeMap<i64, Arc<Level>>>,
    /// numId -> the abstract definition it counts in.
    pub lists: BTreeMap<i64, i64>,
    /// `(numId, ilvl)` restarted at the instance's first item there.
    pub restarts: BTreeSet<(i64, i64)>,
    /// `w:numPicBullet` id -> its picture's relationship id.
    pub picture_bullets: BTreeMap<i64, Option<Text>>,
}

fn val(v: Option<w::Val<'_>>) -> Option<Text> {
    v.and_then(w::Val::val).map(text)
}

fn int(v: Option<w::Val<'_>>) -> Option<i64> {
    v.and_then(w::Val::int)
}

fn level(element: w::Lvl<'_>, colours: &ThemeColours) -> Level {
    Level {
        level: element.ilvl().unwrap_or(0),
        format: val(element.format()),
        text: val(element.text()),
        paragraph: paragraph_properties(element.paragraph().map(w::PPr::element), colours),
        run: run_properties(element.run().map(w::RPr::element), colours),
        suffix: val(element.suffix()),
        start: int(element.start()),
        picture_bullet: int(element.picture_bullet()),
        alignment: val(element.alignment()),
        restart: int(element.restart()),
        legal: element.legal().is_some_and(|l| attr_on(l.element(), "val")),
        style: val(element.style()),
        single: false,
    }
}

struct Definitions<'a> {
    abstract_levels: HashMap<i64, BTreeMap<i64, Arc<Level>>>,
    canonical: HashMap<i64, i64>,
    links: HashMap<i64, Text>,
    instances: Vec<(Option<i64>, w::Num<'a>)>,
}

fn definitions<'a>(root: &'a Element, colours: &ThemeColours) -> Definitions<'a> {
    let numbering = w::Numbering::new(root);
    let mut defs = Definitions {
        abstract_levels: HashMap::new(),
        canonical: HashMap::new(),
        links: HashMap::new(),
        instances: Vec::new(),
    };
    let mut first_by_nsid: HashMap<String, i64> = HashMap::new();
    for element in numbering.definitions() {
        let Some(id) = element.id() else { continue };
        let nsid = val(element.nsid()).filter(|n| !n.is_empty());
        if let Some(first) = nsid.as_ref().and_then(|n| first_by_nsid.get(&n.to_uppercase())) {
            defs.canonical.insert(id, *first);
            continue;
        }
        if let Some(n) = &nsid {
            first_by_nsid.insert(n.to_uppercase(), id);
        }
        defs.canonical.insert(id, id);
        let single = val(element.multi_level_type()).as_deref() == Some("singleLevel");
        let mut levels = BTreeMap::new();
        for lvl in element.levels() {
            let mut made = level(lvl, colours);
            made.single = single;
            levels.insert(made.level, Arc::new(made));
        }
        defs.abstract_levels.insert(id, levels);
        if let Some(link) = val(element.style_link()).filter(|l| !l.is_empty()) {
            defs.links.insert(id, link);
        }
    }
    defs.instances = numbering.instances().map(|n| (n.id(), n)).collect();
    defs
}

impl Definitions<'_> {
    fn instance(&self, id: Option<i64>) -> Option<w::Num<'_>> {
        // A dictionary keyed by numId: the last element of an id wins.
        self.instances.iter().rev().find(|(i, _)| *i == id).map(|(_, n)| *n)
    }

    fn source_levels(
        &self,
        id: i64,
        styles: Option<&StyleSheet>,
        seen: &mut HashSet<i64>,
    ) -> BTreeMap<i64, Arc<Level>> {
        let id = self.canonical.get(&id).copied().unwrap_or(id);
        if let (Some(link), Some(styles)) = (self.links.get(&id), styles) {
            if seen.insert(id) {
                let linked = styles
                    .get(link)
                    .and_then(|s| s.paragraph.get("numPr.numId"))
                    .and_then(|v| v.int());
                let target = linked
                    .and_then(|n| self.instance(Some(n)))
                    .and_then(|e| int(e.definition()));
                if let Some(target) = target {
                    return self.source_levels(target, Some(styles), seen);
                }
            }
        }
        self.abstract_levels.get(&id).cloned().unwrap_or_default()
    }
}

fn apply_override(
    levels: &mut BTreeMap<i64, Arc<Level>>,
    o: w::LvlOverride<'_>,
    colours: &ThemeColours,
) -> (i64, Option<i64>) {
    let mut ilvl = o.ilvl().unwrap_or(0);
    let start = int(o.start());
    if let Some(replacement) = o.level() {
        let mut made = level(replacement, colours);
        made.single = levels.get(&0).is_some_and(|l| l.single);
        ilvl = made.level;
        levels.insert(made.level, Arc::new(made));
    } else if let Some(start) = start {
        let mut base = levels.get(&ilvl).map_or_else(
            || Level {
                level: ilvl,
                ..Level::default()
            },
            |l| (**l).clone(),
        );
        base.start = Some(start);
        levels.insert(ilvl, Arc::new(base));
    }
    (ilvl, start)
}

impl Numbering {
    /// The numbering part, its style links read through `styles`.
    pub fn read(root: &Element, styles: Option<&StyleSheet>, colours: &ThemeColours) -> Numbering {
        let defs = definitions(root, colours);
        let mut out = Numbering::default();
        let mut done = HashSet::new();
        for (id, element) in defs.instances.iter().rev() {
            // Later elements of one numId replace earlier ones, at the earlier one's place.
            if !done.insert(*id) {
                continue;
            }
            let (Some(id), Some(source)) = (*id, int(element.definition())) else {
                continue;
            };
            let mut levels = defs.source_levels(source, styles, &mut HashSet::new());
            for o in element.overrides() {
                let (ilvl, start) = apply_override(&mut levels, o, colours);
                if start.is_some() {
                    out.restarts.insert((id, ilvl));
                }
            }
            out.levels.insert(id, levels);
            out.lists
                .insert(id, defs.canonical.get(&source).copied().unwrap_or(source));
        }
        for bullet in w::Numbering::new(root).picture_bullets() {
            if let Some(id) = bullet.id() {
                out.picture_bullets.insert(id, picture_of(bullet.element()));
            }
        }
        out
    }

    /// The level `ilvl` of instance `num_id` (`None` for numId 0, which removes numbering).
    pub fn level(&self, num_id: i64, ilvl: i64) -> Option<&Arc<Level>> {
        if num_id == 0 {
            return None;
        }
        self.levels.get(&num_id)?.get(&ilvl)
    }

    /// The list instance `num_id` counts in.
    pub fn list_of(&self, num_id: i64) -> ListKey {
        self.lists
            .get(&num_id)
            .map_or(ListKey::Instance(num_id), |a| ListKey::Abstract(*a))
    }
}

const R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

fn picture_of(bullet: &Element) -> Option<Text> {
    let mut stack: Vec<&Element> = vec![bullet];
    while let Some(node) = stack.pop() {
        let found = match local(node) {
            "imagedata" => node.attribute(Some(R), "id"),
            "blip" => node.attribute(Some(R), "embed"),
            _ => None,
        };
        if let Some(f) = found.filter(|f| !f.is_empty()) {
            return Some(text(f));
        }
        let mut kids: Vec<&Element> = node.child_elements().map(|e| &**e).collect();
        kids.reverse();
        stack.extend(kids);
    }
    None
}
