//! A text box the readers read from its VML Fallback (phase 3 RD1a, the one `mc:` rule): its
//! Choice needs a namespace they do not read, so the index keys the Fallback's paragraphs.
//! Reading them works; an edit inside them is refused with the one refusal, and nothing is
//! written -- the sync that keeps a fallback with its choice would put Word's choice back over
//! the edit.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]

use ooxml_session::{CallContext, DocId, DocKind};
use serde_json::json;

const BOX: &str = r#"<w:p w14:paraId="00000001" w14:textId="77777777"><w:r><mc:AlternateContent xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006" xmlns:cx="http://schemas.microsoft.com/office/drawing/2014/chartex" xmlns:v="urn:schemas-microsoft-com:vml"><mc:Choice Requires="cx"><w:drawing/></mc:Choice><mc:Fallback><w:pict><v:shape style="width:100pt;height:50pt"><v:textbox><w:txbxContent><w:p w14:paraId="00000002" w14:textId="77777777"><w:r><w:t>boxed</w:t></w:r></w:p></w:txbxContent></v:textbox></v:shape></w:pict></mc:Fallback></mc:AlternateContent></w:r></w:p>"#;

#[test]
fn an_edit_inside_a_text_box_read_from_its_fallback_is_refused_and_writes_nothing() {
    let bytes = crate::editing::docx(BOX);
    let session = crate::editing::session();
    session.open(bytes.clone(), DocKind::Docx, "box.docx").unwrap();
    let cx = CallContext::default();
    let read = session.call("word_read", json!({"doc": "d1"}), &cx).to_json();
    assert!(read["data"].as_str().unwrap_or_default().contains("boxed"), "{read:?}");
    let reply = session
        .call(
            "word_set_text",
            json!({"doc": "d1", "target": "p:00000002", "text": "changed"}),
            &cx,
        )
        .to_json();
    let error = &reply["error"];
    assert_eq!(
        error["details"]["not_supported"].as_str(),
        Some("an edit inside a text box read from its fallback"),
        "{reply:?}"
    );
    assert!(
        error["details"]["text_box"].as_str().unwrap_or_default().contains("cx"),
        "{reply:?}"
    );
    assert!(
        error["valid_options"].to_string().contains("edit it in Word"),
        "{reply:?}"
    );
    assert_eq!(&*session.save(&DocId::nth(1)).unwrap(), &*bytes);
}
