//! Owns: the layout's model of a version as a projection of its reading
//! ([`Projection::of`]: a [`Docx`] in a [`View`] made a [`WordDocument`]) -- its main part's
//! body, the headers and footers its sections name, its notes, the parts its blocks resolve
//! against and its blocks' names, all found through the reading (its story parts and their
//! indexes, its relationships) -- and the parts and relationships a version's projection is
//! read from, by one rule (`crate::parts`, strict or transitional).
//!
//! Only what changed is projected again: a part the version's delta (where it is from the
//! version projected last) or its tree says is unchanged is taken whole from the latest
//! projection of the view; a body element is taken from the block tier
//! ([`memo::Projections`], the document's, on its budget) where the very element was read
//! with the same colours and fields open, so a block no edit touched keeps its
//! `Arc<Paragraph>`/`Arc<Table>` -- its key with it -- wherever it moved.
//!
//! Must not: resolve properties or count list labels (`crate::Model` does), lay out, or key
//! a memo by a block's key or path (keys name blocks; memos are kept by content, ADR-008).

pub(crate) mod memo;

pub(crate) use memo::Projections;

use crate::document::{Context, Notes, WordDocument};
use crate::docx::{Docx, StoryPart, VersionNames};
use crate::error::WordError;
use crate::ids::{BlockKey, StoryIx, VersionDelta};
use crate::model::{Block, Section, Story};
use crate::numbering::{Numbering, PictureBullet};
use crate::parts::PartRelationships;
use crate::read::{sections, At, BodyElement, Fields, KeyOf, Reader, Steps};
use crate::settings::Settings;
use crate::styles::StyleSheet;
use crate::theme::{colour_mapping, colour_scheme, font_scheme, ThemeColours};
use crate::value::{text, Text};
use crate::view::View;
use crate::xmlread::local;
use memo::{BodyRead, ChildReadings, ElementOf, KeptPart, Latest, PartRead};
use ooxml_opc::Package;
use ooxml_xml::{Element, Tree};
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

const OFFICE: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const MARKERS: &[&str] = &[
    "bookmarkStart",
    "bookmarkEnd",
    "proofErr",
    "permStart",
    "permEnd",
    "commentRangeStart",
    "commentRangeEnd",
    "moveFromRangeStart",
    "moveFromRangeEnd",
    "moveToRangeStart",
    "moveToRangeEnd",
];

/// What one projection did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ProjectionStats {
    /// Top-level body elements walked (none where the body was taken whole).
    pub elements: usize,
    /// Of them, read (not taken from the block tier).
    pub elements_read: usize,
    /// Parts read (not taken whole from the latest projection).
    pub parts_read: usize,
}

/// The layout's model of a version, projected from its reading.
pub enum Projection {}

impl Projection {
    /// `version` projected in `view`: what changed since the view's latest projection read
    /// again, every other part and block taken from it or the block tier. `delta`, what the
    /// version changed against its base, is taken for which parts changed where its base is
    /// the version projected last ([`VersionDelta::base`]); else each part is compared by
    /// its tree.
    ///
    /// # Errors
    /// [`WordError::NoMainPart`] where the version has no main part's tree; a part not XML.
    pub fn of(version: &Docx, delta: Option<&VersionDelta>, view: View) -> Result<WordDocument, WordError> {
        Ok(Projection::counted(version, delta, view)?.0)
    }

    /// [`Projection::of`], and what it did.
    ///
    /// # Errors
    /// As [`Projection::of`].
    pub fn counted(
        version: &Docx,
        delta: Option<&VersionDelta>,
        view: View,
    ) -> Result<(WordDocument, ProjectionStats), WordError> {
        let mut projecting = Projecting::new(version, delta, view)?;
        let document = projecting.document()?;
        let Projecting { stats, now, delta, .. } = projecting;
        version.kept().projections().keep_latest(
            view,
            Latest {
                version: delta.and_then(|d| d.span).map(|s| s.version),
                parts: now,
            },
        );
        Ok((document, stats))
    }
}

/// The parts a version's blocks are read and resolved against, found through its reading.
struct Parts {
    main: String,
    rels: Arc<PartRelationships>,
    document: Tree,
    styles: Option<Tree>,
    numbering: Option<(String, Tree)>,
    settings: Option<Tree>,
    theme: Option<Tree>,
}

