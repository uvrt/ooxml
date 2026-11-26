//! Owns: opening versions of one document ([`Model`]): reading its parts into a
//! [`WordDocument`] and, for every later version, reusing what is unchanged -- a part whose
//! tree is the same (shared, or equal by digest) is not read again; a top-level body element
//! whose content, the fields open at it and the theme's colours are the same is taken as it
//! was read (docx2svg's `KeptBlocks`, round 41), an element the version before shares not
//! digested again; resolved formats stay memoised while the
//! styles, numbering, settings and theme are unchanged; and the body's list labels are
//! brought up to date by the blocks that changed, in O(log n) each.
//!
//! Must not: hold more than one document's state, or anything global; the caller owns the
//! `Model` (the document's derived engine, design section 6).

use crate::digest;
use crate::document::{Context, Notes, ResolvedParagraph, WordDocument};
use crate::error::WordError;
use crate::labels::BlockItems;
use crate::model::{Block, Paragraph, Story, Table};
use crate::numbering::{Numbering, PictureBullet};
use crate::read::{sections, BodyElement, Fields, Reader, Steps};
use crate::resolve::{ResolvedTable, Resolver};
use crate::settings::Settings;
use crate::styles::StyleSheet;
use crate::theme::{colour_mapping, colour_scheme, font_scheme, ThemeColours};
use crate::value::{text, Props, Text};
use crate::xmlread::{elements, local};
use ooxml_base::{Digest, DigestKey, Digester};
use ooxml_derive::{CacheBudget, Memo, MemoStats, MonoidTree, Weigh};
use ooxml_opc::{Package, Relationships};
use ooxml_xml::{Element, Tree};
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

mod keys;
mod labels;

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

/// What the last open did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct OpenStats {
    /// Top-level body elements.
    pub elements: usize,
    /// Of them, read (not taken as before).
    pub elements_read: usize,
    /// Body blocks whose list items were found again.
    pub label_blocks: usize,
    /// Edits made to the labels' tree.
    pub label_edits: usize,
    /// Resolutions computed (paragraphs, runs, tables), not taken from the memo.
    pub resolutions: u64,
    /// Whether the styles, numbering, settings or theme changed (everything resolved again).
    pub context_changed: bool,
}

#[derive(Debug, Clone)]
struct Kept<T>(Arc<T>);

impl Weigh for Kept<BodyElement> {
    fn weight(&self) -> usize {
        256 + self.0.blocks.len() * 1024
    }
}

/// A header's or footer's blocks and the fields it draws as cached.
type StoryRead = (Story, Arc<[Text]>);
/// The header and footer stories by relationship id, and their parts.
type Stories = (BTreeMap<Text, Story>, BTreeMap<Text, Text>);

impl Weigh for Kept<StoryRead> {
    fn weight(&self) -> usize {
        256 + self.0 .0.len() * 1024
    }
}

impl Weigh for Kept<Notes> {
    fn weight(&self) -> usize {
        256 + self.0.notes.len() * 1024
    }
}

/// The part digests of one version, by part name, with the tree each was taken from.
#[derive(Default)]
struct PartDigests {
    known: HashMap<String, (Arc<Element>, Digest)>,
}

impl PartDigests {
    fn of(&mut self, key: DigestKey, name: &str, tree: &Tree) -> Digest {
        if let Some((root, d)) = self.known.get(name) {
            if Arc::ptr_eq(root, tree.root()) {
                return *d;
            }
        }
        let d = digest::element(key, tree.root());
        self.known.insert(name.to_owned(), (Arc::clone(tree.root()), d));
        d
    }
}

/// The content digests of the last version's top-level body elements, by the element
/// (its pointer, the element held): an element the next version shares is not digested
/// again (an edit's tree shares what it did not change).
#[derive(Default)]
struct ElementDigests {
    last: HashMap<usize, (Arc<Element>, Digest)>,
    now: HashMap<usize, (Arc<Element>, Digest)>,
}

impl ElementDigests {
    /// `element`'s content digest, kept for the next version.
    fn of(&mut self, key: DigestKey, element: &Arc<Element>) -> Digest {
        let at = Arc::as_ptr(element) as usize;
        let d = match self.last.get(&at) {
            Some((kept, d)) if Arc::ptr_eq(kept, element) => *d,
            _ => digest::element(key, element),
        };
        self.now.insert(at, (Arc::clone(element), d));
        d
    }

