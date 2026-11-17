//! VML: no run object panics the painter's reading of it. Each input is parsed as XML and,
//! as a `w:pict`, `w:object` or `mc:AlternateContent`, read for its preset shape's paint and
//! its text path, which is then written in a box both upright and fitted by made-up ink.
#![no_main]

use libfuzzer_sys::fuzz_target;
use ooxml_base::Q;
use ooxml_fonts::ink::TextInk;
use ooxml_fonts::outlines::InkBox;
use ooxml_word_layout::px::Px;
use ooxml_word_pages::drawings::vml::{paint, TextPath};
use ooxml_word_pages::drawings::write::PxBox;
use ooxml_xml::{parse, Limits};

fuzz_target!(|data: &[u8]| {
    let Ok(tree) = parse(data, &Limits::default()) else { return };
    let object = tree.root();
    let _ = paint(object);
    let Some(text) = TextPath::of(object, Some("Aptos")) else { return };
    let at = PxBox {
        x: Px::from(Q::new(1, 3)),
        y: Px::from(Q::from_integer(10)),
        width: Px::from(Q::from_integer(400)),
        height: Px::from(Q::new(301, 2)),
    };
    let _ = text.markup(&at, None);
    for ink in [InkBox::Units([-20, -200, 2000, 700]), InkBox::Float([0.5, -1.0, 99.25, 1.0])] {
        let ink = TextInk { units_per_em: 2048, ink, pens: vec![0, 600, 1200], slant: 0.34 };
        let _ = text.markup(&at, Some(&ink));
    }
});
