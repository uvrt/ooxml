//! `check`'s layout facts made warm equal those made cold (the caching policy's page tier,
//! ADR-008): a made-up document of several pages (the Markdown constructs fixture's body,
//! three times: headings, lists, tables, notes, a header and a footer) edited at random --
//! a paragraph made longer, small text, empty paragraphs and keeps inserted, blocks deleted
//! and copied -- and after every edit one engine's check, kept warm across the versions,
//! equals a fresh engine's, with and without minimum sizes and a range; and again with the
//! warm engine's page tier held within a budget too small to keep most pages.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use crate::check::checked;
use crate::common::{engine, engine_with};
use ooxml_base::Limits;
use ooxml_derive::CacheBudget;
use ooxml_opc::Package;
use ooxml_tools_word::{EnginePages, LastCheck};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

const FIXTURE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../crates/ooxml-oracle/fixtures/generated/markdown/constructs.docx"
);

/// A budget of `limit` bytes.
struct Capped {
    used: AtomicUsize,
    limit: usize,
}

impl CacheBudget for Capped {
    fn reserve(&self, bytes: usize) -> bool {
        self.used
            .try_update(Ordering::SeqCst, Ordering::SeqCst, |used| {
                (used + bytes <= self.limit).then_some(used + bytes)
            })
            .is_ok()
    }
    fn release(&self, bytes: usize) {
        let _ = self.used.try_update(Ordering::SeqCst, Ordering::SeqCst, |used| {
            Some(used.saturating_sub(bytes))
        });
    }
    fn used(&self) -> usize {
        self.used.load(Ordering::SeqCst)
    }
}

/// Seeded pseudo-random numbers (splitmix64): the same edits every run.
struct Random(u64);

impl Random {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn below(&mut self, n: usize) -> usize {
        usize::try_from(self.next() % (n.max(1) as u64)).unwrap()
    }
}

/// The end of the top-level element starting at `at` in `xml`: its tags walked in order,
/// those of its name counted.
fn element_end(xml: &str, at: usize) -> usize {
    let name_end = at + xml[at..].find([' ', '>', '/']).unwrap();
    let name = &xml[at + 1..name_end];
    let mut depth = 0usize;
    let mut k = at;
    while let Some(start) = xml[k..].find('<').map(|o| k + o) {
        let end = start + xml[start..].find('>').unwrap() + 1;
        let tag = &xml[start + 1..end - 1];
        let (closing, tag) = tag.strip_prefix('/').map_or((false, tag), |t| (true, t));
        let tag_name = tag.split([' ', '/']).next().unwrap();
        if tag_name == name {
            if closing {
                depth -= 1;
            } else if !tag.ends_with('/') {
                depth += 1;
            }
            if depth == 0 {
                return end;
            }
        }
        k = end;
    }
    panic!("unclosed {name}")
}

/// The document's XML before its body's blocks, its blocks, and what follows them (its
/// section properties on).
fn blocks(xml: &str) -> (String, Vec<String>, String) {
    let start = xml.find("<w:body>").unwrap() + "<w:body>".len();
    let tail = xml.rfind("<w:sectPr").unwrap();
    let mut out = Vec::new();
    let mut k = start;
    while k < tail {
        let at = k + xml[k..tail].find('<').unwrap();
        let end = element_end(xml, at);
        out.push(xml[at..end].to_owned());
        k = end;
    }
    (xml[..start].to_owned(), out, xml[tail..].to_owned())
}

/// `block` without its paragraphs' ids: a copy the tools name again.
fn unnamed(block: &str) -> String {
    let mut out = block.to_owned();
    for attribute in [" w14:paraId=\"", " w14:textId=\""] {
        while let Some(at) = out.find(attribute) {
            let end = at + attribute.len() + out[at + attribute.len()..].find('"').unwrap() + 1;
            out.replace_range(at..end, "");
        }
    }
    out
}

