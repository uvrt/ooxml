//! Block identity's round trips (phase 3 design, section 5 item 2), over every story of
//! every corpus document and over random lineages of made-up parts: every id the index
//! names reads back as written (`BlockId::parse(x).to_string() == x`), odd forms of it too
//! (lower case, short hex, `#k`, `@n`, `/rK`); every paragraph's, table's and row's key
//! finds that block again (`entry_of(key_of(e)) == e`) with its place and id; and every
//! block's name at the edge (`VersionNames::path`) is the path the layout model writes today
//! (`story_model_path`'s, a note's blocks named from their note by its id).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]

use crate::block_index::{document, edited, Rng};
use crate::support::Unlimited;
use ooxml_word_model::docx::cells::story_model_path;
use ooxml_word_model::docx::{Docx, Kept, StoryPart, VersionNames};
use ooxml_word_model::ids::{
    Anchor, BlockId, BlockKey, BlockKind, BlockName, Entry, IdState, StoryIx, StoryName, StoryTable, Sub, W,
};
use ooxml_xml::{Element, Tree};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// The layout model's path of the element at `place` (the reference): `story_model_path`'s,
/// a note's blocks named from their note by its id (`w:footnote[@w:id=2]/w:p[1]`), as the
/// model reads a note as a story of its own.
fn layout_path(root: &Arc<Element>, place: &[u32]) -> Option<String> {
    let path = story_model_path(root, place)?;
    let kind = match root.name().local() {
        "footnotes" => "footnote",
        "endnotes" => "endnote",
        _ => return Some(path),
    };
    let note = root.children().get(*place.first()? as usize)?.as_element()?;
    let id = note.attribute(Some(W), "id").unwrap_or("None");
    let rest = path.strip_prefix(&format!("w:{}s/", kind))?;
    let below = rest.split_once('/').map_or("", |(_, below)| below);
    let named = format!("w:{kind}[@w:id={id}]");
    Some(if below.is_empty() {
        named
    } else {
        format!("{named}/{below}")
    })
}

/// What the checks saw.
#[derive(Default, Debug)]
struct Tally {
    paragraphs: usize,
    tables: usize,
    rows: usize,
    cells: usize,
    durable: usize,
    odd_forms: usize,
    paths: usize,
}

/// About how many paragraphs' paths are checked against the reference per story.
const PATHS_CHECKED: usize = 1_500;

/// `text`, an id the index named for a block of kind `kind` in `story`, read back, and its
/// odd forms read as what they are.
fn check_id(text: &str, kind: BlockKind, story: &StoryName, tally: &mut Tally) {
    let parsed = BlockId::parse(text).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(parsed.to_string(), text);
    assert_eq!(parsed.kind(), Some(kind), "{text}");
    assert_eq!(parsed.story(), Some(story), "{text}");
    let name = parsed.name().unwrap();
    if let Some(made) = BlockId::of(story, kind, name) {
        // the index's spelling of the same name: equal where the paraId was written so
        assert_eq!(made.name(), Some(name), "{text}");
        assert!(
            made.as_str().eq_ignore_ascii_case(text) || made.as_str().len() > text.len(),
            "{text}"
        );
    }
    let prefix = if story.is_body() {
        String::new()
    } else {
        format!("{story}/")
    };
    let mut odd: Vec<(String, BlockKind, BlockName)> = Vec::new();
    odd.push((format!("p@{story}/7"), BlockKind::Paragraph, BlockName::Ordinal(7)));
    odd.push((format!("t@{story}/0"), BlockKind::Table, BlockName::Ordinal(0)));
    if let BlockName::Durable(id) = name {
        let tag = match kind {
            BlockKind::Paragraph => "p",
            BlockKind::Table => "t",
            BlockKind::Row => "tr",
        };
        let short = format!("{prefix}{tag}:{:x}", id.get());
        odd.push((short, kind, name.clone()));
        let lower = format!("{prefix}{tag}:{:08x}", id.get());
        odd.push((lower, kind, name.clone()));
        match kind {
            BlockKind::Paragraph => {
                for k in [1, 12] {
                    odd.push((format!("{text}#{k}"), kind, BlockName::Repeat(Some(*id), k)));
                }
            }
            BlockKind::Table => {
                for row in [0, 3] {
                    let at = BlockName::RowAt {
                        table: Box::new(name.clone()),
                        row,
                    };
                    odd.push((format!("{text}/r{row}"), BlockKind::Row, at));
                }
            }
            BlockKind::Row => {}
        }
    }
    if let BlockName::Ordinal(n) = name {
        if kind == BlockKind::Table {
            let at = BlockName::RowAt {
                table: Box::new(BlockName::Ordinal(*n)),
                row: 2,
            };
            odd.push((format!("{text}/r2"), BlockKind::Row, at));
        }
    }
    for (form, kind, name) in odd {
        let read = BlockId::parse(&form).unwrap_or_else(|e| panic!("{e} (from {text})"));
        assert_eq!(read.to_string(), form);
        assert_eq!(
            (read.kind(), read.story(), read.name()),
            (Some(kind), Some(story), Some(&name)),
            "{form}"
        );
        tally.odd_forms += 1;
    }
}

