//! A story part's element named as the Word model names it (`story_model_path`): every run
//! the model reads -- a computed field's too, named by the run holding its instruction or by
//! its `w:fldSimple` -- has the path the walk gives that element, in paragraphs, hyperlinks,
//! content controls and table cells (rows held in a content control counted among all).
#![allow(clippy::unwrap_used, clippy::panic)]

use crate::support::{open, package, Parts};
use ooxml_word_model::docx::cells::story_model_path;
use ooxml_word_model::docx::{Docx, VersionNames};
use ooxml_word_model::model::Block;
use ooxml_xml::{Element, Node};
use std::collections::HashSet;
use std::sync::Arc;

fn run(text: &str) -> String {
    format!("<w:r><w:t>{text}</w:t></w:r>")
}

fn complex(instruction: &str) -> String {
    format!(
        r#"<w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText xml:space="preserve"> {instruction} </w:instrText></w:r><w:r><w:fldChar w:fldCharType="separate"/></w:r>{}<w:r><w:fldChar w:fldCharType="end"/></w:r>"#,
        run("cached")
    )
}

fn simple(instruction: &str) -> String {
    format!(
        r#"<w:fldSimple w:instr=" {instruction} ">{}</w:fldSimple>"#,
        run("cached")
    )
}

/// Every model run's path from the story: its paragraph's, then its own.
fn model_runs(blocks: &[Block], names: &VersionNames, out: &mut Vec<String>) {
    for block in blocks {
        match block {
            Block::Paragraph(p) => {
                let path = p.path_in(names).unwrap();
                out.extend(p.runs.iter().map(|r| format!("{path}/{}", r.path)));
            }
            Block::Table(t) => t
                .rows
                .iter()
                .flat_map(|row| row.iter())
                .for_each(|c| model_runs(c, names, out)),
        }
    }
}

/// The walk's path of every `w:r` and `w:fldSimple` below `element`.
fn walked(root: &Arc<Element>, element: &Element, place: &mut Vec<u32>, out: &mut HashSet<String>) {
    for (k, node) in element.children().iter().enumerate() {
        let Node::Element(child) = node else { continue };
        place.push(u32::try_from(k).unwrap());
        if matches!(child.name().local(), "r" | "fldSimple") {
            out.insert(story_model_path(root, place).unwrap());
        }
        walked(root, child, place, out);
        place.pop();
    }
}

#[test]
fn every_run_the_model_reads_has_the_path_the_walk_gives_it() {
    let paragraph = format!(
        r#"<w:p><w:pPr/>{}<w:hyperlink w:anchor="x">{}{}</w:hyperlink>{}<w:sdt><w:sdtContent>{}</w:sdtContent></w:sdt>{}</w:p>"#,
        run("a"),
        run("b"),
        run("c"),
        complex(r#"STYLEREF "Heading 1""#),
        run("d"),
        simple("STYLEREF 1"),
    );
    let cell = |inner: &str| format!("<w:tc><w:tcPr/><w:p>{inner}</w:p></w:tc>");
    let table = format!(
        "<w:tbl><w:tblPr/><w:tblGrid/><w:tr>{}{}</w:tr><w:sdt><w:sdtContent><w:tr>{}</w:tr></w:sdtContent></w:sdt><w:tr>{}</w:tr></w:tbl>",
        cell(&run("e")),
        cell(&complex("STYLEREF 2")),
        cell(&run("f")),
        cell(&simple("STYLEREF 1")),
    );
    let block_control = format!("<w:sdt><w:sdtContent><w:p>{}</w:p></w:sdtContent></w:sdt>", run("g"));
    let parts = Parts {
        body: format!("<w:p/>{paragraph}{table}{block_control}{paragraph}"),
        ..Parts::default()
    };
    let (_, document) = open(&parts);
    let mut runs = Vec::new();
    model_runs(&document.body, &document.names, &mut runs);
    let docx = Docx::open(&package(&parts)).unwrap();
    let root = docx.part(&docx.main).unwrap().root();
    let mut paths = HashSet::new();
    walked(root, root, &mut Vec::new(), &mut paths);
    assert!(runs.len() >= 14, "{runs:?}");
    for path in &runs {
        assert!(paths.contains(path), "{path} is not named by the walk: {paths:?}");
    }
    // The computed fields' runs: the instruction's run, and the w:fldSimple.
    assert!(runs.contains(&"w:body/w:p[2]/w:r[3]".to_owned()), "{runs:?}");
    assert!(runs.contains(&"w:body/w:p[2]/w:fldSimple[1]".to_owned()), "{runs:?}");
    assert!(
        runs.contains(&"w:body/w:tbl[1]/w:tr[3]/w:tc[1]/w:p[1]/w:fldSimple[1]".to_owned()),
        "{runs:?}"
    );
}

#[test]
fn a_story_other_than_the_body_is_named_from_its_root() {
    let docx = Docx::open(&package(&Parts {
        body: "<w:p/>".into(),
        ..Parts::default()
    }))
    .unwrap();
    let root = docx.part(&docx.main).unwrap().root();
    // The main part: from the body (child 0 of the document).
    assert_eq!(story_model_path(root, &[0, 0]).as_deref(), Some("w:body/w:p[1]"));
    let xml = format!(r#"<w:hdr {}><w:p/><w:p>{}</w:p></w:hdr>"#, crate::support::W, run("h"));
    let header = ooxml_xml::parse(xml.as_bytes(), &ooxml_xml::Limits::default()).unwrap();
    assert_eq!(
        story_model_path(header.root(), &[1, 0]).as_deref(),
        Some("w:hdr/w:p[2]/w:r[1]")
    );
}
