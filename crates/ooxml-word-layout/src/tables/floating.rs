//! Owns: whether a table floats (`w:tblpPr`), as docx2svg's `table.floating_of` decides
//! it ([`floats`]): laid out in the flow, floating ([`Floating`]: what it is positioned
//! against and how far text keeps off it), or positioned as nothing measured; a floating
//! table's top on its page ([`Floating::top`], `floating_top`); and how far its box reaches
//! past its outer grid lines for the text beside it (`floating_edges`).
//!
//! Must not: place lines beside a floating table (the paginator does, `crate::wrap`).

pub use crate::table::Floating;
use crate::table::Unsupported;
use ooxml_word_model::value::{Props, Value};

/// Twips a floating table's edge with no border reaches past its grid line, for the text
/// beside it (`UNBORDERED_EDGE`).
pub const UNBORDERED_EDGE: i64 = 15;

/// How a table stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Standing {
    /// In the flow (no `w:tblpPr`, or one at 0, 0 not against the text).
    InFlow,
    /// Floating.
    Floating(Floating),
}

fn unsupported(why: &str) -> Unsupported {
    Unsupported(why.to_owned())
}

/// How a table whose declared properties are `declared` and resolved ones `properties`
/// stands (`floating_of`); `Unsupported` where its position was not measured. Below mode
/// 15 a table against the margin or the page at `tblpY` 0 goes where `text` would put it.
pub fn floats(declared: &Props, properties: &Props, mode15: bool) -> Result<Standing, Unsupported> {
    let Some(Value::Attrs(attrs)) = properties.get("tblpPr") else {
        return Ok(Standing::InFlow);
    };
    let get = |key: &str| {
        attrs
            .iter()
            .find(|(k, _)| &**k == key)
            .map(|(_, v)| &**v)
            .filter(|v| !v.is_empty())
    };
    let number = |key: &str| -> Result<i64, Unsupported> {
        match get(key) {
            None => Ok(0),
            Some(v) => v
                .trim()
                .parse::<i64>()
                .map_err(|_| Unsupported(format!("a floating table's {key} that is not a number"))),
        }
    };
    let vertical = get("vertAnchor").unwrap_or("margin");
    let horizontal = get("horzAnchor");
    let (x, y) = (number("tblpX")?, number("tblpY")?);
    let (x_spec, y_spec) = (get("tblpXSpec"), get("tblpYSpec"));
    if !matches!(vertical, "text" | "margin" | "page") || !matches!(horizontal, None | Some("text" | "margin" | "page"))
    {
        return Err(unsupported("a floating table positioned against a frame not measured"));
    }
    if x == 0 && y == 0 && x_spec.is_none() && y_spec.is_none() && vertical != "text" {
        if horizontal.is_some() {
            return Err(unsupported("a floating table at 0, 0 with a horzAnchor (not measured)"));
        }
        return Ok(Standing::InFlow);
    }
    if !matches!(x_spec, None | Some("left" | "center" | "right"))
        || !matches!(y_spec, None | Some("top" | "center" | "bottom"))
    {
        return Err(unsupported("a floating table aligned inside or outside (not measured)"));
    }
    if y_spec.is_some() && vertical == "text" {
        return Err(unsupported("a floating table aligned against the text (not measured)"));
    }
    if declared.contains_key("jc") || declared.contains_key("tblInd") {
        return Err(unsupported(
            "a floating table with its own w:jc or w:tblInd (not measured)",
        ));
    }
    let vertical = if !mode15 && vertical != "text" && y == 0 && y_spec.is_none() {
        "text"
    } else {
        vertical
    };
    Ok(Standing::Floating(Floating {
        vertical: vertical.to_owned(),
        y,
        y_spec: y_spec.map(str::to_owned),
        left: number("leftFromText")?,
        right: number("rightFromText")?,
        top: number("topFromText")?,
        bottom: number("bottomFromText")?,
        page: horizontal == Some("page"),
        x,
        x_spec: x_spec.map(str::to_owned),
    }))
}
