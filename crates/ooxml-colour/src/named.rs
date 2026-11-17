//! Owns: the colours a name or another colour space states -- `a:prstClr`'s named colours
//! (`ST_PresetColorVal`: the CSS named colours, in either of the schema's spellings,
//! [`preset_colour_hex`]), `a:hslClr` ([`hsl_hex`]) and `a:scrgbClr` read as sRGB
//! ([`scrgb_hex`]), as ooxml-common's `drawingml/read.py` turns them into hex.
//!
//! Must not: apply transforms (`crate::transform`). A system colour (`a:sysClr`) is drawn
//! as the `lastClr` it states, as ooxml-common draws it; no system palette is kept here.

/// The CSS named colours, keyed in lower case (Word draws `darkSeaGreen` as `8FBC8F`, the
/// CSS value: docx2svg's `tools/make_dml_probe.py`).
pub const CSS_COLOURS: [(&str, &str); 147] = [
    ("aliceblue", "F0F8FF"),
    ("antiquewhite", "FAEBD7"),
    ("aqua", "00FFFF"),
    ("aquamarine", "7FFFD4"),
    ("azure", "F0FFFF"),
    ("beige", "F5F5DC"),
    ("bisque", "FFE4C4"),
    ("black", "000000"),
    ("blanchedalmond", "FFEBCD"),
    ("blue", "0000FF"),
    ("blueviolet", "8A2BE2"),
    ("brown", "A52A2A"),
    ("burlywood", "DEB887"),
    ("cadetblue", "5F9EA0"),
    ("chartreuse", "7FFF00"),
    ("chocolate", "D2691E"),
    ("coral", "FF7F50"),
    ("cornflowerblue", "6495ED"),
    ("cornsilk", "FFF8DC"),
    ("crimson", "DC143C"),
    ("cyan", "00FFFF"),
    ("darkblue", "00008B"),
    ("darkcyan", "008B8B"),
    ("darkgoldenrod", "B8860B"),
    ("darkgray", "A9A9A9"),
    ("darkgreen", "006400"),
    ("darkgrey", "A9A9A9"),
    ("darkkhaki", "BDB76B"),
    ("darkmagenta", "8B008B"),
    ("darkolivegreen", "556B2F"),
    ("darkorange", "FF8C00"),
    ("darkorchid", "9932CC"),
    ("darkred", "8B0000"),
    ("darksalmon", "E9967A"),
    ("darkseagreen", "8FBC8F"),
    ("darkslateblue", "483D8B"),
    ("darkslategray", "2F4F4F"),
    ("darkslategrey", "2F4F4F"),
    ("darkturquoise", "00CED1"),
    ("darkviolet", "9400D3"),
    ("deeppink", "FF1493"),
    ("deepskyblue", "00BFFF"),
    ("dimgray", "696969"),
    ("dimgrey", "696969"),
    ("dodgerblue", "1E90FF"),
    ("firebrick", "B22222"),
    ("floralwhite", "FFFAF0"),
    ("forestgreen", "228B22"),
    ("fuchsia", "FF00FF"),
    ("gainsboro", "DCDCDC"),
    ("ghostwhite", "F8F8FF"),
    ("gold", "FFD700"),
    ("goldenrod", "DAA520"),
    ("gray", "808080"),
    ("green", "008000"),
    ("greenyellow", "ADFF2F"),
    ("grey", "808080"),
    ("honeydew", "F0FFF0"),
    ("hotpink", "FF69B4"),
    ("indianred", "CD5C5C"),
    ("indigo", "4B0082"),
    ("ivory", "FFFFF0"),
    ("khaki", "F0E68C"),
    ("lavender", "E6E6FA"),
    ("lavenderblush", "FFF0F5"),
    ("lawngreen", "7CFC00"),
    ("lemonchiffon", "FFFACD"),
    ("lightblue", "ADD8E6"),
    ("lightcoral", "F08080"),
    ("lightcyan", "E0FFFF"),
    ("lightgoldenrodyellow", "FAFAD2"),
    ("lightgray", "D3D3D3"),
    ("lightgreen", "90EE90"),
    ("lightgrey", "D3D3D3"),
    ("lightpink", "FFB6C1"),
    ("lightsalmon", "FFA07A"),
    ("lightseagreen", "20B2AA"),
    ("lightskyblue", "87CEFA"),
    ("lightslategray", "778899"),
    ("lightslategrey", "778899"),
    ("lightsteelblue", "B0C4DE"),
    ("lightyellow", "FFFFE0"),
    ("lime", "00FF00"),
    ("limegreen", "32CD32"),
    ("linen", "FAF0E6"),
    ("magenta", "FF00FF"),
    ("maroon", "800000"),
    ("mediumaquamarine", "66CDAA"),
    ("mediumblue", "0000CD"),
    ("mediumorchid", "BA55D3"),
    ("mediumpurple", "9370DB"),
    ("mediumseagreen", "3CB371"),
    ("mediumslateblue", "7B68EE"),
    ("mediumspringgreen", "00FA9A"),
    ("mediumturquoise", "48D1CC"),
    ("mediumvioletred", "C71585"),
    ("midnightblue", "191970"),
    ("mintcream", "F5FFFA"),
    ("mistyrose", "FFE4E1"),
    ("moccasin", "FFE4B5"),
    ("navajowhite", "FFDEAD"),
    ("navy", "000080"),
    ("oldlace", "FDF5E6"),
    ("olive", "808000"),
    ("olivedrab", "6B8E23"),
    ("orange", "FFA500"),
    ("orangered", "FF4500"),
    ("orchid", "DA70D6"),
    ("palegoldenrod", "EEE8AA"),
    ("palegreen", "98FB98"),
    ("paleturquoise", "AFEEEE"),
    ("palevioletred", "DB7093"),
    ("papayawhip", "FFEFD5"),
    ("peachpuff", "FFDAB9"),
    ("peru", "CD853F"),
    ("pink", "FFC0CB"),
    ("plum", "DDA0DD"),
    ("powderblue", "B0E0E6"),
    ("purple", "800080"),
    ("red", "FF0000"),
    ("rosybrown", "BC8F8F"),
    ("royalblue", "4169E1"),
    ("saddlebrown", "8B4513"),
    ("salmon", "FA8072"),
    ("sandybrown", "F4A460"),
    ("seagreen", "2E8B57"),
    ("seashell", "FFF5EE"),
    ("sienna", "A0522D"),
    ("silver", "C0C0C0"),
    ("skyblue", "87CEEB"),
    ("slateblue", "6A5ACD"),
    ("slategray", "708090"),
    ("slategrey", "708090"),
    ("snow", "FFFAFA"),
    ("springgreen", "00FF7F"),
    ("steelblue", "4682B4"),
    ("tan", "D2B48C"),
    ("teal", "008080"),
    ("thistle", "D8BFD8"),
    ("tomato", "FF6347"),
    ("turquoise", "40E0D0"),
    ("violet", "EE82EE"),
    ("wheat", "F5DEB3"),
    ("white", "FFFFFF"),
    ("whitesmoke", "F5F5F5"),
    ("yellow", "FFFF00"),
    ("yellowgreen", "9ACD32"),
];

