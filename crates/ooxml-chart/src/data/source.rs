//! Owns: one data source of a series -- its name (`c:tx`), categories or values -- read as
//! ooxml-edit's `_Data` reads it: the first reference child (`c:strRef`, `c:numRef`,
//! `c:multiLvlStrRef`, `c:strLit`, `c:numLit`, `c:v`), its cache, the point count from
//! `c:ptCount` (else the highest index), and each point as written: text, or a number kept
//! as an integer where it was written as one.
//!
//! Must not: read a formula or the workbook, or default a blank.

use crate::model::MAX_CACHE_POINTS;
use ooxml_xml::view::{attr, child, children, elements, local, py_float, py_int, text};
use ooxml_xml::Element;

/// The reference kinds a data source may hold, in ooxml-edit's order.
const REFERENCES: [&str; 6] = ["strRef", "numRef", "multiLvlStrRef", "strLit", "numLit", "v"];

/// One point of a data source, as written.
#[derive(Debug, Clone, PartialEq)]
pub enum Point {
    /// A string cache's text, unstripped.
    Text(String),
    /// A number written without a point or an exponent (`3`, ` 12 `, `1_000`).
    Int(i64),
    /// Any other finite number (`11.7`, `1e3`).
    Float(f64),
}

impl Point {
    /// Python's `str()` of the point: the text, the integer, or `repr` of the float.
    #[must_use]
    pub fn text(&self) -> String {
        match self {
            Point::Text(t) => t.clone(),
            Point::Int(i) => i.to_string(),
            Point::Float(f) => crate::pyfmt::repr(*f),
        }
    }

    /// Whether Python reads the point as true: non-empty text, a non-zero number.
    #[must_use]
    pub fn truthy(&self) -> bool {
        match self {
            Point::Text(t) => !t.is_empty(),
            Point::Int(i) => *i != 0,
            Point::Float(f) => *f != 0.0,
        }
    }

    /// The point as a number, where it is one.
    #[must_use]
    pub fn number(&self) -> Option<f64> {
        match self {
            Point::Text(_) => None,
            #[allow(clippy::cast_precision_loss)]
            Point::Int(i) => Some(*i as f64),
            Point::Float(f) => Some(*f),
        }
    }
}

/// A source whose points cannot be read as ooxml-edit reads them (a count past
/// [`MAX_CACHE_POINTS`], or a `c:ptCount` too large to be a number): the chart reads as
/// unreadable, as docx-agent's read of it fails.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Unreadable;

/// A data source over its container (`c:tx`, `c:cat`, `c:val`...).
#[derive(Debug, Clone, Copy)]
pub(crate) struct Source<'a> {
    reference: Option<&'a Element>,
}

impl<'a> Source<'a> {
    /// The source in `container`: its first reference child, if any.
    pub(crate) fn of(container: Option<&'a Element>) -> Source<'a> {
        let reference = container.and_then(|c| elements(c).find(|e| REFERENCES.contains(&local(e))));
        Source { reference }
    }

    /// Whether the source has a reference.
    pub(crate) fn is_some(&self) -> bool {
        self.reference.is_some()
    }

    fn kind(&self) -> Option<&'a str> {
        self.reference.map(local)
    }

    /// A number reference or literal.
    pub(crate) fn numeric(&self) -> bool {
        matches!(self.kind(), Some("numRef" | "numLit"))
    }

    /// The cache: a reference's `c:*Cache`, or a literal itself.
    pub(crate) fn cache(&self) -> Option<&'a Element> {
        let reference = self.reference?;
        match local(reference) {
            "strRef" => child(reference, "strCache"),
            "numRef" => child(reference, "numCache"),
            "multiLvlStrRef" => child(reference, "multiLvlStrCache"),
            "strLit" | "numLit" => Some(reference),
            _ => None,
        }
    }

    /// Where the points are: the cache, or a multi-level cache's first level.
    fn points_parent(&self) -> Option<&'a Element> {
        let cache = self.cache()?;
        if self.kind() == Some("multiLvlStrRef") {
            return children(cache, "lvl").next();
        }
        Some(cache)
    }

    /// The number of points: `c:ptCount` where it is digits, else the highest index plus one.
    fn count(&self) -> Result<usize, Unreadable> {
        if self.kind() == Some("v") {
            return Ok(1);
        }
        let Some(cache) = self.cache() else { return Ok(0) };
        let declared = child(cache, "ptCount")
            .and_then(|n| attr(n, "val"))
            .filter(|v| !v.is_empty() && v.bytes().all(|b| b.is_ascii_digit()));
        let count = match declared {
            Some(digits) => digits.parse::<usize>().map_err(|_| Unreadable)?,
            None => {
                let highest = self
                    .points_parent()
                    .into_iter()
                    .flat_map(|p| children(p, "pt"))
                    .map(index)
                    .max()
                    .unwrap_or(-1);
                usize::try_from(highest.saturating_add(1)).unwrap_or(0)
            }
        };
        if count > MAX_CACHE_POINTS {
            return Err(Unreadable);
        }
        Ok(count)
    }

    /// The points in index order, `None` where the cache has none (a blank).
    pub(crate) fn points(&self) -> Result<Vec<Option<Point>>, Unreadable> {
        if let Some(reference) = self.reference.filter(|r| local(r) == "v") {
            return Ok(vec![Some(Point::Text(text(reference)))]);
        }
        let mut values: Vec<Option<Point>> = vec![None; self.count()?];
        for point in self.points_parent().into_iter().flat_map(|p| children(p, "pt")) {
            let Ok(slot) = usize::try_from(index(point)) else {
                continue;
            };
            if let Some(cell) = values.get_mut(slot) {
                *cell = self.decode(child(point, "v"));
            }
        }
        Ok(values)
    }

    fn decode(&self, node: Option<&Element>) -> Option<Point> {
        let written = node.map(text).unwrap_or_default();
        if self.numeric() {
            decode_number(&written)
        } else {
            Some(Point::Text(written))
        }
    }
}

/// A point's `idx` as Python's `int()` reads it; `-1` where absent or not an integer.
fn index(point: &Element) -> i64 {
    attr(point, "idx").map_or(Some(-1), py_int).unwrap_or(-1)
}

/// A cached number as written (ooxml-edit's `decode_number`): `3` stays an integer, `11.7` a
/// float; junk and infinities are blanks. An integer outside `i64` stays a float.
#[must_use]
pub fn decode_number(written: &str) -> Option<Point> {
    let number = py_float(written).filter(|n| n.is_finite())?;
    let whole = number.fract() == 0.0 && !written.contains('.') && !written.to_lowercase().contains('e');
    #[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
    if whole && number >= i64::MIN as f64 && number < i64::MAX as f64 {
        return Some(Point::Int(number as i64));
    }
    Some(Point::Float(number))
}

#[cfg(test)]
mod tests {
    use super::{decode_number, Point};

    #[test]
    fn numbers_keep_how_they_were_written() {
        assert_eq!(decode_number("3"), Some(Point::Int(3)));
        assert_eq!(decode_number(" 1_000 "), Some(Point::Int(1000)));
        assert_eq!(decode_number("3.0"), Some(Point::Float(3.0)));
        assert_eq!(decode_number("1E3"), Some(Point::Float(1000.0)));
        assert_eq!(decode_number("inf"), None);
        assert_eq!(decode_number(""), None);
        assert_eq!(decode_number("x"), None);
        assert_eq!(Point::Float(0.1).text(), "0.1");
        assert_eq!(Point::Float(1e16).text(), "1e+16");
    }
}