/// One random edit of `blocks`.
fn edit(blocks: &mut Vec<String>, random: &mut Random) {
    let at = random.below(blocks.len());
    match random.below(6) {
        0 => {
            let texts: Vec<usize> = (0..blocks.len()).filter(|k| blocks[*k].contains("</w:t>")).collect();
            let k = texts[random.below(texts.len())];
            let end = blocks[k].find("</w:t>").unwrap();
            let more = " and a few more words to make it longer".repeat(1 + random.below(8));
            blocks[k].insert_str(end, &more);
        }
        1 => {
            let size = [10, 12, 14, 16, 20, 24][random.below(6)];
            let words = "Small print that runs on for a while. ".repeat(1 + random.below(4));
            let small = format!(
                r#"<w:p><w:r><w:rPr><w:sz w:val="{size}"/></w:rPr><w:t xml:space="preserve">{words}</w:t></w:r></w:p>"#
            );
            blocks.insert(at, small);
        }
        2 => {
            for _ in 0..=random.below(3) {
                blocks.insert(at, "<w:p/>".to_owned());
            }
        }
        3 if blocks.len() > 12 => {
            blocks.remove(at);
        }
        4 | 3 => {
            let copy = unnamed(&blocks[random.below(blocks.len())]);
            blocks.insert(at, copy);
        }
        _ => {
            let kept = r#"<w:p><w:pPr><w:keepNext/></w:pPr><w:r><w:t>Kept with the next.</w:t></w:r></w:p>"#;
            blocks.insert(at, kept.to_owned());
        }
    }
}

/// What a round checks: the layout's facts, with no minimum or with one; with a range
/// every third.
fn asked(round: usize) -> Value {
    let mut asked = match round % 3 {
        0 => json!({"doc": "d1", "include": ["layout"]}),
        1 => json!({"doc": "d1", "include": ["layout"], "min_size": 11}),
        _ => json!({"doc": "d1", "include": ["layout"], "min_size": 9}),
    };
    if round % 3 == 2 {
        asked["range"] = json!(["s:1"]);
    }
    asked
}

/// The made-up document and `rounds - 1` versions after it, each one or two random edits
/// from `seed` after the one before.
fn versions(seed: u64, rounds: usize) -> Vec<Package> {
    let package = Package::open(Arc::from(std::fs::read(FIXTURE).unwrap()), &Limits::default()).unwrap();
    let xml = String::from_utf8(package.read("word/document.xml").unwrap().unwrap().to_vec()).unwrap();
    let (head, first, tail) = blocks(&xml);
    let mut body = first.clone();
    body.extend(first.iter().map(|b| unnamed(b)));
    body.extend(first.iter().map(|b| unnamed(b)));
    let mut random = Random(seed);
    let mut out = Vec::new();
    for round in 0..rounds {
        if round > 0 {
            for _ in 0..=random.below(2) {
                edit(&mut body, &mut random);
            }
        }
        let document = format!("{head}{}{tail}", body.concat());
        out.push(package.with_bytes("word/document.xml", Arc::from(document.into_bytes())));
    }
    out
}

/// `rounds` random edits from `seed`, each version checked by `warm` (kept across them)
/// and by a fresh engine: the same layout facts (and range). How many rounds found small
/// text.
fn warm_equals_cold(warm: &EnginePages, seed: u64, rounds: usize) -> usize {
    let mut small = 0;
    for (round, version) in versions(seed, rounds).iter().enumerate() {
        let number = u64::try_from(round).unwrap() + 1;
        let asked = asked(round);
        let ours = checked(version, number, asked.clone(), &LastCheck::default(), Some(warm));
        let cold = EnginePages::of(engine());
        let theirs = checked(version, number, asked, &LastCheck::default(), Some(&cold));
        assert!(ours["data"]["layout"].is_object(), "seed {seed} round {round}: {ours}");
        assert_eq!(
            ours["data"]["layout"], theirs["data"]["layout"],
            "seed {seed} round {round}: warm and cold layout facts differ"
        );
        assert_eq!(
            ours["data"]["range"], theirs["data"]["range"],
            "seed {seed} round {round}"
        );
        if ours["data"]["layout"]["small_text"]
            .as_array()
            .is_some_and(|s| !s.is_empty())
        {
            small += 1;
        }
    }
    small
}

#[test]
fn a_check_kept_warm_across_random_edits_reports_as_a_cold_check() {
    let mut small = 0;
    for seed in [1, 2] {
        let warm = EnginePages::of(engine());
        small += warm_equals_cold(&warm, seed, 8);
    }
    assert!(small > 0, "the edits make small text");
}

#[test]
fn a_page_tier_too_small_for_the_pages_still_reports_as_a_cold_check() {
    let budget = Arc::new(Capped {
        used: AtomicUsize::new(0),
        limit: 48 << 10,
    });
    let warm = EnginePages::of(engine_with(budget));
    warm_equals_cold(&warm, 3, 6);
}

#[test]
#[ignore = "many seeds: the seeds gate (nightly), and a lane's end"]
fn many_seeds() {
    for seed in 10..=70 {
        let warm = EnginePages::of(engine());
        let _ = warm_equals_cold(&warm, seed, 10);
    }
}