impl Parts {
    fn of(version: &Docx) -> Result<Parts, WordError> {
        let package = &version.package;
        let main = version.main.clone();
        let document = package
            .tree(&main)?
            .ok_or_else(|| WordError::NoMainPart(main.clone()))?;
        let rels = version.relationships(&main)?;
        let named = |kind: &str| -> Result<Option<(String, Tree)>, WordError> {
            let Some(name) = first_of_type(&rels, kind) else {
                return Ok(None);
            };
            Ok(package.tree(&name)?.map(|t| (name, t)))
        };
        Ok(Parts {
            styles: named("styles")?.map(|(_, t)| t),
            numbering: named("numbering")?,
            settings: named("settings")?.map(|(_, t)| t),
            theme: named("theme")?.map(|(_, t)| t),
            main,
            rels,
            document,
        })
    }
}

/// The first part `rels` relates by the type `kind` (`styles`), transitional or strict.
fn first_of_type(rels: &PartRelationships, kind: &str) -> Option<String> {
    let wanted = format!("{OFFICE}/{kind}");
    rels.items()
        .iter()
        .filter(|r| crate::parts::rel_type_is(&r.rel_type, &wanted))
        .find_map(|r| r.target_part.clone())
}

/// The header and footer stories by relationship id, and their parts.
type HeaderStories = (BTreeMap<Text, Story>, BTreeMap<Text, Text>);

/// One projection under way.
struct Projecting<'v> {
    version: &'v Docx,
    delta: Option<&'v VersionDelta>,
    view: View,
    parts: Parts,
    /// The view's latest projection, and whether `delta` is from its version.
    latest: Option<(Arc<Latest>, bool)>,
    /// This projection's parts as read: the next one's latest.
    now: HashMap<String, KeptPart>,
    stats: ProjectionStats,
}

