//! Owns: the elements a new chart part is built of, as ooxml-edit's `charts/create.py` makes
//! them (`_val`, `scheme`, `_fill`, `_no_line`, `_grey_line`, `_sp`, `_none_sp`, `_body`,
//! `text_defaults`, `text_properties`, `title_element`, `_formula`, `_str_ref`, `_num_ref`,
//! `_unique_id`, the markers): in the chart part's own `c` and `a` prefixes, with a
//! namespace declared only on an element ooxml-edit gives an `nsmap`.
//!
//! Must not: put an element anywhere.

use super::super::make::Make;
use super::super::number_text;
use super::super::owned::Owned;
use super::super::replace_body_text;
use super::super::series::series_colour;

/// Office 2014's chart namespace (`c16`).
pub(crate) const C16: &str = "http://schemas.microsoft.com/office/drawing/2014/chart";
/// The sheet a new chart's data goes on.
pub(crate) const SHEET: &str = "Sheet1";
/// A line series' line (2.25 pt).
pub(crate) const LINE_WIDTH: &str = "28575";
/// A new scatter series' unfilled line (1.5 pt).
pub(crate) const SCATTER_LINE_WIDTH: &str = "19050";

/// `_val`: an element with its `val`.
pub(crate) fn val(make: &Make, local: &str, value: &str) -> Owned {
    make.c(local).with_attr("val", value)
}

/// `scheme`: `a:schemeClr` with its modifiers.
pub(crate) fn scheme(make: &Make, name: &str, mods: &[(&str, u32)]) -> Owned {
    let mut colour = make.a("schemeClr").with_attr("val", name);
    for (key, value) in mods {
        colour.push(make.a(key).with_attr("val", &value.to_string()));
    }
    colour
}

/// `_fill`: a solid fill of `colour`.
pub(crate) fn fill(make: &Make, colour: Owned) -> Owned {
    make.a("solidFill").with(colour)
}

/// `_no_line`.
pub(crate) fn no_line(make: &Make) -> Owned {
    make.a("ln").with(make.a("noFill"))
}

/// `_grey_line`: a hairline at `level` of the text colour.
pub(crate) fn grey_line(make: &Make, level: u32) -> Owned {
    make.a("ln")
        .with_attr("w", "9525")
        .with_attr("cap", "flat")
        .with_attr("cmpd", "sng")
        .with_attr("algn", "ctr")
        .with(fill(
            make,
            scheme(make, "tx1", &[("lumMod", level), ("lumOff", 100_000 - level)]),
        ))
        .with(make.a("round"))
}

/// `_sp`: `c:spPr` of `children` and an empty effect list.
pub(crate) fn sp(make: &Make, children: Vec<Owned>) -> Owned {
    let mut shape = make.c("spPr");
    for child in children {
        shape.push(child);
    }
    shape.with(make.a("effectLst"))
}

/// `_none_sp`: no fill, no line.
pub(crate) fn none_sp(make: &Make) -> Owned {
    sp(make, vec![make.a("noFill"), no_line(make)])
}

/// `series_color` filled: Office's colour for series `index`.
pub(crate) fn series_fill(make: &Make, index: usize) -> Owned {
    fill(make, series_colour(make, index))
}

/// `_body`: Office's text body properties at rotation `rot`.
fn body(make: &Make, rot: &str) -> Owned {
    make.a("bodyPr")
        .with_attr("rot", rot)
        .with_attr("spcFirstLastPara", "1")
        .with_attr("vertOverflow", "ellipsis")
        .with_attr("vert", "horz")
        .with_attr("wrap", "square")
        .with_attr("anchor", "ctr")
        .with_attr("anchorCtr", "1")
}

/// `text_defaults`: `a:defRPr` in the theme's minor face, the text colour at 65%.
fn text_defaults(make: &Make, size: u32, title: bool) -> Owned {
    let shade = 65_000;
    let mut defaults = make.a("defRPr").with_attr("sz", &size.to_string());
    for (name, value) in [
        ("b", "0"),
        ("i", "0"),
        ("u", "none"),
        ("strike", "noStrike"),
        ("kern", "1200"),
    ] {
        defaults.set_attr(name, value);
    }
    if title {
        defaults.set_attr("spc", "0");
    }
    defaults
        .with_attr("baseline", "0")
        .with(fill(
            make,
            scheme(make, "tx1", &[("lumMod", shade), ("lumOff", 100_000 - shade)]),
        ))
        .with(make.a("latin").with_attr("typeface", "+mn-lt"))
        .with(make.a("ea").with_attr("typeface", "+mn-ea"))
        .with(make.a("cs").with_attr("typeface", "+mn-cs"))
}

/// `_end`: the paragraph's end, in `lang`.
pub(crate) fn end(make: &Make, lang: &str) -> Owned {
    let end = make.a("endParaRPr");
    if lang.is_empty() {
        end
    } else {
        end.with_attr("lang", lang)
    }
}