    /// The version's elements digested: the next version is taken from them.
    fn finish(&mut self) {
        self.last = std::mem::take(&mut self.now);
    }
}

/// The parts a version's blocks are read and resolved against.
struct Parts {
    main: String,
    rels: Relationships,
    document: Tree,
    styles: Option<(String, Tree)>,
    numbering: Option<(String, Tree)>,
    settings: Option<Tree>,
    theme: Option<Tree>,
}

/// Opens versions of one Word document, reusing what each shares with the last.
pub struct Model {
    key: DigestKey,
    budget: Arc<dyn CacheBudget>,
    digests: PartDigests,
    bodies: ElementDigests,
    context: Option<(Digest, Context)>,
    elements: Memo<Digest, Kept<BodyElement>>,
    stories: Memo<Digest, Kept<StoryRead>>,
    notes: Memo<Digest, Kept<Notes>>,
    resolver: Resolver,
    label_keys: Vec<Digest>,
    block_keys: labels::LabelKeys,
    labels: MonoidTree<BlockItems>,
    label_memo: Memo<Digest, BlockItems>,
    /// Each version's story parts' indexes, which its blocks are keyed and named by.
    names: keys::Names,
    stats: OpenStats,
}

impl std::fmt::Debug for Model {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Model").field("stats", &self.stats).finish()
    }
}

fn first_of_type(rels: &Relationships, kind: &str) -> Option<String> {
    rels.targets_of_type(&format!("{OFFICE}/{kind}"))
        .next()
        .map(str::to_owned)
}

fn main_part(package: &Package) -> Result<String, WordError> {
    let root = package.relationships("")?;
    let main = root
        .iter()
        .filter(|r| r.rel_type == format!("{OFFICE}/officeDocument"))
        .find_map(|r| r.target_part.clone())
        .unwrap_or_else(|| "word/document.xml".to_owned());
    Ok(main)
}

fn tree_of(package: &Package, name: &str) -> Result<Option<Tree>, WordError> {
    Ok(package.tree(name)?)
}

/// The names of the parts a version's blocks resolve against -- its main part's styles,
/// numbering, settings and theme -- as [`Model::open`] finds them: a change of any of them
/// may change what the model resolves (its context).
///
/// # Errors
/// The package's relationships do not read.
pub fn context_parts(package: &Package) -> Result<Vec<String>, WordError> {
    let main = main_part(package)?;
    let rels = package.relationships(&main)?;
    Ok(["styles", "numbering", "settings", "theme"]
        .iter()
        .filter_map(|kind| first_of_type(&rels, kind))
        .collect())
}

fn parts(package: &Package) -> Result<Parts, WordError> {
    let main = main_part(package)?;
    let document = tree_of(package, &main)?.ok_or_else(|| WordError::NoMainPart(main.clone()))?;
    let rels = package.relationships(&main)?;
    let named = |kind: &str| -> Result<Option<(String, Tree)>, WordError> {
        let Some(name) = first_of_type(&rels, kind) else {
            return Ok(None);
        };
        Ok(tree_of(package, &name)?.map(|t| (name, t)))
    };
    Ok(Parts {
        styles: named("styles")?,
        numbering: named("numbering")?,
        settings: named("settings")?.map(|(_, t)| t),
        theme: named("theme")?.map(|(_, t)| t),
        main,
        rels,
        document,
    })
}

impl Model {
    /// A model for one document, its caches reserving from `budget`, its content digests
    /// keyed by `key` (the session's).
    pub fn new(budget: Arc<dyn CacheBudget>, key: DigestKey) -> Model {
        Model {
            key,
            digests: PartDigests::default(),
            bodies: ElementDigests::default(),
            context: None,
            elements: Memo::new(Arc::clone(&budget)),
            stories: Memo::new(Arc::clone(&budget)),
            notes: Memo::new(Arc::clone(&budget)),
            resolver: Resolver::new(&budget),
            label_keys: Vec::new(),
            block_keys: labels::LabelKeys::default(),
            labels: MonoidTree::new(),
            label_memo: Memo::new(Arc::clone(&budget)),
            names: keys::Names::default(),
            stats: OpenStats::default(),
            budget,
        }
    }

