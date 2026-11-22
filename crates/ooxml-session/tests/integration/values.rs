//! What a check of numbers and a scale read of a Word version beside the tools (lane H7e):
//! the body's and the cells' texts with a digit, and a scale's targets' values with the step
//! each is styled as now.

use crate::editing::{docx, session};
use ooxml_session::DocKind;
use ooxml_tools_word::{ReadScale, ReadStep, ValueHandle};

const BODY: &str = r#"<w:p w14:paraId="0A000001"><w:r><w:t>In 2024 we saw 12 sites.</w:t></w:r></w:p><w:p w14:paraId="0A000002"><w:r><w:t>No digits here.</w:t></w:r></w:p><w:tbl><w:tblGrid><w:gridCol/><w:gridCol/></w:tblGrid><w:tr w14:paraId="0A000003"><w:trPr><w:tblHeader/></w:trPr><w:tc><w:p w14:paraId="0A000004"><w:r><w:t>site</w:t></w:r></w:p></w:tc><w:tc><w:p w14:paraId="0A000005"><w:r><w:t>score</w:t></w:r></w:p></w:tc></w:tr><w:tr w14:paraId="0A000006"><w:tc><w:p w14:paraId="0A000007"><w:r><w:t>A</w:t></w:r></w:p></w:tc><w:tc><w:tcPr><w:shd w:val="clear" w:color="auto" w:fill="c62828"/></w:tcPr><w:p w14:paraId="0A000008"><w:r><w:t>1.5</w:t></w:r></w:p><w:p w14:paraId="0A000009"><w:r><w:t>of 5</w:t></w:r></w:p></w:tc></w:tr></w:tbl>"#;

#[test]
fn the_texts_of_numbers_are_the_body_s_paragraphs_and_cells_with_a_digit() {
    let s = session();
    let d = s.open(docx(BODY), DocKind::Docx, "a.docx").unwrap();
    let found = s
        .snapshot(&d)
        .unwrap()
        .number_texts("d1", None, None, None)
        .unwrap()
        .unwrap();
    let texts: Vec<(&str, &str)> = found.texts.iter().map(|t| (t.at.as_str(), t.text.as_str())).collect();
    let table = s
        .snapshot(&d)
        .unwrap()
        .word()
        .unwrap()
        .docx()
        .unwrap()
        .body()
        .unwrap()
        .ids
        .tables()
        .get(0)
        .unwrap()
        .id;
    let cell = format!("{table}/c1,1");
    assert_eq!(
        texts,
        vec![("p:0A000001", "In 2024 we saw 12 sites."), (cell.as_str(), "1.5\nof 5")]
    );
    assert!(found.texts.iter().all(|t| t.page.is_none() && t.written.is_empty()));
    assert!(found.warnings.is_empty());
    let scoped = s
        .snapshot(&d)
        .unwrap()
        .number_texts("d1", Some(&["p:0A000001".to_owned()]), None, None)
        .unwrap()
        .unwrap();
    assert_eq!(scoped.texts.len(), 1);
    let refused = s
        .snapshot(&d)
        .unwrap()
        .number_texts("d1", Some(&["p:7FFFFFFF".to_owned()]), None, None);
    assert!(refused.is_err());
}

#[test]
fn a_scale_reads_a_column_s_body_cells_and_a_cell_shaded_as_a_step() {
    let s = session();
    let d = s.open(docx(BODY), DocKind::Docx, "a.docx").unwrap();
    let version = s.snapshot(&d).unwrap();
    let table = version
        .word()
        .unwrap()
        .docx()
        .unwrap()
        .body()
        .unwrap()
        .ids
        .tables()
        .get(0)
        .unwrap()
        .id;
    let step = |fill: &str| ReadStep {
        fill: fill.into(),
        text: "FFFFFF".into(),
        label: None,
    };
    let scale = ReadScale {
        name: "Score".into(),
        steps: vec![step("C62828"), step("2E7D32")],
        targets: vec![format!("{table}/col1"), "p:0A000002".into(), "p:7FFFFFFF".into()],
        labels: false,
    };
    let found = version
        .scale_values("d1", &scale, None, None, None)
        .unwrap()
        .unwrap()
        .values;
    assert_eq!(found.len(), 3, "{found:?}");
    let cell = &found[0];
    assert_eq!(
        (cell.target.as_str(), cell.text.as_str()),
        (format!("{table}/c1,1").as_str(), "1.5\nof 5")
    );
    assert_eq!((cell.now, cell.exact), (Some(0), false));
    assert_eq!(cell.differs.as_deref(), Some("text not in the step's style"));
    assert!(matches!(&cell.handle, Some((ValueHandle::Cell(_), ids)) if ids.len() == 2));
    assert_eq!(found[1].target, "p:0A000002");
    assert_eq!(found[1].now, None);
    assert!(
        found[2]
            .skip
            .as_deref()
            .is_some_and(|s| s.starts_with("no p:7FFFFFFF: ")),
        "{:?}",
        found[2]
    );
}
