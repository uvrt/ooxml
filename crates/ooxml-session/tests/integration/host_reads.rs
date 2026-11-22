//! What a host reads of a Word version beside the tools (lane H7a): its problems, its
//! summary, and its id state, which the id state reader takes back.

use crate::editing::{docx, session};
use ooxml_opc::Package;
use ooxml_session::{DocKind, HeldId, IdStateOf, RetiredBy};
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

#[test]
fn a_document_opened_with_its_state_keeps_the_ids_it_retired_and_its_numbers() {
    let s = session();
    let d = s.open(docx(BODY), DocKind::Docx, "a.docx").unwrap();
    let host = IdStateOf {
        version: 3,
        last: 5,
        gone: vec![(0x0A00_0009, 2)],
        said: vec![(2, "word_delete (agent=a)".into())],
        ..IdStateOf::default()
    };
    let state = s.snapshot(&d).unwrap().id_state(&host).unwrap().unwrap();
    let (again, numbers) = s.open_state(docx(BODY), "a.docx", &state).unwrap();
    assert_eq!(numbers, (3, 5));
    let retired = s.retired(&again).unwrap();
    assert!(
        retired.iter().any(|(id, by)| *id == HeldId::Block(0x0A00_0009)
            && *by
                == RetiredBy::Earlier {
                    version: 2,
                    said: Some("word_delete (agent=a)".into())
                }),
        "{retired:?}"
    );
}

#[test]
fn a_state_of_other_bytes_is_refused_in_docx_agent_s_words() {
    let s = session();
    let d = s.open(docx(BODY), DocKind::Docx, "a.docx").unwrap();
    let state = s
        .snapshot(&d)
        .unwrap()
        .id_state(&IdStateOf::default())
        .unwrap()
        .unwrap();
    let other = docx(r#"<w:p w14:paraId="0B000001"/>"#);
    let refused = s.open_state(other, "b.docx", &state).unwrap_err();
    assert!(refused.refuses_state(), "{refused}");
    assert!(
        refused.to_string().contains("this id state is of other bytes"),
        "{refused}"
    );
    let refused = s.open_state(docx(BODY), "a.docx", "not a state").unwrap_err();
    assert!(refused.refuses_state(), "{refused}");
}