impl<'v> Projecting<'v> {
    fn new(version: &'v Docx, delta: Option<&'v VersionDelta>, view: View) -> Result<Projecting<'v>, WordError> {
        let latest = version.kept().projections().latest(view).map(|latest| {
            let from_it = latest.version.is_some() && delta.and_then(VersionDelta::base) == latest.version;
            (latest, from_it)
        });
        Ok(Projecting {
            parts: Parts::of(version)?,
            version,
            delta,
            view,
            latest,
            now: HashMap::new(),
            stats: ProjectionStats::default(),
        })
    }

    /// The latest projection's reading of `part` from `tree` with `colours`, where the part
    /// is unchanged since: the delta from it says so, or the tree is the very one.
    fn kept(&self, part: &str, tree: &Tree, colours: &Arc<ThemeColours>) -> Option<PartRead> {
        let (latest, from_it) = self.latest.as_ref()?;
        let kept = latest.parts.get(part)?;
        let unchanged = match (from_it, self.delta) {
            (true, Some(delta)) => !delta.changed_parts().contains(&part) && kept.of(tree, colours),
            _ => kept.of(tree, colours),
        };
        unchanged.then(|| kept.read.clone())
    }

    /// Keep `read` as this projection's reading of `part`.
    fn keep(&mut self, part: &str, tree: &Tree, colours: &Arc<ThemeColours>, read: PartRead) {
        self.now.insert(part.to_owned(), KeptPart::new(tree, colours, read));
    }

    /// The story part `part` of the reading, keyed by its story's index.
    fn story_part(&self, part: &str) -> Option<(StoryIx, &'v StoryPart)> {
        let found = self.version.part(part)?;
        Some((found.index?, found))
    }

    fn document(&mut self) -> Result<WordDocument, WordError> {
        let p = &self.parts;
        let context = self.version.kept().projections().context([
            p.theme.as_ref(),
            p.settings.as_ref(),
            p.styles.as_ref(),
            p.numbering.as_ref().map(|(_, t)| t),
        ]);
        let colours = Arc::clone(&context.colours);
        let body = self.body(&context, &colours);
        let mut warnings = body.warnings.to_vec();
        let mut cached = body.cached.to_vec();
        if self.parts.styles.is_none() {
            warnings.push(text("styles-part-missing"));
        }
        let (stories, story_parts) = self.header_stories(&body.sections, &context, &mut cached)?;
        let footnotes = self.notes("footnotes", "footnote", &context)?;
        let endnotes = self.notes("endnotes", "endnote", &context)?;
        let picture_bullets = picture_bullets(&self.version.package, self.parts.numbering.as_ref(), &context)?;
        let names = VersionNames::from_parts(
            self.version
                .parts
                .iter()
                .filter_map(|part| Some((part.index?, Arc::clone(part)))),
        );
        Ok(WordDocument {
            main_part: text(&self.parts.main),
            context,
            body: body.blocks,
            sections: body.sections,
            stories,
            story_parts,
            footnotes,
            endnotes,
            picture_bullets,
            cached_fields: Arc::from(cached),
            warnings: Arc::from(warnings),
            labels: ooxml_derive::MonoidTree::new(),
            names: Arc::new(names),
        })
    }

    /// The body: as the latest projection read it where the main part is unchanged, else
    /// each top-level element from the block tier or read.
    fn body(&mut self, cx: &Context, colours: &Arc<ThemeColours>) -> BodyRead {
        let main = self.parts.main.clone();
        let tree = self.parts.document.clone();
        if let Some(PartRead::Body(kept)) = self.kept(&main, &tree, colours) {
            self.keep(&main, &tree, colours, PartRead::Body(kept.clone()));
            return kept;
        }
        self.stats.parts_read += 1;
        let read = self.body_read(cx, colours, &tree);
        self.keep(&main, &tree, colours, PartRead::Body(read.clone()));
        read
    }

    fn body_read(&mut self, cx: &Context, colours: &Arc<ThemeColours>, tree: &Tree) -> BodyRead {
        let root = tree.root();
        let found = root.children().iter().enumerate().find_map(|(k, n)| {
            n.as_element()
                .filter(|e| local(e) == "body")
                .map(|e| (k, Arc::clone(e)))
        });
        let Some((at_body, body)) = found else {
            return BodyRead {
                blocks: Arc::from(Vec::new()),
                cached: Arc::from(Vec::new()),
                sections: Arc::from(Vec::new()),
                warnings: Arc::from(vec![text("document-no-body")]),
                children: None,
            };
        };
        let keyed = self.story_part(&self.parts.main.clone());
        let keyer = keyed.map(|(ix, part)| Keyer::new(ix, part));
        // the body's blocks keyed from its index where it has one, else named by their paths
        let at = match &keyer {
            Some(keyer) => At::Keyed(keyer, vec![u32::try_from(at_body).unwrap_or(u32::MAX)]),
            None => At::Path(text("w:body")),
        };
        let before = keyer.as_ref().and_then(|_| self.latest_body(colours));
        let mut walk = BodyWalk {
            reader: Reader::new(&cx.colours, self.view),
            projections: self.version.kept().projections(),
            keyer: keyer.as_ref(),
            view: self.view,
            colours,
            shared: before.as_ref().map(|(old, read)| SharedChildren::new(old, &body, read)),
            kept: vec![None; body.children().len()],
            fields: Fields::new(true, true),
            steps: Steps::new("w:body"),
            out: Vec::new(),
            warnings: Vec::new(),
            stats: ProjectionStats::default(),
        };
        for (k, node) in body.children().iter().enumerate() {
            if let Some(node) = node.as_element() {
                walk.child(k, node, &at);
            }
        }
        self.stats.elements += walk.stats.elements;
        self.stats.elements_read += walk.stats.elements_read;
        let blocks: Story = Arc::from(crate::read::body_story(self.view, walk.out));
        let sections: Arc<[Section]> = Arc::from(sections(
            &blocks,
            crate::w::Body::new(&body).section().map(crate::w::SectPr::element),
        ));
        BodyRead {
            blocks,
            cached: Arc::from(walk.fields.cached),
            sections,
            warnings: Arc::from(walk.warnings),
            children: keyer.is_some().then(|| (body, Arc::from(walk.kept))),
        }
    }

    /// The body the view's latest projection read with `colours` (whatever its tree), and
    /// its children's readings.
    fn latest_body(&self, colours: &Arc<ThemeColours>) -> Option<(Arc<Element>, ChildReadings)> {
        let (latest, _) = self.latest.as_ref()?;
        let kept = latest.parts.get(&self.parts.main)?;
        match &kept.read {
            PartRead::Body(read) if kept.read_with(colours) => read.children.clone(),
            _ => None,
        }
    }

    /// The headers and footers `sections` name, by relationship id, and their parts.
    fn header_stories(
        &mut self,
        sections: &[Section],
        cx: &Context,
        cached: &mut Vec<Text>,
    ) -> Result<HeaderStories, WordError> {
        let (mut stories, mut parts_of) = (BTreeMap::new(), BTreeMap::new());
        let references = sections
            .iter()
            .flat_map(|s| s.properties.headers.iter().chain(s.properties.footers.iter()));
        for (_, rid) in references {
            if stories.contains_key(rid) {
                continue;
            }
            let Some(part) = self
                .parts
                .rels
                .get(rid)
                .filter(|r| !r.external)
                .and_then(|r| r.target_part.clone())
            else {
                continue;
            };
            let Some(tree) = self.version.package.tree(&part)? else {
                continue;
            };
            let (blocks, fields) = self.story(&part, &tree, cx);
            cached.extend(fields.iter().cloned());
            stories.insert(rid.clone(), blocks);
            parts_of.insert(rid.clone(), text(&part));
        }
        Ok((stories, parts_of))
    }

    /// A header's or footer's blocks and the fields it draws as cached.
    fn story(&mut self, part: &str, tree: &Tree, cx: &Context) -> (Story, Arc<[Text]>) {
        let colours = Arc::clone(&cx.colours);
        if let Some(PartRead::Story(blocks, fields)) = self.kept(part, tree, &colours) {
            self.keep(part, tree, &colours, PartRead::Story(blocks.clone(), fields.clone()));
            return (blocks, fields);
        }
        self.stats.parts_read += 1;
        let keyer = self.story_part(part).map(|(ix, found)| Keyer::new(ix, found));
        let at = match &keyer {
            Some(keyer) => At::Keyed(keyer, Vec::new()),
            None => At::Path(text(&format!("w:{}", local(tree.root())))),
        };
        let mut fields = Fields::new(false, true);
        let blocks = Reader::new(&cx.colours, self.view).story(tree.root(), &at, &mut fields);
        let made: (Story, Arc<[Text]>) = (Arc::from(blocks), Arc::from(fields.cached));
        self.keep(part, tree, &colours, PartRead::Story(made.0.clone(), made.1.clone()));
        made
    }

    /// The notes of kind `kind` in the part the main part relates as `rel` (`footnotes`).
    fn notes(&mut self, rel: &str, kind: &'static str, cx: &Context) -> Result<Notes, WordError> {
        let Some(part) = first_of_type(&self.parts.rels, rel) else {
            return Ok(Notes::default());
        };
        let Some(tree) = self.version.package.tree(&part)? else {
            return Ok(Notes::default());
        };
        let colours = Arc::clone(&cx.colours);
        if let Some(PartRead::Notes(k, notes)) = self.kept(&part, &tree, &colours) {
            if k == kind {
                self.keep(&part, &tree, &colours, PartRead::Notes(kind, notes.clone()));
                return Ok(notes);
            }
        }
        self.stats.parts_read += 1;
        let keyer = self.story_part(&part).map(|(ix, found)| Keyer::new(ix, found));
        let keys = keyer.as_ref().map(|k| k as &dyn KeyOf);
        let notes = read_notes(&Reader::new(&cx.colours, self.view), tree.root(), kind, keys);
        self.keep(&part, &tree, &colours, PartRead::Notes(kind, notes.clone()));
        Ok(notes)
    }
}

