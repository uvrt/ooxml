//! Owns: giving a document's blocks durable ids before its first edit, as docx-agent's
//! default stamping policy does (`Document._stamp_document`): every paragraph and row, in
//! every story, without a valid and unique `w14:paraId` gets a fresh paraId and textId from
//! its part's sequence, every drawing without an anchorId and editId gets them, and the
//! renames are recorded; a paragraph whose text changed gets a new textId
//! ([`new_text_id`]); and the invariant every version a command makes keeps
//! ([`Stamped`]): no paragraph, row or drawing of any story without a valid and unique id,
//! however the command wrote it.
//!
//! Must not: write but through the sink, or stamp a block twice (a stamped block's id is
//! valid, and is left alone).

use crate::doc::{Key, WordDoc};
use crate::names::{W14, WP, WP14};
use crate::policy::RevisionPolicy;
use crate::sink::{ensured_head, EditSink};
use crate::{EditError, Result};
use ooxml_store::{Command, Txn};
use ooxml_word_model::docx::{live_paths, Docx, StoryPart};
use ooxml_word_model::ids::{valid_long_hex, IdState, ParagraphId, Path, TableId};
use ooxml_word_model::ids::{BlockId, Renames};
use ooxml_xml::{Attribute, Element, Head, Name, Node, QName};
use std::collections::BTreeMap;
use std::ops::Bound;
use std::sync::Arc;

/// Attributes to set on an element, in order: each one's name (with the prefix it is
/// written with there) and value.
type Attrs = Vec<(Name, String)>;

/// A command, then every block it left without a durable id stamped, in its transaction,
/// when it changed anything. Word drops every paraId of a document holding one paragraph or
/// table row without a valid, unique paraId, or one drawing without its ids, when it saves
/// it (crates/ooxml-oracle/word/observations/paraid-durability.json, factors), and the
/// block ids rest on paraIds: so no version a command makes may hold one. Most commands
/// stamp the document before they edit it; this stamps what they wrote without ids (a
/// field's update, a part they rewrote), and costs nothing on a stamped story (its index
/// counts what lacks an id).
pub(crate) struct Stamped<'c, C>(pub(crate) &'c C);

impl<C: Command<WordDoc, Error = EditError>> Command<WordDoc> for Stamped<'_, C> {
    type Out = C::Out;
    type Error = EditError;

    fn validate(&self, doc: &WordDoc) -> Result<()> {
        self.0.validate(doc)
    }

    fn prepare(&self, tx: &mut Txn<'_, WordDoc>) -> Result<C::Out> {
        let out = self.0.prepare(tx)?;
        if !tx.changes().is_empty() {
            let mut sink = EditSink::new(tx, RevisionPolicy::Untracked);
            let docx = sink.docx()?;
            stamp_document(&mut sink, &docx)?;
        }
        Ok(out)
    }
}

/// Stamp every block of every story that needs an id; the renames, in the order made.
///
/// # Errors
/// When a part is not XML.
pub(crate) fn stamp_document(sink: &mut EditSink<'_, '_>, docx: &Docx) -> Result<Renames> {
    stamp_drawn(sink, docx, draw_pairs)
}

/// [`stamp_document`], each part's ids drawn by `draw` (a test draws them one element at a
/// time, as stamping once did, to show the bulk draw gives the same ids).
fn stamp_drawn(sink: &mut EditSink<'_, '_>, docx: &Docx, draw: DrawPairs) -> Result<Renames> {
    let mut renames = Renames::new();
    for story in &docx.parts {
        let made = stamp_part(sink, story, draw)?;
        if made.is_empty() {
            continue;
        }
        let known = sink.doc().ids().aliases().clone();
        let fresh: Renames = made
            .iter()
            .filter(|(k, _)| !known.contains_key(k.as_str()))
            .cloned()
            .collect();
        sink.rename(fresh)?;
        renames.extend(made);
    }
    Ok(renames)
}