/// Every block of `part` (the story `ix` of `names`) found again by its key, with its place,
/// id and path.
fn check_part(names: &VersionNames, ix: StoryIx, part: &StoryPart, tally: &mut Tally) {
    // the reference walks the siblings before each step: in a long story, every k-th block
    let every = (part.ids.paragraphs().len() / PATHS_CHECKED).max(1);
    let story = Story {
        names,
        ix,
        part,
        name: StoryName::new(&part.story),
        every,
        short: short_written(&part.ids),
    };
    let mut keys: HashSet<BlockKey> = HashSet::new();
    let seen = check_paragraphs(&story, &mut keys, tally) + check_tables(&story, &mut keys, tally);
    assert_eq!(keys.len(), seen, "keys are unique in a story");
}

/// A story under check.
struct Story<'a> {
    names: &'a VersionNames,
    ix: StoryIx,
    part: &'a StoryPart,
    name: StoryName,
    /// Paths are checked for every `every`-th block.
    every: usize,
    /// Whether a paraId is written with fewer than eight digits.
    short: bool,
}

impl Story<'_> {
    fn sampled(&self, k: usize) -> bool {
        k.is_multiple_of(self.every)
    }
}

fn check_paragraphs(story: &Story<'_>, keys: &mut HashSet<BlockKey>, tally: &mut Tally) -> usize {
    let (names, ix, ids, root) = (story.names, story.ix, &story.part.ids, story.part.root());
    let mut seen = 0;
    for p in ids.paragraphs().iter() {
        let entry = Entry::Paragraph(p.index);
        let key = ids.key_of(ix, entry).expect("a paragraph has a key");
        assert_eq!((key.story(), key.kind()), (ix, BlockKind::Paragraph));
        assert_eq!(ids.entry_of(key), Some(entry), "{}", p.id);
        assert_eq!(ids.row_of(key), None);
        assert_eq!(ids.place_of(key).as_deref(), Some(&*p.path), "{}", p.id);
        assert_eq!(names.id(key).map(String::from), Some(p.id.clone()));
        if story.sampled(p.index) {
            let path = layout_path(root, &p.path);
            assert_eq!(names.path(Anchor::whole(key)), path, "{}", p.id);
            let label = Anchor {
                block: key,
                sub: Sub::Label,
            };
            assert_eq!(names.path(label), path.map(|b| format!("{b}/label")));
            tally.paths += 1;
        }
        let durable = key.durable();
        assert!(!durable || p.state == IdState::Ok, "{}", p.id);
        if p.state == IdState::Ok && p.raw.as_deref().is_some_and(|r| r.len() == 8) && !story.short {
            assert!(durable, "{}", p.id);
        }
        tally.durable += usize::from(durable);
        check_id(&p.id, BlockKind::Paragraph, &story.name, tally);
        keys.insert(key);
        seen += 1;
        tally.paragraphs += 1;
    }
    seen
}

fn check_tables(story: &Story<'_>, keys: &mut HashSet<BlockKey>, tally: &mut Tally) -> usize {
    let (names, ix, ids, root) = (story.names, story.ix, &story.part.ids, story.part.root());
    let mut seen = 0;
    for (n, table) in ids.tables().iter().enumerate() {
        let entry = Entry::Table(n);
        let key = ids.key_of(ix, entry).expect("a table has a key");
        assert_eq!(ids.entry_of(key), Some(entry), "{}", table.id);
        assert_eq!(ids.place_of(key).as_deref(), Some(&*table.path));
        assert_eq!(names.id(key).map(String::from), Some(table.id.clone()));
        assert_eq!(names.path(Anchor::whole(key)), layout_path(root, &table.path));
        assert_eq!(
            names.path(Anchor {
                block: key,
                sub: Sub::Label
            }),
            None
        );
        tally.durable += usize::from(key.durable());
        check_id(&table.id, BlockKind::Table, &story.name, tally);
        keys.insert(key);
        seen += 1;
        tally.tables += 1;
        for (r, (path, id)) in table.rows.iter().enumerate() {
            let row = ids.row_key_of(ix, n, r).expect("a row has a key");
            assert_eq!(row.kind(), BlockKind::Row);
            assert_eq!(ids.row_of(row), Some((n, r)), "{id}");
            assert_eq!(ids.entry_of(row), None);
            assert_eq!(ids.place_of(row).as_deref(), Some(&**path));
            assert_eq!(names.id(row).map(String::from), Some(id.clone()));
            if story.sampled(r) {
                assert_eq!(names.path(Anchor::whole(row)), layout_path(root, path), "{id}");
                tally.paths += 1;
            }
            check_id(id, BlockKind::Row, &story.name, tally);
            keys.insert(row);
            seen += 1;
            tally.rows += 1;
        }
        if story.sampled(n) {
            tally.cells += check_cells(names, key, root, &table.path);
        }
    }
    seen
}

