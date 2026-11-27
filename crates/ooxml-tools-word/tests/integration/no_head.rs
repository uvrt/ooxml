//! No new head access in a handler (user decision 2026-11-25, plan-time races): a tool plans
//! its edit on the version its call's transaction reads, never on the document's head read
//! beside it (`CallEditor::current`, the store's snapshot), where a commit landing in between
//! goes unseen. The host's `CallEnv` reaches no head; this test holds the Word tools to it:
//! every place in `src` that reads a head is one of the known sites below (the 11 handler
//! sites lanes fix-plan-a and fix-plan-b are moving into the transaction, the reply-only
//! sites, the editor's own definitions and test code), and a file may only lose them. A new
//! one fails here: plan through the transaction (`ooxml_word_edit::planning`) instead.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::collections::BTreeMap;
use std::path::Path;

/// What reads a head: the call editor's current version, the store, a store's snapshot.
const HEAD: [&str; 3] = [".current()", ".store()", ".snapshot()"];

/// The known sites, by file under `src`: the most each may hold.
const KNOWN: [(&str, usize); 21] = [
    ("objects/controls.rs", 1),
    ("objects/drawings.rs", 1),
    ("objects/drawings_picture.rs", 9),
    ("objects/edit_table.rs", 3),
    // edit_chart's add: the language new chart text is written in (Word's GraphicWriter)
    ("objects/graphics.rs", 1),
    ("objects/planned_tests.rs", 22),
    ("planned.rs", 10),
    ("review/call_editor.rs", 3),
    ("review/changes.rs", 1),
    ("review/comments.rs", 1),
    ("review/tracking.rs", 1),
    ("structure/fields.rs", 1),
    ("structure/links.rs", 1),
    ("structure/notes.rs", 1),
    ("structure/sections.rs", 1),
    ("style/follow.rs", 1),
    ("style/format.rs", 1),
    ("style/styles.rs", 1),
    ("style/template.rs", 1),
    ("text/delete.rs", 2),
    ("text/set_text.rs", 2),
];

fn walk(dir: &Path, root: &Path, found: &mut BTreeMap<String, usize>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            walk(&path, root, found);
        } else if path.extension().is_some_and(|e| e == "rs") {
            let text = std::fs::read_to_string(&path).unwrap();
            let count: usize = HEAD.iter().map(|pattern| text.matches(pattern).count()).sum();
            if count > 0 {
                let name = path.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/");
                found.insert(name, count);
            }
        }
    }
}

#[test]
fn no_handler_reads_a_head_outside_the_known_sites() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut found = BTreeMap::new();
    walk(&root, &root, &mut found);
    let known: BTreeMap<&str, usize> = KNOWN.into_iter().collect();
    let new: Vec<String> = found
        .iter()
        .filter(|(file, count)| **count > known.get(file.as_str()).copied().unwrap_or(0))
        .map(|(file, count)| {
            format!(
                "{file}: {count} (known {})",
                known.get(file.as_str()).copied().unwrap_or(0)
            )
        })
        .collect();
    assert!(
        new.is_empty(),
        "a head read outside the known sites -- plan on the transaction's version instead \
         (ooxml_word_edit::planning): {new:?}"
    );
}
