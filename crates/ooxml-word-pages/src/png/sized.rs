//! Owns: a page's SVG at a device size (docx2svg's `_sized`, `render_page`'s width and
//! height): the root's natural `width="…pt" height="…pt"` replaced by whole pixels, a side
//! not given from the other by the page's own pixel shape, rounded half to even as Python
//! rounds a fraction.
//!
//! Must not: change anything but the root's size.

use ooxml_word_layout::geometry::PageFrame;

/// The height of a page `width` pixels wide, by `frame`'s pixel shape (`round(Fraction(
/// width * height_px, width_px))`).
#[must_use]
pub fn height_at(frame: &PageFrame, width: i64) -> i64 {
    rounded(i128::from(width) * i128::from(frame.height_px), frame.width_px)
}

/// The width of a page `height` pixels high, by `frame`'s pixel shape (`round(Fraction(
/// height * width_px, height_px))`).
#[must_use]
pub fn width_at(frame: &PageFrame, height: i64) -> i64 {
    rounded(i128::from(height) * i128::from(frame.width_px), frame.height_px)
}

/// `numerator / denominator` rounded half to even (Python's `round` of a `Fraction`).
fn rounded(numerator: i128, denominator: i64) -> i64 {
    let denominator = i128::from(denominator.max(1));
    let (quotient, remainder) = (numerator.div_euclid(denominator), numerator.rem_euclid(denominator));
    let twice = 2 * remainder;
    let up = twice > denominator || (twice == denominator && quotient % 2 != 0);
    i64::try_from(quotient + i128::from(up)).unwrap_or(i64::MAX)
}

/// `svg` (a page at its natural size) `width` pixels wide, and its height; `None` where
/// its root names no size in points.
#[must_use]
pub fn sized(svg: &str, frame: &PageFrame, width: i64) -> Option<(String, i64)> {
    resized(svg, frame, Some(width), None).map(|(out, _, height)| (out, height))
}

/// `svg` (a page at its natural size) at `width` and `height` pixels -- a side not given
/// from the other, in the page's shape -- and its size; `svg` itself where neither is
/// given. `None` where its root names no size in points.
#[must_use]
pub fn resized(svg: &str, frame: &PageFrame, width: Option<i64>, height: Option<i64>) -> Option<(String, i64, i64)> {
    let (width, height) = match (width, height) {
        (None, None) => return Some((svg.to_owned(), frame.width_px, frame.height_px)),
        (Some(w), None) => (w, height_at(frame, w)),
        (None, Some(h)) => (width_at(frame, h), h),
        (Some(w), Some(h)) => (w, h),
    };
    let root_end = svg.find('>')?;
    let root = svg.get(..root_end)?;
    let start = root.find(" width=\"")?;
    let after_width = start + root.get(start..)?.find("pt\" height=\"")? + "pt\" height=\"".len();
    let end = after_width + root.get(after_width..)?.find("pt\"")? + "pt\"".len();
    let mut out = String::with_capacity(svg.len());
    out.push_str(svg.get(..start)?);
    out.push_str(&format!(" width=\"{width}\" height=\"{height}\""));
    out.push_str(svg.get(end..)?);
    Some((out, width, height))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ooxml_base::Q;
    use ooxml_word_layout::px::Px;

    fn frame(width_px: i64, height_px: i64) -> PageFrame {
        PageFrame {
            width_px,
            height_px,
            width_pt: Q::from_integer(612),
            height_pt: Q::from_integer(792),
            left: Px::default(),
            right: Px::default(),
            top: Px::default(),
            bottom: Px::default(),
            section: 0,
        }
    }

    #[test]
    fn the_height_is_rounded_half_to_even() {
        // 1000 * 1056 / 816 = 1294.117...
        assert_eq!(height_at(&frame(816, 1056), 1000), 1294);
        // 3 * 5 / 2 = 7.5 -> 8; 1 * 5 / 2 = 2.5 -> 2
        assert_eq!(height_at(&frame(2, 5), 3), 8);
        assert_eq!(height_at(&frame(2, 5), 1), 2);
    }

    #[test]
    fn a_side_not_given_follows_the_page_s_shape() {
        let svg = r#"<svg width="612pt" height="792pt"><rect/></svg>"#;
        let page = frame(816, 1056);
        let (_, width, height) = resized(svg, &page, None, Some(1294)).unwrap();
        assert_eq!((width, height), (1000, 1294));
        let (out, width, height) = resized(svg, &page, Some(10), Some(20)).unwrap();
        assert_eq!(
            (out.as_str(), width, height),
            (r#"<svg width="10" height="20"><rect/></svg>"#, 10, 20)
        );
        assert_eq!(resized(svg, &page, None, None).unwrap(), (svg.to_owned(), 816, 1056));
    }

    #[test]
    fn only_the_root_s_size_changes() {
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="612pt" height="792pt" viewBox="0 0 816 1056"><rect width="5pt" height="6pt"/></svg>"#;
        let (out, height) = sized(svg, &frame(816, 1056), 1000).unwrap();
        assert_eq!(height, 1294);
        assert_eq!(
            out,
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="1000" height="1294" viewBox="0 0 816 1056"><rect width="5pt" height="6pt"/></svg>"#
        );
    }
}