/// Each cell of a table without row- or cell-level wrappers named as the model names it.
fn check_cells(names: &VersionNames, key: BlockKey, root: &Arc<Element>, place: &[u32]) -> usize {
    let table = place
        .iter()
        .try_fold(root, |node, k| node.children().get(*k as usize)?.as_element())
        .unwrap();
    let wrapped = |e: &Element| {
        e.child_elements()
            .any(|c| matches!(c.name().local(), "sdt" | "customXml"))
    };
    if wrapped(table) {
        return 0;
    }
    let mut count = 0;
    let rows = table.children().iter().enumerate().filter_map(|(k, n)| {
        let e = n.as_element()?;
        e.name().is(Some(W), "tr").then_some((k, e))
    });
    for (r, (tr_at, tr)) in rows.enumerate() {
        if wrapped(tr) {
            continue;
        }
        let cells = tr.children().iter().enumerate().filter_map(|(k, n)| {
            let e = n.as_element()?;
            e.name().is(Some(W), "tc").then_some(k)
        });
        for (c, tc_at) in cells.enumerate() {
            let mut at = place.to_vec();
            at.extend([u32::try_from(tr_at).unwrap(), u32::try_from(tc_at).unwrap()]);
            let sub = Sub::Cell(u16::try_from(r).unwrap(), u16::try_from(c).unwrap());
            assert_eq!(names.path(Anchor { block: key, sub }), layout_path(root, &at));
            count += 1;
        }
    }
    count
}

/// Whether some paraId of the part is written with fewer than eight digits.
fn short_written(ids: &ooxml_word_model::ids::PartIds) -> bool {
    let short = |raw: &Option<String>| raw.as_deref().is_some_and(|r| r.len() < 8);
    ids.paragraphs().iter().any(|p| short(&p.raw))
        || ids.tables().iter().any(|t| {
            t.rows
                .iter()
                .any(|(_, id)| id.rsplit(':').next().is_some_and(|r| r.len() < 8))
        })
}

fn docx_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            docx_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "docx") {
            out.push(path);
        }
    }
}

#[test]
fn every_block_of_every_corpus_story_round_trips_by_key_id_and_path() {
    let oracle = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../ooxml-oracle");
    let mut files = Vec::new();
    docx_files(&oracle, &mut files);
    files.sort();
    let (mut documents, mut stories_seen) = (0, 0);
    let mut tally = Tally::default();
    for file in &files {
        let Ok(bytes) = std::fs::read(file) else { continue };
        let Ok(package) = ooxml_opc::Package::open(Arc::from(bytes), &ooxml_base::Limits::default()) else {
            continue;
        };
        let Ok(docx) = Docx::open(&package) else { continue };
        let mut stories = StoryTable::new();
        let names = VersionNames::new(&docx, &mut stories);
        for part in &docx.parts {
            let ix = stories.get(&part.story).unwrap();
            assert!(std::ptr::eq(&**names.part(ix).unwrap(), &**part));
            check_part(&names, ix, part, &mut tally);
            stories_seen += 1;
        }
        documents += 1;
    }
    assert!(
        documents > 300 && stories_seen > documents,
        "{documents} documents, {stories_seen} stories"
    );
    assert!(
        tally.paragraphs > 10_000 && tally.rows > 1_000 && tally.cells > 1_000 && tally.durable > 2_000,
        "{tally:?}"
    );
}

/// A part's version as a story part: `earlier`'s index followed where given.
fn story_part(name: &str, root: &Arc<Element>, earlier: Option<&StoryPart>, kept: &Arc<Kept>) -> StoryPart {
    let part = if name == "body" {
        "word/document.xml"
    } else {
        "word/header1.xml"
    };
    StoryPart::new(
        part.into(),
        name.into(),
        Tree::from_root(Arc::clone(root)),
        earlier,
        kept,
    )
}