use crate::transform::py_mod;

fn css(key: &str) -> Option<&'static str> {
    CSS_COLOURS.iter().find(|(k, _)| *k == key).map(|(_, v)| *v)
}

/// `RRGGBB` for an `a:prstClr@val`, in either spelling (`darkSeaGreen`, `dkSeaGreen`).
pub fn preset_colour_hex(name: &str) -> Option<&'static str> {
    let key = name.to_lowercase();
    if let Some(found) = css(&key) {
        return Some(found);
    }
    for (short, long) in [("dk", "dark"), ("lt", "light"), ("med", "medium")] {
        let next_is_upper = name
            .strip_prefix(short)
            .and_then(|rest| rest.chars().next())
            .is_some_and(char::is_uppercase);
        if name.starts_with(short) && next_is_upper {
            let rest: String = key.chars().skip(short.chars().count()).collect();
            return css(&format!("{long}{rest}"));
        }
    }
    None
}

/// A level as Python's `f"{value:02X}"` writes `round(value)`: two upper-case hex digits,
/// a minus sign before a negative one.
fn hex_level(value: f64) -> String {
    let level = value.round_ties_even();
    if level < 0.0 {
        format!("-{:X}", (-level) as u64)
    } else {
        format!("{:02X}", level as u64)
    }
}

/// A channel on 0-255 as a level clamped to 0-255.
fn level(value: f64) -> String {
    hex_level(value.round_ties_even().clamp(0.0, 255.0))
}

/// `a:hslClr` as `RRGGBB`: hue as a fraction of a turn, saturation and luminance 0-1.
pub fn hsl_hex(hue: f64, sat: f64, lum: f64) -> String {
    if sat == 0.0 {
        // Not clamped, as the Python reads it.
        let grey = hex_level(lum * 255.0);
        return format!("{grey}{grey}{grey}");
    }
    let hue_to_rgb = |p: f64, q: f64, t: f64| {
        let t = py_mod(t, 1.0);
        if t < 1.0 / 6.0 {
            p + (q - p) * 6.0 * t
        } else if t < 1.0 / 2.0 {
            q
        } else if t < 2.0 / 3.0 {
            p + (q - p) * (2.0 / 3.0 - t) * 6.0
        } else {
            p
        }
    };
    let q = if lum < 0.5 {
        lum * (1.0 + sat)
    } else {
        lum + sat - lum * sat
    };
    let p = 2.0 * lum - q;
    [hue + 1.0 / 3.0, hue, hue - 1.0 / 3.0]
        .map(|t| level(hue_to_rgb(p, q, t) * 255.0))
        .concat()
}

/// `a:scrgbClr`'s channels (stated in 1/1000 percent), clamped to 0-1, read as sRGB:
/// `RRGGBB`, and the clamped channels.
pub fn scrgb_hex(r: f64, g: f64, b: f64) -> (String, [f64; 3]) {
    let channels = [r, g, b].map(|v| (v / 100_000.0).clamp(0.0, 1.0));
    (channels.map(|c| level(c * 255.0)).concat(), channels)
}