/// One walk over a body's children: each taken as the latest projection read it (the very
/// child, no field open), else from the block tier, else read.
struct BodyWalk<'w> {
    reader: Reader<'w>,
    projections: &'w Projections,
    keyer: Option<&'w Keyer<'w>>,
    view: View,
    colours: &'w Arc<ThemeColours>,
    shared: Option<SharedChildren>,
    /// Each child's reading, where read or taken with no field open and its keys its own.
    kept: Vec<Option<Arc<BodyElement>>>,
    fields: Fields,
    steps: Steps,
    out: Vec<Block>,
    warnings: Vec<Text>,
    stats: ProjectionStats,
}

impl BodyWalk<'_> {
    /// The body's child `k`, `node`, below `at`.
    fn child(&mut self, k: usize, node: &Arc<Element>, at: &At<'_>) {
        // a path counts every child before it (a body no index keys); a place is made only
        // for a child read
        let path = self.keyer.is_none().then(|| at.child(k, node, &mut self.steps));
        let name = local(node);
        if !matches!(name, "p" | "tbl" | "sdt" | "customXml") {
            let known = matches!(name, "sectPr") || MARKERS.contains(&name);
            let code = text(&format!("body-element-unsupported:{name}"));
            if !known && !self.warnings.contains(&code) {
                self.warnings.push(code);
            }
            return;
        }
        self.stats.elements += 1;
        let closed = self.fields.stack.is_empty();
        let taken = self.shared.as_mut().filter(|_| closed).and_then(|s| s.reading_of(k));
        let open = if closed { Vec::new() } else { self.fields.open_state() };
        let of = self.keyer.map(|keyer| ElementOf {
            view: self.view,
            story: keyer.story,
            colours: self.colours,
            open: &open,
        });
        if let Some(kept) = taken.or_else(|| of.as_ref().and_then(|of| self.projections.element(node, of))) {
            self.fields.cached.extend(kept.cached.iter().cloned());
            self.out.extend(kept.blocks.iter().cloned());
            if let Some(slot) = self.kept.get_mut(k).filter(|_| closed) {
                *slot = Some(kept);
            }
            return;
        }
        self.stats.elements_read += 1;
        let stack = self.fields.stack.clone();
        let here = match path {
            Some(path) => path,
            None => at.child(k, node, &mut self.steps),
        };
        let element = Arc::new(self.reader.body_element(node, &here, &mut self.fields));
        self.out.extend(element.blocks.iter().cloned());
        // kept where it closed no field it did not open: what it read is its own
        if let Some(of) = of.filter(|_| self.fields.stack == stack) {
            let own = element.blocks.iter().all(|b| memo::durable(b, of.story));
            if let Some(slot) = self.kept.get_mut(k).filter(|_| closed && own) {
                *slot = Some(Arc::clone(&element));
            }
            self.projections.keep_element(node, &of, element);
        }
    }
}