    /// What the last [`Model::open`] did.
    pub fn stats(&self) -> OpenStats {
        self.stats
    }

    /// Drop what the memos hold that no version opened since the last sweep read and
    /// nothing else holds ([`ooxml_derive::Memo::sweep`]): called once a version is
    /// opened and laid out, it keeps the memos at what the versions in use need, so they
    /// do not grow with every version. Every open reads every body element, story and
    /// note part, so what the newest version is made of stays; list items and
    /// resolutions stay while a version holds them. Returns how many entries were dropped.
    pub fn sweep(&mut self) -> usize {
        self.elements.sweep(|_| false)
            + self.stories.sweep(|_| false)
            + self.notes.sweep(|_| false)
            + self.label_memo.sweep(|items| Arc::strong_count(&items.items) > 1)
            + self.resolver.sweep()
    }

    /// The resolution memo's counters.
    pub fn resolver_stats(&self) -> MemoStats {
        self.resolver.stats()
    }

    /// The budget the caches reserve from.
    pub fn budget(&self) -> &Arc<dyn CacheBudget> {
        &self.budget
    }

    fn digest_of(&mut self, name: &str, tree: Option<&Tree>) -> Digest {
        tree.map_or_else(Digest::default, |t| self.digests.of(self.key, name, t))
    }

    /// The parts the blocks resolve against (reused while unchanged), and the digest of
    /// what the body's reading depends on (the theme's colours and their mapping).
    fn context(&mut self, p: &Parts) -> (Context, Digest) {
        let theme_d = self.digest_of("theme", p.theme.as_ref());
        let settings_d = self.digest_of("settings", p.settings.as_ref());
        let mut c = Digester::new(self.key);
        c.field(&theme_d.0);
        c.field(&settings_d.0);
        let colours_d = c.finish();
        let styles_d = self.digest_of("styles", p.styles.as_ref().map(|(_, t)| t));
        let numbering_d = self.digest_of("numbering", p.numbering.as_ref().map(|(_, t)| t));
        let mut d = Digester::new(self.key);
        for part in [theme_d, settings_d, styles_d, numbering_d] {
            d.field(&part.0);
        }
        let digest = d.finish();
        self.stats.context_changed = false;
        if let Some((known, cx)) = &self.context {
            if *known == digest {
                return (cx.clone(), colours_d);
            }
        }
        let cx = read_context(
            p.theme.as_ref(),
            p.settings.as_ref(),
            p.styles.as_ref().map(|(_, t)| t),
            p.numbering.as_ref().map(|(_, t)| t),
        );
        self.resolver.clear();
        self.label_memo.clear();
        self.label_keys.clear();
        self.labels = MonoidTree::new();
        self.stats.context_changed = true;
        self.context = Some((digest, cx.clone()));
        (cx, colours_d)
    }
}

/// The key of a top-level body element's reading: its content, the fields open at it,
/// the theme's colours, and its place unless it is a paragraph (a table's cells' paths
/// start with its own).
fn element_key(key: DigestKey, colours: Digest, content: Digest, fields: &Fields, place: Option<&str>) -> Digest {
    let mut d = Digester::new(key);
    d.field(&colours.0);
    d.field(&content.0);
    for open in fields.open_state() {
        d.field(&[
            u8::from(open.part == crate::read::fields::Part::Result),
            u8::from(open.computed),
        ]);
        d.field(open.keyword.as_bytes());
    }
    d.update(b"|");
    if let Some(place) = place {
        d.field(place.as_bytes());
    }
    d.finish()
}

/// Whether two lists hold the same blocks (the same `Arc`s).
fn same_blocks(a: &[Block], b: &[Block]) -> bool {
    a.len() == b.len()
        && a.iter().zip(b).all(|(x, y)| match (x, y) {
            (Block::Paragraph(p), Block::Paragraph(q)) => Arc::ptr_eq(p, q),
            (Block::Table(t), Block::Table(u)) => Arc::ptr_eq(t, u),
            _ => false,
        })
}

/// A paragraph taken from the memo, at its new place.
fn moved(block: &Block, here: &str) -> Block {
    match block {
        Block::Paragraph(p) if *p.path != *here => Block::Paragraph(Arc::new(Paragraph {
            path: text(here),
            ..(**p).clone()
        })),
        other => other.clone(),
    }
}

