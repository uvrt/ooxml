//! Owns: whether a table floats (`w:tblpPr`), as docx2svg's `table.floating_of` decides
//! it ([`floats`]): laid out in the flow, floating, or positioned as nothing measured.
//!
//! Must not: position a floating table (that is the floats' lane, L3c: a floating table
//! stops this layout).

use crate::table::Unsupported;
use ooxml_word_model::value::{Props, Value};

/// How a table stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Standing {
    /// In the flow (no `w:tblpPr`, or one at 0, 0 not against the text).
    InFlow,
    /// Floating.
    Floating,
}

fn unsupported(why: &str) -> Unsupported {
    Unsupported(why.to_owned())
}

/// How a table whose declared properties are `declared` and resolved ones `properties`
/// stands (`floating_of`); `Unsupported` where its position was not measured.
pub fn floats(declared: &Props, properties: &Props) -> Result<Standing, Unsupported> {
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
    for key in ["leftFromText", "rightFromText", "topFromText", "bottomFromText"] {
        number(key)?;
    }
    Ok(Standing::Floating)
}
