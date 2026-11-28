//! The block tier's validity: an entry is taken only for the very element it was read from.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]

use super::memo::{ElementOf, Projections};
use crate::ids::StoryIx;
use crate::read::BodyElement;
use crate::theme::ThemeColours;
use crate::view::View;
use ooxml_base::XmlLimits;
use ooxml_derive::Limit;
use ooxml_xml::Element;
use std::sync::Arc;

fn paragraph(k: usize) -> Arc<Element> {
    let xml = format!(
        r#"<w:p xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:r><w:t>{k}</w:t></w:r></w:p>"#
    );
    Arc::clone(ooxml_xml::parse(xml.as_bytes(), &XmlLimits::default()).unwrap().root())
}

fn memo() -> Projections {
    Projections::new(Arc::new(Limit::new(64 << 20)))
}

#[test]
fn an_element_kept_is_taken_for_itself_and_not_for_an_equal_one() {
    let projections = memo();
    let colours = Arc::new(ThemeColours::default());
    let of = ElementOf {
        view: View::Final,
        story: StoryIx::BODY,
        colours: &colours,
        open: &[],
    };
    let (kept, equal) = (paragraph(1), paragraph(1));
    projections.keep_element(&kept, &of, Arc::new(BodyElement::default()));
    assert!(projections.element(&kept, &of).is_some());
    assert!(
        projections.element(&equal, &of).is_none(),
        "taken by identity, not by content"
    );
    let other_view = ElementOf {
        view: View::Original,
        ..of
    };
    assert!(projections.element(&kept, &other_view).is_none(), "kept per view");
    let open = [crate::read::fields::OpenState {
        part: crate::read::fields::Part::Result,
        computed: false,
        keyword: "TOC".to_owned(),
    }];
    let in_field = ElementOf { open: &open, ..of };
    assert!(projections.element(&kept, &in_field).is_none(), "kept per open fields");
}

#[test]
fn a_freed_element_is_never_taken_for_a_new_one_at_its_address() {
    let projections = memo();
    let colours = Arc::new(ThemeColours::default());
    let of = ElementOf {
        view: View::Final,
        story: StoryIx::BODY,
        colours: &colours,
        open: &[],
    };
    let freed = paragraph(0);
    projections.keep_element(&freed, &of, Arc::new(BodyElement::default()));
    drop(freed);
    // new elements of the same shape made after it was freed (one may well land where it
    // was): none is taken for it
    for k in 1..2_000 {
        let made = paragraph(k);
        assert!(projections.element(&made, &of).is_none(), "element {k}");
    }
}
