//! A block projected alone held to its story's projection on the corpus: (1) its cells'
//! facts -- every paragraph's cell, every table's resolved formats -- equal the story's
//! everywhere; (2) the whole block equals the story's where its context is the one it
//! assumes ([`super::Assumed`]); and a table's resolution from its structure alone
//! (`docx::cells::resolved_shape`) equals its resolution through the projection, but for
//! a table the final view parts (content moved out of it: its shape is all of it).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use super::{block_place, visit, BlockAlone};
use crate::docx::cells::resolved_shape;
use crate::docx::formatting::{Cell, Formatting};
use crate::docx::{at_path, Docx};
use crate::ids::BlockKey;
use crate::model::Block;
use crate::projection::Projection;
use crate::resolve::resolve_table;
use crate::view::View;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::Arc;

fn docx_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if !path.file_name().is_some_and(|n| n == "target") {
                docx_files(&path, out);
            }
        } else if path.extension().is_some_and(|e| e == "docx") {
            out.push(path);
        }
    }
}

/// Every paragraph's cell in `blocks`, by its key.
fn cells(blocks: &[Block]) -> HashMap<BlockKey, Option<Cell>> {
    let mut out = HashMap::new();
    visit(blocks, &mut |block| {
        if let Block::Paragraph(p) = block {
            if let Some(key) = p.key {
                let cell = p.table_style_id.as_ref().map(|style| Cell {
                    style: style.clone(),
                    conditions: Arc::clone(&p.table_conditions),
                });
                out.entry(key).or_insert(cell);
            }
        }
    });
    out
}

/// Every table in `blocks`, by its key (its first part where the view parts it).
fn tables(blocks: &[Block]) -> HashMap<BlockKey, Arc<crate::model::Table>> {
    let mut out = HashMap::new();
    visit(blocks, &mut |block| {
        if let Block::Table(t) = block {
            if let Some(key) = t.key {
                out.entry(key).or_insert_with(|| Arc::clone(t));
            }
        }
    });
    out
}

/// How many parts of the table `key` `blocks` hold (more than one where the final view
/// moves content out of it).
fn parts_of(blocks: &[Block], key: BlockKey) -> usize {
    let mut count = 0;
    visit(blocks, &mut |block| {
        if matches!(block, Block::Table(t) if t.key == Some(key)) {
            count += 1;
        }
    });
    count
}

#[derive(Debug, Default)]
struct Tally {
    documents: usize,
    blocks: usize,
    paragraphs: usize,
    tables: usize,
    whole_equal: usize,
    whole_differ: usize,
    /// The whole tables differing where no paragraph before them joined in: none.
    whole_differ_at: Vec<String>,
    shapes: usize,
    shapes_differ: Vec<String>,
    /// Of the shapes differing, tables the final view parts (content moved out of them).
    shapes_parted: usize,
    /// Of the whole tables differing, those a paragraph before them joined into.
    joined_in: usize,
}

#[test]
fn a_block_projected_alone_has_its_story_s_cells_and_a_table_resolves_from_its_shape_alike() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut files = Vec::new();
    docx_files(&root, &mut files);
    files.sort();
    let mut seen = HashSet::new();
    let mut tally = Tally::default();
    for file in &files {
        let Ok(bytes) = std::fs::read(file) else { continue };
        let mut h = std::collections::hash_map::DefaultHasher::new();
        bytes.hash(&mut h);
        if !seen.insert(h.finish()) {
            continue;
        }
        let Ok(package) = ooxml_opc::Package::open(Arc::from(bytes), &ooxml_base::Limits::default()) else {
            continue;
        };
        let Ok(docx) = Docx::open(
            &crate::WordPackage::open(package).0,
            Arc::new(ooxml_derive::Limit::new(usize::MAX)),
        ) else {
            continue;
        };
        tally.documents += 1;
        let formatting = Formatting::read(&docx);
        for part in &docx.parts {
            let story = part.story.as_str();
            if !(story == "body" || story.starts_with("header") || story.starts_with("footer")) {
                continue;
            }
            let Ok(Some(full)) = Projection::story(&docx, &part.part, View::Final) else {
                continue;
            };
            let (full_cells, full_tables) = (cells(&full), tables(&full));
            let tops: BTreeSet<Vec<u32>> = part
                .ids
                .tables()
                .iter()
                .filter_map(|t| block_place(part.root(), &t.path))
                .collect();
            for top in tops {
                let alone: BlockAlone = Projection::block(&docx, &part.part, &top, View::Final)
                    .unwrap()
                    .unwrap();
                tally.blocks += 1;
                let name = format!("{} {} {top:?}", file.display(), part.part);
                // (1) the cells' facts
                for (key, cell) in cells(alone.blocks()) {
                    tally.paragraphs += 1;
                    assert_eq!(full_cells.get(&key), Some(&cell), "{name}: the cell of {key:?}");
                }
                for (key, table) in &tables(alone.blocks()) {
                    tally.tables += 1;
                    let full_table = full_tables.get(key).unwrap_or_else(|| panic!("{name}: {key:?}"));
                    one_table(
                        &Story {
                            part,
                            full: &full,
                            formatting: &formatting,
                        },
                        (&name, *key),
                        (table, full_table),
                        &mut tally,
                    );
                }
            }
        }
    }
    eprintln!("block alone: {tally:?}");
    assert!(tally.blocks > 100, "{tally:?}");
    assert!(tally.shapes_differ.is_empty(), "{:#?}", tally.shapes_differ);
    assert!(tally.whole_differ_at.is_empty(), "{:#?}", tally.whole_differ_at);
}

/// A story's projection, and what a table of it is read with.
struct Story<'a> {
    part: &'a crate::docx::StoryPart,
    full: &'a [Block],
    formatting: &'a Formatting,
}

/// One table projected alone (`table`) against the story's (`full_table`).
fn one_table(
    story: &Story<'_>,
    (name, key): (&str, BlockKey),
    (table, full_table): (&crate::model::Table, &crate::model::Table),
    tally: &mut Tally,
) {
    let sources = story.formatting.sources();
    assert_eq!(
        resolve_table(&sources, table),
        resolve_table(&sources, full_table),
        "{name}: {key:?} resolved"
    );
    // the table resolved from its structure alone
    let place = story.part.ids.place_of(key).unwrap();
    let element = at_path(story.part.root(), &place).unwrap();
    tally.shapes += 1;
    if resolved_shape(story.formatting, element) != Some(resolve_table(&sources, table)) {
        if parts_of(story.full, key) > 1 {
            tally.shapes_parted += 1;
        } else {
            tally.shapes_differ.push(format!("{name}: {key:?}"));
        }
    }
    // (2) the whole table, where its context is the one assumed
    if table == full_table {
        tally.whole_equal += 1;
    } else {
        tally.whole_differ += 1;
        if full_table.joined && !table.joined {
            tally.joined_in += 1;
        } else {
            tally.whole_differ_at.push(format!("{name}: {key:?}"));
        }
    }
}
