//! The players on small made-up files, against what ooxml-common draws of the same bytes
//! (the full differential is ooxml-oracle's: `tests/metafile_cases.rs`).

use ooxml_metafile::{dib, draw, is_metafile, MetafileLimits, Options};

fn hex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap_or(0))
        .collect()
}

/// An EMF: a red solid brush selected, a rectangle, the end.
const EMF: &str = "0100000058000000000000000000000063000000310000000000000000000000560a00002b05000020454d460000\
0100a800000005000000010000000000000000000000000000006000000060000000190000001900000027000000180000000100000000000000ff0\
0000000000000250000000c000000010000002b000000180000000a0000000a0000003c000000280000000e0000001400000000000000000000001\
4000000";

/// A placeable WMF: a rectangle in a 200 by 100 box at 1440 to the inch.
const WMF: &str =
    "d7cdc69a000000000000c8006400a005000000000000010009000003130000000000000000000000070000001b045000960014003200030000000000";

#[test]
fn an_emf_rectangle_draws_as_ooxml_common_draws_it() {
    let drawing = draw(&hex(EMF), &Options::default()).unwrap();
    let svg = String::from_utf8(drawing.svg()).unwrap();
    assert_eq!(
        svg,
        "<svg xmlns=\"http://www.w3.org/2000/svg\" version=\"1.1\" width=\"100.006\" height=\"50.003\" \
         viewBox=\"0 0 102.606 51.803\" preserveAspectRatio=\"none\" data-ooxml-metafile=\"gdi\"><path \
         d=\"M10 10L60 10L60 40L10 40Z\" fill=\"#FF0000\" fill-rule=\"evenodd\" stroke=\"#000000\" \
         stroke-width=\"1\" stroke-linecap=\"round\" stroke-miterlimit=\"10\"/></svg>"
    );
    assert_eq!(
        (drawing.width, drawing.height, drawing.drawn),
        (100.00629921259844, 50.00314960629922, 1)
    );
    assert!(drawing.complete() && drawing.warning().is_none());
}

#[test]
fn a_placeable_wmf_rectangle_draws_as_ooxml_common_draws_it() {
    let drawing = draw(&hex(WMF), &Options::default()).unwrap();
    let svg = String::from_utf8(drawing.svg()).unwrap();
    assert!(svg.contains("viewBox=\"0 0 200 100\""), "{svg}");
    assert!(
        svg.contains("<path d=\"M50 20L150 20L150 80L50 80Z\" fill=\"#FFFFFF\" fill-rule=\"evenodd\""),
        "{svg}"
    );
    assert_eq!((drawing.width, drawing.height), (13.333333333333334, 6.666666666666667));
}

#[test]
fn caps_stop_the_playback_and_say_so() {
    let options = Options {
        limits: MetafileLimits {
            max_records: 2,
            ..MetafileLimits::default()
        },
        ..Options::default()
    };
    let drawing = draw(&hex(EMF), &options).unwrap();
    assert!(drawing.truncated);
    assert_eq!(
        drawing.warning().as_deref(),
        Some("it is larger than the player's limits, so the rest of it is not drawn")
    );
}

#[test]
fn what_is_not_a_metafile_is_none() {
    for bytes in [&b""[..], b"not a metafile", &hex(EMF)[..40]] {
        assert!(!is_metafile(bytes));
        assert!(draw(bytes, &Options::default()).is_none());
    }
}

#[test]
fn a_24_bit_dib_becomes_a_png_of_its_pixels() {
    // BITMAPINFOHEADER 1 x 1 at 24 bits, bottom-up; one blue-green-red pixel and its padding.
    let mut header = vec![40, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 1, 0, 24, 0];
    header.resize(40, 0);
    let image = dib::to_image(&header, &[0x30, 0x20, 0x10, 0]).unwrap();
    assert_eq!(image.mime, "image/png");
    let decoded = dib::decode(&header, &[0x30, 0x20, 0x10, 0]).unwrap();
    assert_eq!(
        (decoded.width, decoded.height, decoded.rgba.as_slice()),
        (1, 1, &[0x10, 0x20, 0x30, 255][..])
    );
    assert!(dib::to_image(&header[..20], &[]).is_none());
}