/// The children a body shares with the body the latest projection read, by identity: each
/// child of the new body outside the runs where the two child lists differ is the very child
/// of the old body at its index shifted by the runs before it ([`ooxml_xml::changed_runs`],
/// what costs what differs between the two, not what they hold).
struct SharedChildren {
    runs: std::vec::IntoIter<ooxml_xml::Run>,
    run: Option<ooxml_xml::Run>,
    /// The old index of a new child past the last run: the new index plus this.
    shift: isize,
    read: ChildReadings,
}

impl SharedChildren {
    fn new(old: &Element, new: &Element, read: &ChildReadings) -> SharedChildren {
        let mut runs = ooxml_xml::changed_runs(old.children(), new.children()).into_iter();
        let run = runs.next();
        SharedChildren {
            runs,
            run,
            shift: 0,
            read: Arc::clone(read),
        }
    }

    /// The old body's reading of the new body's child `k`, where it is the very child the old
    /// body held and was read with no field open; asked in increasing `k`.
    fn reading_of(&mut self, k: usize) -> Option<Arc<BodyElement>> {
        while let Some(run) = self.run.as_ref().filter(|run| run.new.end <= k) {
            let (old, new) = (run.old.len() as isize, run.new.len() as isize);
            self.shift += old - new;
            self.run = self.runs.next();
        }
        if self.run.as_ref().is_some_and(|run| run.new.contains(&k)) {
            return None;
        }
        let old = usize::try_from(k as isize + self.shift).ok()?;
        self.read.get(old)?.clone()
    }
}

/// The notes of kind `kind` below `root`, their blocks keyed by `keys` where the part has an
/// index, else named by their paths.
fn read_notes(reader: &Reader<'_>, root: &Element, kind: &str, keys: Option<&dyn KeyOf>) -> Notes {
    let mut out = Notes::default();
    for (k, node) in root.children().iter().enumerate() {
        let Some(node) = node.as_element().filter(|n| local(n) == kind) else {
            continue;
        };
        let view = crate::w::Note::new(node);
        let at = match keys {
            Some(keys) => At::Keyed(keys, vec![u32::try_from(k).unwrap_or(u32::MAX)]),
            None => At::Path(text(&format!("w:{kind}[@w:id={}]", view.id().unwrap_or("None")))),
        };
        let mut blocks = reader.story(node, &at, &mut Fields::new(false, false));
        if kind == "footnote" {
            blocks.retain(|b| matches!(b, Block::Paragraph(_)));
        }
        let id = crate::xmlread::attr_int(node, "id");
        match view.kind() {
            Some(t @ ("separator" | "continuationSeparator" | "continuationNotice")) => {
                out.separators.insert(text(t), Arc::from(blocks));
                if let Some(id) = id {
                    out.separator_ids.insert(text(t), id);
                }
            }
            _ => {
                if let Some(id) = id {
                    out.notes.insert(id, Arc::from(blocks));
                }
            }
        }
    }
    out
}

