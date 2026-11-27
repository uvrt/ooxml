//! The listed changes holding given records, read over a stretch of their part, equal what
//! grouping the whole part gives (made-up documents).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use crate::support;

use ooxml_word_model::docx::changes::{change_groups, changes_holding};
use ooxml_word_model::docx::revisions::records;
use ooxml_word_model::docx::Docx;
use support::{package, Parts};

/// A small deterministic generator.
struct Seq(u64);

impl Seq {
    fn next(&mut self, n: u64) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (self.0 >> 33) % n
    }
}

fn stamp(seq: &mut Seq, id: &mut u32) -> String {
    *id += 1;
    let author = ["Ann", "Bo"][usize::try_from(seq.next(2)).unwrap()];
    let date = ["2026-01-01T00:00:00Z", "2026-01-02T00:00:00Z"][usize::try_from(seq.next(2)).unwrap()];
    format!(r#"w:id="{id}" w:author="{author}" w:date="{date}""#)
}

fn paragraph(seq: &mut Seq, id: &mut u32) -> String {
    let mark = match seq.next(4) {
        0 => format!("<w:pPr><w:rPr><w:del {}/></w:rPr></w:pPr>", stamp(seq, id)),
        1 => format!("<w:pPr><w:rPr><w:ins {}/></w:rPr></w:pPr>", stamp(seq, id)),
        _ => String::new(),
    };
    let mut runs = String::new();
    for _ in 0..seq.next(3) + 1 {
        runs.push_str(&match seq.next(4) {
            0 => format!(
                r#"<w:del {}><w:r><w:delText>gone</w:delText></w:r></w:del>"#,
                stamp(seq, id)
            ),
            1 => format!(r#"<w:ins {}><w:r><w:t>new</w:t></w:r></w:ins>"#, stamp(seq, id)),
            _ => "<w:r><w:t>kept</w:t></w:r>".to_owned(),
        });
    }
    format!("<w:p>{mark}{runs}</w:p>")
}

fn table(seq: &mut Seq, id: &mut u32) -> String {
    let mut rows = String::new();
    for _ in 0..3 {
        let deleted = if seq.next(3) == 0 {
            format!("<w:trPr><w:del {}/></w:trPr>", stamp(seq, id))
        } else {
            String::new()
        };
        let cells: String = (0..2)
            .map(|_| {
                format!(
                    "<w:tc><w:tcPr><w:tcW w:w=\"2000\" w:type=\"dxa\"/></w:tcPr>{}</w:tc>",
                    paragraph(seq, id)
                )
            })
            .collect();
        rows.push_str(&format!("<w:tr>{deleted}{cells}</w:tr>"));
    }
    format!(r#"<w:tbl><w:tblPr/><w:tblGrid><w:gridCol w:w="2000"/><w:gridCol w:w="2000"/></w:tblGrid>{rows}</w:tbl>"#)
}

fn document(seed: u64) -> Docx {
    let mut seq = Seq(seed);
    let mut id = 0;
    let mut body = String::new();
    for _ in 0..40 {
        body.push_str(&if seq.next(5) == 0 {
            table(&mut seq, &mut id)
        } else {
            paragraph(&mut seq, &mut id)
        });
    }
    let parts = Parts {
        body,
        ..Parts::default()
    };
    Docx::open(
        &package(&parts),
        std::sync::Arc::new(ooxml_derive::Limit::new(usize::MAX)),
    )
    .unwrap()
}

/// What grouping the whole part gives (`groups`): the first ids of the groups holding the
/// records `ids`.
fn expected(groups: &[Vec<String>], ids: &[&str]) -> Vec<String> {
    groups
        .iter()
        .filter(|g| g.iter().any(|r| ids.contains(&r.as_str())))
        .filter_map(|g| g.first().cloned())
        .collect()
}

#[test]
fn changes_holding_records_equal_the_whole_parts_grouping() {
    for seed in 1..=6 {
        let docx = document(seed);
        let all = records(&docx);
        let groups = change_groups(&docx);
        assert!(all.len() > 20, "seed {seed}: {} records", all.len());
        for (k, record) in all.iter().enumerate() {
            let mut wanted = vec![(record.part.clone(), record.path.to_vec())];
            let mut ids = vec![record.id.as_str()];
            assert_eq!(
                changes_holding(&docx, &wanted),
                expected(&groups, &ids),
                "seed {seed} record {k}"
            );
            if let Some(other) = all.get(k + 7).filter(|_| k % 3 == 0) {
                wanted.push((other.part.clone(), other.path.to_vec()));
                ids.push(&other.id);
                assert_eq!(
                    changes_holding(&docx, &wanted),
                    expected(&groups, &ids),
                    "seed {seed} pair {k}"
                );
            }
        }
    }
}

#[test]
fn no_records_wanted_hold_no_change() {
    let docx = document(3);
    assert!(changes_holding(&docx, &[]).is_empty());
}