/// `text_properties`: `c:txPr` of text Office styles (title, axis, legend), at `rot`.
pub(crate) fn text_properties(make: &Make, size: u32, rot: &str, title: bool, lang: &str) -> Owned {
    let paragraph = make
        .a("p")
        .with(make.a("pPr").with(text_defaults(make, size, title)))
        .with(end(make, lang));
    make.c("txPr")
        .with(body(make, rot))
        .with(make.a("lstStyle"))
        .with(paragraph)
}

/// `title_element`: a chart's `c:title` holding `text`, in Office's title style.
pub(crate) fn title_element(make: &Make, text: &str, size: u32, lang: &str) -> Owned {
    let paragraph = make
        .a("p")
        .with(make.a("pPr").with(text_defaults(make, size, true)))
        .with(end(make, lang));
    let rich = make
        .c("rich")
        .with(body(make, "0"))
        .with(make.a("lstStyle"))
        .with(paragraph);
    let rich = Owned::open(&replace_body_text(&rich.close(), text));
    make.c("title")
        .with(make.c("tx").with(rich))
        .with(val(make, "overlay", "0"))
        .with(none_sp(make))
        .with(text_properties(make, size, "0", true, lang))
}

/// Column `column` (from 1) as letters (`column_letters`).
pub(crate) fn letters(column: u32) -> String {
    let mut out = Vec::new();
    let mut n = column;
    while n > 0 {
        let rest = (n - 1) % 26;
        out.push(char::from(b'A' + u8::try_from(rest).unwrap_or(0)));
        n = (n - 1) / 26;
    }
    out.iter().rev().collect()
}

/// `_formula`: a cell, or a column's cells from `first` to `last`, on the sheet.
pub(crate) fn formula(column: u32, first: usize, last: Option<usize>) -> String {
    let letters = letters(column);
    match last {
        Some(last) if last != first => format!("{SHEET}!${letters}${first}:${letters}${last}"),
        _ => format!("{SHEET}!${letters}${first}"),
    }
}

/// `_f`.
fn f(make: &Make, formula: &str) -> Owned {
    make.c("f").with_text(formula)
}

/// `_str_ref`: a text reference and its cache.
pub(crate) fn str_ref(make: &Make, formula: &str, values: &[String]) -> Owned {
    let mut cache = make.c("strCache").with(val(make, "ptCount", &values.len().to_string()));
    for (index, value) in values.iter().enumerate() {
        cache.push(
            make.c("pt")
                .with_attr("idx", &index.to_string())
                .with(make.c("v").with_text(value)),
        );
    }
    make.c("strRef").with(f(make, formula)).with(cache)
}

/// `_num_ref`: a number reference and its cache (a blank has no point).
pub(crate) fn num_ref(make: &Make, formula: &str, values: &[Option<f64>], format_code: &str) -> Owned {
    let mut cache = make
        .c("numCache")
        .with(make.c("formatCode").with_text(format_code))
        .with(val(make, "ptCount", &values.len().to_string()));
    for (index, value) in values.iter().enumerate() {
        if let Some(value) = value {
            cache.push(
                make.c("pt")
                    .with_attr("idx", &index.to_string())
                    .with(make.c("v").with_text(&number_text(*value))),
            );
        }
    }
    make.c("numRef").with(f(make, formula)).with(cache)
}

/// `_unique_id`: a series' `c16:uniqueId`, its number then the chart's shared tail.
pub(crate) fn unique_id(make: &Make, index: usize, tail: &str) -> Owned {
    let ext = make
        .c("ext")
        .declaring("c16", C16)
        .with_attr("uri", "{C3380CC4-5D6E-409C-BE32-E72D297353CC}")
        .with(Owned::named(Some("c16"), "uniqueId", Some(C16)).with_attr("val", &format!("{{{index:08X}-{tail}}}")));
    make.c("extLst").with(ext)
}

/// `no_line_shape`: a series' `c:spPr` with its line off, `width` wide.
pub(crate) fn no_line_shape(make: &Make, width: &str) -> Owned {
    let line = make
        .a("ln")
        .with_attr("w", width)
        .with_attr("cap", "rnd")
        .with(make.a("noFill"))
        .with(make.a("round"));
    sp(make, vec![line])
}

/// `dot_marker`: a 5 pt circle filled and outlined in series `index`'s colour.
pub(crate) fn dot_marker(make: &Make, index: usize) -> Owned {
    let line = make.a("ln").with_attr("w", "9525").with(series_fill(make, index));
    make.c("marker")
        .with(val(make, "symbol", "circle"))
        .with(val(make, "size", "5"))
        .with(sp(make, vec![series_fill(make, index), line]))
}

/// `_no_labels`: no data labels (a pie's with leader lines).
pub(crate) fn no_labels(make: &Make, pie: bool) -> Owned {
    let mut labels = make.c("dLbls");
    for flag in [
        "showLegendKey",
        "showVal",
        "showCatName",
        "showSerName",
        "showPercent",
        "showBubbleSize",
    ] {
        labels.push(val(make, flag, "0"));
    }
    if pie {
        labels.push(val(make, "showLeaderLines", "1"));
    }
    labels
}