impl Model {
    fn body(
        &mut self,
        p: &Parts,
        cx: &Context,
        colours_d: Digest,
        warnings: &mut Vec<Text>,
    ) -> (Vec<Block>, Vec<Text>, Option<Arc<Element>>) {
        let reader = Reader::new(&cx.colours);
        let root = p.document.root();
        let Some(body) = elements(root).find(|e| local(e) == "body").cloned() else {
            warnings.push(text("document-no-body"));
            return (Vec::new(), Vec::new(), None);
        };
        let named = self.names.part(&p.main, "body", &p.document);
        let keyer = named.as_ref().map(|(ix, part)| keys::Keyer::new(*ix, part));
        let keyed = |blocks: &[Block], fresh: bool, out: &mut Vec<Block>| match keyer
            .as_ref()
            .and_then(|k| k.keyed(blocks, fresh))
        {
            Some(keyed) => out.extend(keyed),
            None => out.extend(blocks.iter().cloned()),
        };
        let mut fields = Fields::new(true, true);
        let mut steps = Steps::new("w:body");
        let mut out = Vec::new();
        let (mut count, mut read) = (0, 0);
        for node in elements(&body) {
            let here = steps.step_str(node);
            let name = local(node);
            if !matches!(name, "p" | "tbl" | "sdt" | "customXml") {
                let known = matches!(name, "sectPr") || MARKERS.contains(&name);
                let code = text(&format!("body-element-unsupported:{name}"));
                if !known && !warnings.contains(&code) {
                    warnings.push(code);
                }
                continue;
            }
            count += 1;
            let content = self.bodies.of(self.key, node);
            let paragraph = name == "p";
            // A paragraph taken to a new place is kept there too (by its content and its
            // place): the versions after it, where it stays, share its blocks.
            let placed = paragraph.then(|| element_key(self.key, colours_d, content, &fields, Some(here)));
            if let Some(Kept(kept)) = placed.as_ref().and_then(|placed| self.elements.get(placed)) {
                fields.cached.extend(kept.cached.iter().cloned());
                keyed(&kept.blocks, false, &mut out);
                continue;
            }
            let key = element_key(self.key, colours_d, content, &fields, (!paragraph).then_some(here));
            if let Some(Kept(kept)) = self.elements.get(&key) {
                fields.cached.extend(kept.cached.iter().cloned());
                match placed {
                    // A paragraph is kept by its content wherever it stands: it takes its
                    // new place. Any other element is kept at its own place (its key holds
                    // it), and the paragraphs in it keep their paths below it.
                    Some(placed) => {
                        let blocks: Arc<[Block]> = kept.blocks.iter().map(|b| moved(b, here)).collect();
                        keyed(&blocks, false, &mut out);
                        if !same_blocks(&blocks, &kept.blocks) {
                            let at_place = BodyElement {
                                blocks,
                                cached: Arc::clone(&kept.cached),
                            };
                            self.elements.insert(placed, Kept(Arc::new(at_place)));
                        }
                    }
                    None => keyed(&kept.blocks, false, &mut out),
                }
                continue;
            }
            read += 1;
            let before = fields.stack.clone();
            let mut element = reader.body_element(node, &text(here), &mut fields);
            let start = out.len();
            keyed(&element.blocks, true, &mut out);
            element.blocks = out.get(start..).map(Arc::from).unwrap_or_default();
            if fields.stack == before {
                self.elements.insert(key, Kept(Arc::new(element)));
            }
        }
        self.bodies.finish();
        self.stats.elements = count;
        self.stats.elements_read = read;
        (out, std::mem::take(&mut fields.cached), Some(body))
    }

