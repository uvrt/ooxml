//! Helpers the test modules share: clocks, an unlimited budget, no cancelling, the page
//! engine on the repository's fonts and the namespaces made-up parts declare (one copy instead
//! of one per module).
// Tests may unwrap, panic and index (docs/architecture/README.md, section 2.5).
#![allow(
    dead_code,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use ooxml_base::Cancel;
use ooxml_derive::CacheBudget;
use ooxml_fonts::{Folder, Location, WordFontOptions};
use ooxml_store::Clock;
use ooxml_word_pages::{DiskFonts, EngineBudgets, EngineOptions, FontSetup, PageEngine};
use std::sync::Arc;
use std::time::{Duration, Instant};

pub struct Never;

impl Cancel for Never {
    fn is_cancelled(&self) -> bool {
        false
    }
}

pub struct Still;

impl Clock for Still {
    fn monotonic(&self) -> Duration {
        Duration::ZERO
    }
}

pub struct Unlimited;

impl CacheBudget for Unlimited {
    fn reserve(&self, _bytes: usize) -> bool {
        true
    }
    fn release(&self, _bytes: usize) {}
    fn used(&self) -> usize {
        0
    }
}

pub struct Monotonic(pub Instant);

impl Clock for Monotonic {
    fn monotonic(&self) -> Duration {
        self.0.elapsed()
    }
}

pub fn engine() -> PageEngine {
    engine_with(Arc::new(Unlimited))
}

/// The page engine on the repository's fonts, what it paints and reads counted on `paint`.
pub fn engine_with(paint: Arc<dyn CacheBudget>) -> PageEngine {
    let folder = Folder {
        location: Location::Folder,
        path: concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../python/pptx2svg/packages/pptx2svg-fonts/src/pptx2svg_fonts/files"
        )
        .to_owned(),
    };
    let options = WordFontOptions {
        substitutes: Vec::new(),
        recorded_faces: true,
        fallbacks: true,
    };
    let setup = Arc::new(FontSetup::new(Arc::new(DiskFonts), &[folder], options));
    let budgets = EngineBudgets {
        held: Arc::new(Unlimited),
        paint,
    };
    PageEngine::new(setup, EngineOptions::default(), budgets)
}

pub const W: &str = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:w14="http://schemas.microsoft.com/office/word/2010/wordml""#;

pub const R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/";

/// `package` read as a session reads a document it opens: every block without an id given
/// one ([`ooxml_word_model::WordPackage::open`]); a stamped package's trees are its own.
pub fn read(package: &ooxml_opc::Package) -> ooxml_word_model::WordPackage {
    ooxml_word_model::WordPackage::open(package.clone()).0
}

/// `package` opened as a session's editor opens it: its readings the document's one chain.
pub fn doc(package: &ooxml_opc::Package) -> ooxml_word_edit::WordDoc {
    ooxml_word_edit::WordDoc::open(package.clone(), Arc::new(Unlimited)).unwrap()
}

/// Version 1 of `doc` named `name`, as a reading tool is handed it.
pub fn version<'a>(name: &'a str, doc: &'a ooxml_word_edit::WordDoc) -> ooxml_tools_word::Version<'a> {
    ooxml_tools_word::Version::new(name, doc.package(), 1).with_word(Some(doc))
}

/// Each block id a reading of `package` by place gave ([`by_place`]) (positional `p@body/3`,
/// `t@body/0`, a repeat) that the document opened -- every block stamped in memory -- names
/// otherwise, the longest first.
pub fn stamped_ids(package: &ooxml_opc::Package) -> Vec<(String, String)> {
    use ooxml_word_model::ids::{story_name, PartIds};
    let word = read(package);
    let Ok(Some(main)) = ooxml_word_model::parts::main_part(package) else {
        return Vec::new();
    };
    let names = |ids: PartIds| {
        let mut out: Vec<String> = ids.paragraphs().iter().map(|p| p.id).collect();
        for table in ids.tables().iter() {
            out.push(table.id.clone());
            out.extend(table.rows.iter().map(|(_, r)| r.clone()));
        }
        out
    };
    let mut renamed = Vec::new();
    for part in ooxml_word_model::parts::parts(package).unwrap_or_default() {
        let story = story_name(&main, &part);
        let (Ok(Some(old)), Ok(Some(new))) = (package.tree(&part), word.tree(&part)) else {
            continue;
        };
        let pairs = by_place(&story, old.root())
            .into_iter()
            .zip(names(PartIds::new(&story, new.root())));
        renamed.extend(pairs.filter(|(a, b)| a != b));
    }
    renamed.sort_by(|a, b| b.0.len().cmp(&a.0.len()).then_with(|| a.0.cmp(&b.0)));
    renamed
}