#[test]
fn keys_round_trip_in_every_version_of_random_lineages_and_durable_ones_survive_edits() {
    let kept = Arc::new(Kept::new(Arc::new(Unlimited)));
    let mut tally = Tally::default();
    for seed in 1..=15u64 {
        let mut rng = Rng(0xD1B5_4A32_D192_ED03 ^ seed.wrapping_mul(0x9E37_79B9_7F4A_7C15));
        let story = seed % 3 == 0;
        let name = if story { "header1" } else { "body" };
        let mut stories = StoryTable::new();
        let ix = stories.intern(name).unwrap();
        let blocks = 10 + rng.below(60);
        let mut root = document(&mut rng, blocks, story);
        let mut part = Arc::new(story_part(name, &root, None, &kept));
        for _ in 0..10 {
            let names = VersionNames::from_parts([(ix, Arc::clone(&part))]);
            check_part(&names, ix, &part, &mut tally);
            let next = edited(&mut rng, &root, story);
            let after = Arc::new(story_part(name, &next, Some(&part), &kept));
            // a durable key names the block with the same paraId after the edit, wherever it is
            for p in part.ids.paragraphs().iter() {
                let key = part.ids.key_of(ix, Entry::Paragraph(p.index)).unwrap();
                if !key.durable() {
                    continue;
                }
                if let Some(id) = after.ids.id_of_key(key) {
                    assert!(id.as_str().eq_ignore_ascii_case(&p.id), "{id} was {}", p.id);
                }
            }
            (root, part) = (next, after);
        }
    }
    assert!(
        tally.durable > 50 && tally.rows > 50 && tally.odd_forms > 1_000,
        "{tally:?}"
    );
}

/// The lookups' costs on one document (`OOXML_NAMES_DOCX`), in a release build:
/// `cargo test --release -p ooxml-word-model --test integration names_cost -- --ignored --nocapture`.
#[test]
#[ignore = "a measurement: needs OOXML_NAMES_DOCX"]
fn names_cost() {
    use std::time::Instant;
    let Ok(file) = std::env::var("OOXML_NAMES_DOCX") else {
        return;
    };
    let bytes = std::fs::read(file).unwrap();
    let package = ooxml_opc::Package::open(Arc::from(bytes), &ooxml_base::Limits::default()).unwrap();
    let docx = Docx::open(&package).unwrap();
    let mut stories = StoryTable::new();
    let names = VersionNames::new(&docx, &mut stories);
    let body = names.part(StoryIx::BODY).unwrap();
    let ids = &body.ids;
    let n = ids.paragraphs().len();
    let each = |t: Instant, k: usize| t.elapsed().as_secs_f64() * 1e9 / k.max(1) as f64;
    let t = Instant::now();
    let keys: Vec<BlockKey> = (0..n)
        .filter_map(|k| ids.key_of(StoryIx::BODY, Entry::Paragraph(k)))
        .collect();
    let key_of = each(t, n);
    let t = Instant::now();
    let found = keys.iter().filter(|k| ids.entry_of(**k).is_some()).count();
    let entry_of = each(t, n);
    let t = Instant::now();
    let named: Vec<BlockId> = keys.iter().filter_map(|k| names.id(*k)).collect();
    let id = each(t, n);
    let t = Instant::now();
    let paths: Vec<String> = keys.iter().filter_map(|k| names.path(Anchor::whole(*k))).collect();
    let path = each(t, n);
    let t = Instant::now();
    let parsed = named.iter().filter(|i| BlockId::parse(i).is_ok()).count();
    let parse = each(t, n);
    let t = Instant::now();
    let by_text = named.iter().filter(|i| ids.get(i).is_some()).count();
    let get = each(t, n);
    let rows: Vec<(usize, usize)> = ids
        .tables()
        .iter()
        .enumerate()
        .flat_map(|(t, table)| (0..table.rows.len()).map(move |r| (t, r)))
        .collect();
    let t = Instant::now();
    let row_keys: Vec<BlockKey> = rows
        .iter()
        .filter_map(|(t, r)| ids.row_key_of(StoryIx::BODY, *t, *r))
        .collect();
    let row_key = each(t, rows.len());
    let t = Instant::now();
    let row_paths = row_keys.iter().filter_map(|k| names.path(Anchor::whole(*k))).count();
    let row_path = each(t, rows.len());
    println!(
        "{{\"paragraphs\":{n},\"rows\":{},\"durable\":{},\"key_of_ns\":{key_of:.0},\"entry_of_ns\":{entry_of:.0},\"id_ns\":{id:.0},\"path_ns\":{path:.0},\"parse_ns\":{parse:.0},\"get_str_ns\":{get:.0},\"row_key_of_ns\":{row_key:.0},\"row_path_ns\":{row_path:.0},\"checks\":[{found},{parsed},{by_text},{},{row_paths}]}}",
        rows.len(),
        keys.iter().filter(|k| k.durable()).count(),
        paths.len(),
    );
}