    fn story(
        &mut self,
        package: &Package,
        main: &str,
        part: &str,
        colours_d: Digest,
        cx: &Context,
    ) -> Result<Option<StoryRead>, WordError> {
        let Some(tree) = tree_of(package, part)? else {
            return Ok(None);
        };
        let mut d = Digester::new(self.key);
        d.field(&colours_d.0);
        d.field(&self.digests.of(self.key, part, &tree).0);
        // its blocks are keyed in its own story: a part of the same content is another story
        d.field(part.as_bytes());
        let key = d.finish();
        let story = crate::ids::story_name(main, part);
        let named = self.names.part(part, &story, &tree);
        let keyer = named.as_ref().map(|(ix, part)| keys::Keyer::new(*ix, part));
        if let Some(Kept(kept)) = self.stories.get(&key) {
            let (blocks, cached) = (*kept).clone();
            return Ok(Some(match keyer.and_then(|k| k.keyed(&blocks, false)) {
                Some(keyed) => (Arc::from(keyed), cached),
                None => (blocks, cached),
            }));
        }
        let mut fields = Fields::new(false, true);
        let path = format!("w:{}", local(tree.root()));
        let mut blocks = Reader::new(&cx.colours).story(tree.root(), &path, &mut fields);
        if let Some(keyed) = keyer.and_then(|k| k.keyed(&blocks, true)) {
            blocks = keyed;
        }
        let made = (Arc::from(blocks), Arc::from(fields.cached));
        self.stories.insert(key, Kept(Arc::new(made.clone())));
        Ok(Some(made))
    }

    fn notes(
        &mut self,
        package: &Package,
        (main, part): (&str, Option<String>),
        kind: &str,
        colours_d: Digest,
        cx: &Context,
    ) -> Result<Notes, WordError> {
        let Some(part) = part else { return Ok(Notes::default()) };
        let Some(tree) = tree_of(package, &part)? else {
            return Ok(Notes::default());
        };
        let mut d = Digester::new(self.key);
        d.field(&colours_d.0);
        d.field(&self.digests.of(self.key, &part, &tree).0);
        d.field(kind.as_bytes());
        let key = d.finish();
        let story = crate::ids::story_name(main, &part);
        let named = self.names.part(&part, &story, &tree);
        let keyer = named.as_ref().map(|(ix, part)| keys::Keyer::new(*ix, part));
        let (notes, fresh) = match self.notes.get(&key) {
            Some(Kept(kept)) => ((*kept).clone(), false),
            None => (read_notes(&Reader::new(&cx.colours), tree.root(), kind), true),
        };
        let notes = match &keyer {
            Some(keyer) => keyed_notes(keyer, notes, fresh),
            None => notes,
        };
        if fresh {
            self.notes.insert(key, Kept(Arc::new(notes.clone())));
        }
        Ok(notes)
    }
}

/// `notes` with every note's and separator's blocks keyed (`fresh`: read now).
fn keyed_notes(keyer: &keys::Keyer<'_>, mut notes: Notes, fresh: bool) -> Notes {
    let keyed = |blocks: &mut Arc<[Block]>| {
        if let Some(made) = keyer.keyed(blocks, fresh) {
            *blocks = Arc::from(made);
        }
    };
    notes.notes.values_mut().for_each(keyed);
    notes.separators.values_mut().for_each(keyed);
    notes
}