/// `value` (a call's arguments, an expected reply) with every old id of `package`
/// ([`stamped_ids`]) in its strings as the document names it now.
pub fn now(package: &ooxml_opc::Package, value: serde_json::Value) -> serde_json::Value {
    let renamed = stamped_ids(package);
    fn walk(value: serde_json::Value, renamed: &[(String, String)]) -> serde_json::Value {
        use serde_json::Value;
        match value {
            Value::String(text) => Value::String(named(&text, renamed)),
            Value::Array(items) => Value::Array(items.into_iter().map(|v| walk(v, renamed)).collect()),
            Value::Object(map) => Value::Object(
                map.into_iter()
                    .map(|(k, v)| (named(&k, renamed), walk(v, renamed)))
                    .collect(),
            ),
            other => other,
        }
    }
    walk(value, &renamed)
}

/// `text` with every old id of `renamed` as it is named now.
fn named(text: &str, renamed: &[(String, String)]) -> String {
    let mut out = String::new();
    let mut rest = text;
    'scan: while !rest.is_empty() {
        for (old, new) in renamed {
            if let Some(after) = rest.strip_prefix(old.as_str()) {
                if !after.starts_with(|c: char| c.is_ascii_digit()) {
                    out.push_str(new);
                    rest = after;
                    continue 'scan;
                }
            }
        }
        let mut chars = rest.chars();
        out.extend(chars.next());
        rest = chars.as_str();
    }
    out
}

/// The names a reading of the part `root` (story `story`) gave its blocks by place before
/// every block had an id of its own (stamp on open; the index no longer gives them): what
/// this suite's data was written with. In [`ooxml_word_model::ids::PartIds`]' order: the
/// paragraphs, then each table and its rows.
fn by_place(story: &str, root: &ooxml_xml::Element) -> Vec<String> {
    use ooxml_word_model::ids::{valid_long_hex, PartIds};
    use std::collections::HashMap;
    let ids = PartIds::new(story, root);
    let prefix = if story == "body" {
        String::new()
    } else {
        format!("{story}/")
    };
    let raw_of = |name: &str| {
        name.rsplit_once(':')
            .map(|(_, raw)| raw.to_owned())
            .filter(|r| !r.is_empty())
    };
    // repeats were counted by spelling, paragraphs and rows together in document order
    let mut blocks: Vec<(Box<[u32]>, Option<String>)> = ids.paragraphs().iter().map(|p| (p.path, p.raw)).collect();
    for table in ids.tables().iter() {
        blocks.extend(table.rows.iter().map(|(path, name)| (path.clone(), raw_of(name))));
    }
    blocks.sort_by(|a, b| a.0.cmp(&b.0));
    let mut seen: HashMap<String, usize> = HashMap::new();
    let repeat: HashMap<Box<[u32]>, usize> = blocks
        .into_iter()
        .map(|(path, raw)| {
            let k = raw.map_or(0, |r| {
                let n = seen.entry(r.to_uppercase()).or_insert(0);
                *n += 1;
                *n - 1
            });
            (path, k)
        })
        .collect();
    let at = |path: &[u32]| repeat.get(path).copied().unwrap_or(0);
    let mut out: Vec<String> = ids
        .paragraphs()
        .iter()
        .map(|p| match (&p.raw, at(&p.path)) {
            (None, _) => format!("p@{story}/{}", p.index),
            (Some(raw), 0) => format!("{prefix}p:{raw}"),
            (Some(raw), k) => format!("{prefix}p:{raw}#{k}"),
        })
        .collect();
    for (n, table) in ids.tables().iter().enumerate() {
        let good = |path: &[u32], name: &str| raw_of(name).filter(|r| valid_long_hex(r) && at(path) == 0);
        let id = match table.rows.first().and_then(|(path, name)| good(path, name)) {
            Some(raw) => format!("{prefix}t:{raw}"),
            None => format!("t@{story}/{n}"),
        };
        out.push(id.clone());
        for (k, (path, name)) in table.rows.iter().enumerate() {
            out.push(match good(path, name) {
                Some(raw) => format!("{prefix}tr:{raw}"),
                None => format!("{id}/r{k}"),
            });
        }
    }
    out
}
