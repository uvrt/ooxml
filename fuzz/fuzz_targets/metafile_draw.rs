//! EMF, EMF+ and WMF players: no input panics or runs away (every cap holds), a drawing's SVG
//! is written whole, and drawing is deterministic. The same bytes are also read as a packed
//! DIB.
#![no_main]

use libfuzzer_sys::fuzz_target;
use ooxml_metafile::{dib, draw, Options};

fuzz_target!(|data: &[u8]| {
    let options = Options::default();
    let first = draw(data, &options);
    if let Some(drawing) = &first {
        let svg = drawing.svg();
        assert!(svg.starts_with(b"<svg ") && svg.ends_with(b"</svg>"));
        assert!(
            svg.len() <= 4 * options.limits.max_output_chars + 4096,
            "the output cap holds"
        );
    }
    assert_eq!(draw(data, &options), first, "drawing is deterministic");
    if let Some((header, bits)) = dib::split_packed(data) {
        let _ = dib::to_image(header, bits);
        let _ = dib::decode(header, bits);
    }
});
