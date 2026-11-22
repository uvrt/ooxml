//! What a host reads of a Word version beside the tools (lane H7a): its problems, its
//! summary, and its id state, which the id state reader takes back.

use crate::editing::{docx, session};
use ooxml_opc::Package;
use ooxml_session::{DocKind, IdStateOf};
use ooxml_word_model::ids::load_state;
use ooxml_word_model::parts::parts;

const BODY: &str = r#"<w:p w14:paraId="0A000001"><w:pPr><w:pStyle w:val="Heading1"/></w:pPr><w:r><w:t>Title</w:t></w:r></w:p><w:p w14:paraId="0A000002"><w:r><w:t>Text.</w:t></w:r></w:p><w:tbl><w:tr w14:paraId="0A000003"><w:tc><w:p w14:paraId="0A000004"><w:r><w:t>a</w:t></w:r></w:p></w:tc></w:tr></w:tbl><w:p w14:paraId="0A000005"/>"#;

#[test]
fn the_summary_counts_the_body_and_says_it_is_no_template() {
    let s = session();
    let d = s.open(docx(BODY), DocKind::Docx, "a.docx").unwrap();
    let found = s.snapshot(&d).unwrap().summary().unwrap().unwrap();
    assert_eq!(found["paragraphs"], 4, "{found}");
    assert_eq!(found["tables"], 1, "{found}");
    assert_eq!(found["comments"], 0, "{found}");
    assert_eq!(found["revisions"], 0, "{found}");
    assert_eq!(found["template"], false, "{found}");
    assert!(found["title"].is_null(), "{found}");
    assert!(s.snapshot(&d).unwrap().problems().unwrap().is_some());
}

#[test]
fn the_id_state_is_read_back_with_the_host_s_numbers_and_retired_ids() {
    let s = session();
    let d = s.open(docx(BODY), DocKind::Docx, "a.docx").unwrap();
    let version = s.snapshot(&d).unwrap();
    let host = IdStateOf {
        version: 3,
        last: 5,
        gone: vec![(0x0A00_0009, 2)],
        said: vec![(2, "word_delete (agent=a)".into())],
        ..IdStateOf::default()
    };
    let state = version.id_state(&host).unwrap().unwrap();
    let package = Package::open(docx(BODY), &ooxml_base::Limits::default()).unwrap();
    let trees: Vec<_> = parts(&package)
        .unwrap()
        .iter()
        .filter_map(|p| package.tree(p).unwrap())
        .collect();
    let roots: Vec<_> = trees.iter().map(|t| t.root().as_ref()).collect();
    let loaded = load_state(&state, &roots).unwrap();
    assert_eq!((loaded.versions.version, loaded.versions.last), (3, 5));
    assert_eq!(loaded.gone.get(&0x0A00_0009), Some(&2));
    assert_eq!(loaded.steps.get(&2), Some(&Some("word_delete (agent=a)".to_owned())));
    assert!(loaded.pool.contains(0x0A00_0004));
}

const OWN: &str = r#"<w:p w14:paraId="0A000001"><w:r><w:t>Text.</w:t></w:r></w:p><w:tbl><w:tr w14:paraId="0A000003"><w:tc><w:p w14:paraId="0A000004"><w:r><w:t>a</w:t></w:r></w:p></w:tc></w:tr></w:tbl><w:sdt><w:sdtPr><w:id w:val="7"/></w:sdtPr><w:sdtContent><w:p w14:paraId="0A000006"><w:r><w:t>In a control.</w:t></w:r></w:p></w:sdtContent></w:sdt><w:p w14:paraId="0A000005"><w:sdt><w:sdtPr><w:id w:val="8"/></w:sdtPr><w:sdtContent><w:r><w:t>inline</w:t></w:r></w:sdtContent></w:sdt></w:p>"#;

#[test]
fn a_block_id_says_what_kind_of_block_it_names() {
    let s = session();
    let d = s.open(docx(OWN), DocKind::Docx, "a.docx").unwrap();
    let version = s.snapshot(&d).unwrap();
    let kind = |id: &str| version.block_kind(id).unwrap();
    assert_eq!(kind("p:0A000001"), Some("paragraph"));
    assert_eq!(kind("p@body/0"), Some("paragraph"));
    let table = s.snapshot(&d).unwrap();
    let tables: Vec<_> = ["t:0A000003", "t@body/0"]
        .iter()
        .map(|t| table.block_kind(t).unwrap())
        .collect();
    assert!(tables.contains(&Some("table")), "{tables:?}");
    assert_eq!(kind("cc:7"), Some("content control"));
    assert_eq!(kind("cc:8"), None, "a control in the line is no block");
    assert_eq!(kind("p:0A0000FF"), None);
    assert_eq!(kind("cc:9"), None);
    assert!(!version.has_drawing("d:1").unwrap());
}