/// The tables of `story` with a row without a durable id, each with the places of those
/// rows.
fn unstamped_rows(story: &StoryPart, prefix: &str) -> Vec<(TableId, Vec<usize>)> {
    let stamped = format!("{prefix}tr:");
    story
        .ids
        .tables()
        .iter()
        .filter_map(|table| {
            let rows: Vec<usize> = table
                .rows
                .iter()
                .enumerate()
                .filter(|(_, (_, id))| !id.starts_with(&stamped))
                .map(|(k, _)| k)
                .collect();
            (!rows.is_empty()).then_some((table, rows))
        })
        .collect()
}

/// The names of a pair of ids: their namespace, its usual prefix, and the two locals.
type Names = (&'static str, &'static str, [&'static str; 2]);

/// What draws a part's pairs of ids ([`draw_pairs`]).
type DrawPairs = fn(&mut EditSink<'_, '_>, &str, &Element, &[&[u32]], Names) -> Result<Vec<(Attrs, String)>>;

/// Two ids drawn from `seed` for each element of `paths`, in order, all at once (one draw
/// per seed: drawing them one element at a time copies the version's id maps per element);
/// for each, the attributes `locals` (in `namespace`, written with the prefix in scope
/// there) and the first id.
fn draw_pairs(
    sink: &mut EditSink<'_, '_>,
    seed: &str,
    root: &Element,
    paths: &[&[u32]],
    names: Names,
) -> Result<Vec<(Attrs, String)>> {
    let (namespace, usual, locals) = names;
    let ids = sink.draw(seed, 2 * paths.len())?;
    Ok(paths
        .iter()
        .zip(ids.as_chunks::<2>().0)
        .map(|(path, pair)| {
            let prefix = prefix_at(root, path, namespace, usual);
            let attrs: Attrs = locals
                .iter()
                .zip(pair)
                .map(|(local, value)| {
                    (
                        Arc::new(QName::new(Some(&prefix), local, Some(namespace))),
                        value.clone(),
                    )
                })
                .collect();
            let [first, _] = pair;
            (attrs, first.clone())
        })
        .collect())
}

const BLOCK_IDS: Names = (W14, "w14", ["paraId", "textId"]);
const DRAWING_IDS: Names = (WP14, "wp14", ["anchorId", "editId"]);

/// Stamp one story part; its renames.
fn stamp_part(sink: &mut EditSink<'_, '_>, story: &StoryPart, draw: DrawPairs) -> Result<Renames> {
    let part = story.part.as_str();
    let prefix = if story.story == "body" {
        String::new()
    } else {
        format!("{}/", story.story)
    };
    // the index counts what lacks an id: a stamped part is not walked
    let (paragraphs, rows): (Vec<ParagraphId>, _) = if story.ids.fully_stamped() {
        (Vec::new(), Vec::new())
    } else {
        let paragraphs = story
            .ids
            .paragraphs()
            .iter()
            .filter(|p| p.state != IdState::Ok)
            .collect();
        (paragraphs, unstamped_rows(story, &prefix))
    };
    // drawings are looked for only in a part whose memoised facts say it has one to stamp
    let drawings = if crate::validate::unstamped_drawings(sink.doc(), part)? {
        live_paths(story.root(), &[], &|e| {
            (e.name().is(Some(WP), "inline") || e.name().is(Some(WP), "anchor"))
                && !(e.attribute(Some(WP14), "anchorId").is_some_and(valid_long_hex)
                    && e.attribute(Some(WP14), "editId").is_some_and(valid_long_hex))
        })
    } else {
        Vec::new()
    };
    if paragraphs.is_empty() && rows.is_empty() && drawings.is_empty() {
        return Ok(Vec::new());
    }
    sink.read(Key::part(part));
    let root = story.root();
    let mut sets: Vec<(Box<[u32]>, Attrs)> = Vec::new();
    let mut renames = Renames::new();
    let paths: Vec<&[u32]> = paragraphs.iter().map(|p| &*p.path).collect();
    let drawn = draw(sink, part, root, &paths, BLOCK_IDS)?;
    for (paragraph, (attrs, para)) in paragraphs.iter().zip(drawn) {
        sets.push((paragraph.path.clone(), attrs));
        renames.push((BlockId::from(&paragraph.id), BlockId::from(format!("{prefix}p:{para}"))));
    }
    let table_rows: Vec<(&TableId, usize, &(Path, String))> = rows
        .iter()
        .flat_map(|(table, rows)| rows.iter().map(move |k| (table, *k)))
        .filter_map(|(table, k)| table.rows.get(k).map(|row| (table, k, row)))
        .collect();
    let paths: Vec<&[u32]> = table_rows.iter().map(|(_, _, (path, _))| &**path).collect();
    let drawn = draw(sink, &format!("{part}\0row"), root, &paths, BLOCK_IDS)?;
    let mut table_renames = Vec::new();
    for ((table, k, (path, row_id)), (attrs, para)) in table_rows.into_iter().zip(drawn) {
        sets.push((path.clone(), attrs));
        renames.push((BlockId::from(row_id), BlockId::from(format!("{prefix}tr:{para}"))));
        if k == 0 {
            table_renames.push((BlockId::from(&table.id), BlockId::from(format!("{prefix}t:{para}"))));
        }
    }
    renames.extend(table_renames);
    let paths: Vec<&[u32]> = drawings.iter().map(|d| &*d.path).collect();
    let drawn = draw(sink, &format!("{part}\0drawing"), root, &paths, DRAWING_IDS)?;
    for (drawing, (attrs, _)) in drawings.iter().zip(drawn) {
        sets.push((drawing.path.clone(), attrs));
    }
    let tree = story.tree.clone();
    let mut root = rewrite(
        tree.root(),
        &[],
        &sets.iter().map(|(p, s)| (&**p, s)).collect::<BTreeMap<_, _>>(),
    );
    if !paragraphs.is_empty() {
        root = ensured(&root, false);
    }
    root = ensured(&root, !drawings.is_empty());
    sink.put(part, tree.with_root(root))?;
    Ok(renames)
}

/// `root` with `w14` (and `wp14`) declared as docx-agent's `ensure_w14` leaves it.
fn ensured(root: &Arc<Element>, drawings: bool) -> Arc<Element> {
    match ensured_head(root, drawings, &[]) {
        None => Arc::clone(root),
        Some((head, declared)) => {
            let new = Arc::new(Element::from_head(Arc::new(head), root.children().clone()));
            if declared {
                crate::namespaces::cleaned(&new)
            } else {
                new
            }
        }
    }
}

/// The prefix `namespace`'s attributes are written with at the element at `path` below
/// `root`: the nearest declaration on the way down, else the namespace's usual prefix.
fn prefix_at(root: &Element, path: &[u32], namespace: &str, usual: &str) -> String {
    let declared = |e: &Element| {
        e.namespaces()
            .iter()
            .find(|d| &*d.uri == namespace && d.prefix.is_some())
            .and_then(|d| d.prefix.as_deref().map(str::to_owned))
    };
    let mut found = declared(root);
    let mut at = root;
    for k in path {
        let Some(child) = at.children().get(*k as usize).and_then(Node::as_element) else {
            break;
        };
        at = child;
        found = declared(at).or(found);
    }
    found.unwrap_or_else(|| usual.to_owned())
}

/// `element` (at `place`) with the attributes of `sets` set on the elements they name,
/// every unchanged subtree shared.
fn rewrite(element: &Arc<Element>, place: &[u32], sets: &BTreeMap<&[u32], &Attrs>) -> Arc<Element> {
    // the paths below `place` follow it in the map's order (one range, not every path:
    // walking all of them at every element is quadratic in a part's blocks)
    let below: Vec<u32> = sets
        .range::<[u32], _>((Bound::Excluded(place), Bound::Unbounded))
        .map(|(p, _)| *p)
        .take_while(|p| p.starts_with(place))
        .filter_map(|p| p.get(place.len()).copied())
        .collect();
    let mut children = element.children().clone();
    let mut done = std::collections::BTreeSet::new();
    for k in below {
        if !done.insert(k) {
            continue;
        }
        let Some(Node::Element(child)) = element.children().get(k as usize) else {
            continue;
        };
        let mut path = place.to_vec();
        path.push(k);
        let edited = rewrite(child, &path, sets);
        if let Some(spliced) = children.splice(k as usize..k as usize + 1, vec![Node::Element(edited)]) {
            children = spliced;
        }
    }
    match sets.get(place) {
        Some(attributes) => {
            let mut head: Head = (**element.head()).clone();
            for (name, value) in attributes.iter() {
                head = with_attribute(&head, name, value);
            }
            Arc::new(Element::from_head(Arc::new(head), children))
        }
        None => Arc::new(element.with_children(children)),
    }
}

/// `head` with the attribute `name` set to `value`: in its place when the element has it,
/// else after the others.
pub(crate) fn with_attribute(head: &Head, name: &Name, value: &str) -> Head {
    let mut attributes = head.attributes.to_vec();
    let local = name.local();
    match attributes.iter_mut().find(|a| a.name.is(name.namespace(), local)) {
        Some(a) => a.value = Arc::from(value),
        None => attributes.push(Attribute {
            name: Arc::clone(name),
            value: Arc::from(value),
        }),
    }
    Head {
        name: head.name.clone(),
        namespaces: head.namespaces.clone(),
        attributes: attributes.into(),
    }
}

/// The prefix the paragraph at `path` of `root` writes `w14` attributes with.
pub(crate) fn w14_prefix(root: &Element, path: &[u32]) -> String {
    prefix_at(root, path, W14, "w14")
}

/// The prefix the element at `path` of `root` writes `wp14` attributes with.
pub(crate) fn wp14_prefix(root: &Element, path: &[u32]) -> String {
    prefix_at(root, path, WP14, "wp14")
}

#[cfg(test)]
mod tests {
    //! Stamping draws each part's ids in one draw per seed, and gives every block the ids it
    //! got when each block's pair was drawn on its own: the same package bytes and renames on
    //! every fixture. And the first edit of a long unstamped document costs a few draws, not
    //! one per block (each draw copies the version's id maps).
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

    use super::*;
    use crate::doc::Change;
    use crate::validate::Unlimited;
    use crate::Editor;
    use ooxml_base::Limits;
    use ooxml_opc::Package;
    use ooxml_store::{Caller, Clock, StoreConfig};
    use std::path::PathBuf;
    use std::time::{Duration, Instant};

    struct Monotonic(Instant);

    impl Clock for Monotonic {
        fn monotonic(&self) -> Duration {
            self.0.elapsed()
        }
    }

    /// Stamp the whole document with `draw`; the renames and the draws written (each one's
    /// seed and how many ids it drew).
    struct StampWith(DrawPairs);

    type Draws = Vec<(Arc<str>, usize)>;

    impl Command<WordDoc> for StampWith {
        type Out = (Renames, Draws);
        type Error = EditError;

        fn validate(&self, _doc: &WordDoc) -> Result<()> {
            Ok(())
        }

        fn prepare(&self, tx: &mut Txn<'_, WordDoc>) -> Result<(Renames, Draws)> {
            let mut sink = EditSink::new(tx, RevisionPolicy::Untracked);
            let docx = sink.docx()?;
            let renames = stamp_drawn(&mut sink, &docx, self.0)?;
            let draws = tx
                .changes()
                .iter()
                .filter_map(|c| match c {
                    Change::Draw { seed, stand_ins } => Some((Arc::clone(seed), stand_ins.len())),
                    _ => None,
                })
                .collect();
            Ok((renames, draws))
        }
    }

    /// Each element's pair drawn on its own, as stamping drew them before the bulk draw.
    fn draw_singly(
        sink: &mut EditSink<'_, '_>,
        seed: &str,
        root: &Element,
        paths: &[&[u32]],
        names: Names,
    ) -> Result<Vec<(Attrs, String)>> {
        let mut out = Vec::with_capacity(paths.len());
        for path in paths {
            out.extend(draw_pairs(sink, seed, root, &[*path], names)?);
        }
        Ok(out)
    }

    fn editor(package: Package) -> Option<Editor> {
        Editor::open(
            package,
            StoreConfig::default(),
            Arc::new(Monotonic(Instant::now())),
            Arc::new(Unlimited),
        )
        .ok()
    }

    fn package(path: &std::path::Path) -> Package {
        let bytes: Arc<[u8]> = Arc::from(std::fs::read(path).unwrap());
        Package::open(bytes, &Limits::default()).unwrap()
    }

    fn fixtures(dir: &std::path::Path, out: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                fixtures(&path, out);
            } else if path.extension().is_some_and(|e| e == "docx") {
                out.push(path);
            }
        }
    }

    /// A package's parts: each one's name and bytes.
    type Parts = Vec<(String, Arc<[u8]>)>;

    /// Every part's bytes, by name.
    fn parts(doc: &WordDoc) -> Parts {
        let package = doc.package();
        package
            .part_names()
            .into_iter()
            .map(|name| {
                let bytes = package.read(&name).unwrap().unwrap();
                (name, bytes)
            })
            .collect()
    }

    /// The stamped package, the renames and the draws written.
    fn stamped(package: &Package, draw: DrawPairs) -> Option<(Parts, Renames, Draws)> {
        let editor = editor(package.clone())?;
        let done = editor.store().run(&Caller::anonymous(), &StampWith(draw)).unwrap();
        let (renames, draws) = done.out;
        Some((parts(done.version.doc()), renames, draws))
    }

    #[test]
    fn the_bulk_draw_stamps_every_fixture_as_one_draw_per_block_did() {
        let oracle = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../crates/ooxml-oracle");
        let mut files = Vec::new();
        fixtures(&oracle.join("fixtures"), &mut files);
        fixtures(&oracle.join("corpus"), &mut files);
        files.sort();
        assert!(files.len() > 200);
        // ids drawn for paragraphs, rows and drawings
        let (mut paragraphs, mut rows, mut drawings) = (0, 0, 0);
        for file in files {
            let package = package(&file);
            let Some((bulk, renames, draws)) = stamped(&package, draw_pairs) else {
                continue;
            };
            let (single, single_renames, _) = stamped(&package, draw_singly).unwrap();
            let what = file.display();
            assert_eq!(renames, single_renames, "{what}: renames");
            assert_eq!(bulk.len(), single.len(), "{what}: parts");
            for ((name, bytes), (other, other_bytes)) in bulk.iter().zip(&single) {
                assert_eq!(name, other, "{what}");
                assert!(bytes == other_bytes, "{what}: {name} differs");
            }
            // one draw per seed: a part's paragraphs, its rows, its drawings
            let mut seeds: Vec<&str> = draws.iter().map(|(seed, _)| &**seed).collect();
            seeds.sort_unstable();
            seeds.dedup();
            assert_eq!(seeds.len(), draws.len(), "{what}: {draws:?}");
            for (seed, count) in &draws {
                match seed.split_once('\0') {
                    None => paragraphs += count / 2,
                    Some((_, "row")) => rows += count / 2,
                    Some(_) => drawings += count / 2,
                }
            }
        }
        // the fixtures and corpus stamp all three kinds
        assert!(
            paragraphs > 500 && rows > 50 && drawings > 10,
            "{paragraphs} paragraphs, {rows} rows, {drawings} drawings"
        );
    }

    #[test]
    fn stamping_twenty_thousand_paragraphs_draws_once_per_seed() {
        let base = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../crates/ooxml-oracle/fixtures/samplelib/sample-long.docx");
        let base = package(&base);
        let blocks = 20_000;
        let mut xml = String::from(
            "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
             <w:document xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\"><w:body>",
        );
        for k in 0..blocks {
            xml.push_str(&format!("<w:p><w:r><w:t>Paragraph {k}</w:t></w:r></w:p>"));
        }
        xml.push_str("<w:tbl><w:tr><w:tc><w:p/></w:tc></w:tr><w:tr><w:tc><w:p/></w:tc></w:tr></w:tbl>");
        xml.push_str("<w:sectPr/></w:body></w:document>");
        let package = base.with_bytes("word/document.xml", Arc::from(xml.into_bytes()));
        let started = Instant::now();
        let (_, renames, draws) = stamped(&package, draw_pairs).unwrap();
        let draws = draws.len();
        let took = started.elapsed();
        eprintln!("stamped {} blocks in {took:?}", renames.len());
        assert!(renames.len() > blocks, "{} renames", renames.len());
        // one draw per seed (body paragraphs, body rows, and the other stories' parts),
        // however many blocks: not one per block
        assert!(draws < 50, "{draws} draws for {} renames", renames.len());
        // generous (one draw per block took minutes in a debug build)
        assert!(took < Duration::from_secs(120), "stamping took {took:?}");
    }
}