fn read_notes(reader: &Reader<'_>, root: &Element, kind: &str) -> Notes {
    let mut out = Notes::default();
    for node in elements(root).filter(|n| local(n) == kind) {
        let view = crate::w::Note::new(node);
        let path = format!("w:{kind}[@w:id={}]", view.id().unwrap_or("None"));
        let mut blocks = reader.story(node, &path, &mut Fields::new(false, false));
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

impl Model {
    fn picture_bullets(
        &self,
        package: &Package,
        p: &Parts,
        cx: &Context,
    ) -> Result<BTreeMap<i64, PictureBullet>, WordError> {
        let mut out = BTreeMap::new();
        let Some((part, _)) = &p.numbering else { return Ok(out) };
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

    fn header_stories(
        &mut self,
        package: &Package,
        p: &Parts,
        sections: &[crate::model::Section],
        colours_d: Digest,
        cx: &Context,
        cached: &mut Vec<Text>,
    ) -> Result<Stories, WordError> {
        let (mut stories, mut parts_of) = (BTreeMap::new(), BTreeMap::new());
        let references = sections
            .iter()
            .flat_map(|s| s.properties.headers.iter().chain(s.properties.footers.iter()));
        for (_, rid) in references {
            if stories.contains_key(rid) {
                continue;
            }
            let Some(part) = p
                .rels
                .get(rid)
                .filter(|r| !r.external)
                .and_then(|r| r.target_part.clone())
            else {
                continue;
            };
            if !package.has_part(&part) {
                continue;
            }
            if let Some((blocks, fields)) = self.story(package, &p.main, &part, colours_d, cx)? {
                cached.extend(fields.iter().cloned());
                stories.insert(rid.clone(), blocks);
                parts_of.insert(rid.clone(), text(&part));
            }
        }
        Ok((stories, parts_of))
    }

    /// Open a version of the document, reusing what it shares with the last one opened.
    pub fn open(&mut self, package: &Package) -> Result<WordDocument, WordError> {
        let misses_before = self.resolver.stats().misses;
        let p = parts(package)?;
        let (cx, colours_d) = self.context(&p);
        let mut warnings = Vec::new();
        let (blocks, mut cached, body) = self.body(&p, &cx, colours_d, &mut warnings);
        let body_blocks: Arc<[Block]> = Arc::from(crate::read::body_story(blocks));
        let sections: Arc<[crate::model::Section]> = match &body {
            Some(b) => Arc::from(sections(
                &body_blocks,
                crate::w::Body::new(b).section().map(crate::w::SectPr::element),
            )),
            None => Arc::from(Vec::new()),
        };
        if p.styles.is_none() {
            warnings.push(text("styles-part-missing"));
        }
        let (stories, story_parts) = self.header_stories(package, &p, &sections, colours_d, &cx, &mut cached)?;
        let footnotes = self.notes(
            package,
            (&p.main, first_of_type(&p.rels, "footnotes")),
            "footnote",
            colours_d,
            &cx,
        )?;
        let endnotes = self.notes(
            package,
            (&p.main, first_of_type(&p.rels, "endnotes")),
            "endnote",
            colours_d,
            &cx,
        )?;
        let picture_bullets = self.picture_bullets(package, &p, &cx)?;
        self.update_labels(&cx, &body_blocks);
        let names = self.names.finish();
        self.stats.resolutions = self.resolver.stats().misses - misses_before;
        Ok(WordDocument {
            main_part: text(&p.main),
            context: cx,
            body: body_blocks,
            sections,
            stories,
            story_parts,
            footnotes,
            endnotes,
            picture_bullets,
            cached_fields: Arc::from(cached),
            warnings: Arc::from(warnings),
            labels: self.labels.clone(),
            names,
        })
    }

    /// A paragraph's resolved formats in `document` (a version this model opened).
    pub fn resolve_paragraph(&mut self, document: &WordDocument, paragraph: &Paragraph) -> ResolvedParagraph {
        self.resolver.paragraph(&document.context, paragraph)
    }

    /// A paragraph's own resolved properties.
    pub fn resolve_paragraph_props(&mut self, document: &WordDocument, paragraph: &Paragraph) -> Arc<Props> {
        self.resolver.paragraph_props(&document.context, paragraph)
    }

    /// A table's resolved formats in `document`.
    pub fn resolve_table(&mut self, document: &WordDocument, table: &Table) -> Arc<ResolvedTable> {
        self.resolver.table(&document.context, table)
    }
}

/// What blocks resolve against, read from the theme, settings, styles and numbering parts
/// (each where the package has it): the one reading of them, for the model's versions and
/// for any other reader of a version's formatting (`crate::docx`).
#[must_use]
pub fn read_context(
    theme: Option<&Tree>,
    settings: Option<&Tree>,
    styles: Option<&Tree>,
    numbering: Option<&Tree>,
) -> Context {
    let colours = ThemeColours {
        scheme: theme.map(|t| colour_scheme(t.root())).unwrap_or_default(),
        mapping: settings.map(|t| colour_mapping(t.root())).unwrap_or_default(),
        active: true,
    };
    let styles = styles.map(|t| Arc::new(StyleSheet::read(t.root(), &colours)));
    let numbering = numbering.map(|t| Numbering::read(t.root(), styles.as_deref(), &colours));
    Context {
        numbering: Arc::new(numbering.unwrap_or_default()),
        settings: Arc::new(settings.map(|t| Settings::read(t.root())).unwrap_or_default()),
        font_scheme: theme.and_then(|t| font_scheme(t.root())).map(Arc::new),
        colours: Arc::new(colours),
        styles,
    }
}