/// The numbering part's picture bullets, each with its image's size in pixels.
fn picture_bullets(
    package: &Package,
    numbering: Option<&(String, Tree)>,
    cx: &Context,
) -> Result<BTreeMap<i64, PictureBullet>, WordError> {
    let mut out = BTreeMap::new();
    let Some((part, _)) = numbering else { return Ok(out) };
    if cx.numbering.picture_bullets.is_empty() {
        return Ok(out);
    }
    let rels = package.relationships(part)?;
    for (id, relationship) in &cx.numbering.picture_bullets {
        let image = relationship
            .as_deref()
            .and_then(|r| rels.get(r))
            .filter(|r| !r.external)
            .and_then(|r| r.target_part.clone());
        let pixels = match image.filter(|i| package.has_part(i)) {
            Some(image) => package.read(&image)?.and_then(|bytes| crate::imagemeta::pixels(&bytes)),
            None => None,
        };
        out.insert(
            *id,
            PictureBullet {
                relationship: relationship.clone(),
                part: Some(text(part)),
                pixels,
            },
        );
    }
    Ok(out)
}

/// Keys one story's blocks from its part's index, by their places, as the reader reads them.
struct Keyer<'a> {
    part: &'a StoryPart,
    story: StoryIx,
}

impl<'a> Keyer<'a> {
    fn new(story: StoryIx, part: &'a StoryPart) -> Keyer<'a> {
        Keyer { part, story }
    }
}

impl KeyOf for Keyer<'_> {
    fn key_at(&self, place: &[u32]) -> Option<BlockKey> {
        self.part.ids.key_of(self.story, self.part.ids.at(place)?)
    }

    fn path_at(&self, place: &[u32]) -> Option<String> {
        crate::docx::names::block_path(self.part, place)
    }
}

/// What blocks resolve against, read from the theme, settings, styles and numbering parts
/// (each where the package has it): the one reading of them, for the projections and for
/// any other reader of a version's formatting (`crate::docx`).
#[must_use]
pub fn read_context(
    theme: Option<&Tree>,
    settings: Option<&Tree>,
    styles: Option<&Tree>,
    numbering: Option<&Tree>,
) -> Context {
    read_context_after(None, [theme, settings, styles, numbering])
}

/// The context `trees` (theme, settings, styles, numbering) read, taking each piece from
/// `before` where the parts it is read from are the same (`same`, by part, in that order):
/// the colours (theme, settings; or equal), the settings, the font scheme (theme), the styles
/// (styles, colours) and the numbering (numbering, styles, colours).
pub(crate) fn read_context_after(before: Option<(&Context, [bool; 4])>, trees: [Option<&Tree>; 4]) -> Context {
    let [theme, settings, styles, numbering] = trees;
    let kept = |k: usize| {
        before
            .filter(|(_, same)| same.get(k).copied().unwrap_or(false))
            .map(|(cx, _)| cx)
    };
    let colours = match (kept(0), kept(1)) {
        (Some(cx), Some(_)) => Arc::clone(&cx.colours),
        _ => {
            let read = ThemeColours {
                scheme: theme.map(|t| colour_scheme(t.root())).unwrap_or_default(),
                mapping: settings.map(|t| colour_mapping(t.root())).unwrap_or_default(),
                active: true,
            };
            match before {
                Some((cx, _)) if *cx.colours == read => Arc::clone(&cx.colours),
                _ => Arc::new(read),
            }
        }
    };
    let colours_kept = before.is_some_and(|(cx, _)| Arc::ptr_eq(&cx.colours, &colours));
    let styles_read = match kept(2).filter(|_| colours_kept) {
        Some(cx) => cx.styles.clone(),
        None => styles.map(|t| Arc::new(StyleSheet::read(t.root(), &colours))),
    };
    let styles_kept = before.is_some_and(|(cx, _)| match (&cx.styles, &styles_read) {
        (Some(a), Some(b)) => Arc::ptr_eq(a, b),
        (None, None) => true,
        _ => false,
    });
    let numbering_read = match kept(3).filter(|_| styles_kept && colours_kept) {
        Some(cx) => Arc::clone(&cx.numbering),
        None => Arc::new(
            numbering
                .map(|t| Numbering::read(t.root(), styles_read.as_deref(), &colours))
                .unwrap_or_default(),
        ),
    };
    Context {
        numbering: numbering_read,
        settings: match kept(1) {
            Some(cx) => Arc::clone(&cx.settings),
            None => Arc::new(settings.map(|t| Settings::read(t.root())).unwrap_or_default()),
        },
        font_scheme: match kept(0) {
            Some(cx) => cx.font_scheme.clone(),
            None => theme.and_then(|t| font_scheme(t.root())).map(Arc::new),
        },
        colours,
        styles: styles_read,
    }
}

#[cfg(test)]
mod tests;
