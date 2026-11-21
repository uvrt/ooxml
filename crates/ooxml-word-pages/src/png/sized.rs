//! Owns: a page's SVG at a device size (docx2svg's `_sized`, `render_page`'s width): the
//! root's natural `width="…pt" height="…pt"` replaced by whole pixels, the height from the
//! width by the page's own pixel shape, rounded half to even as Python rounds a fraction.
//!
//! Must not: change anything but the root's size.

use ooxml_word_layout::geometry::PageFrame;

/// The height of a page `width` pixels wide, by `frame`'s pixel shape (`round(Fraction(
/// width * height_px, width_px))`).
#[must_use]
pub fn height_at(frame: &PageFrame, width: i64) -> i64 {
    let numerator = i128::from(width) * i128::from(frame.height_px);
    let denominator = i128::from(frame.width_px.max(1));
    let (quotient, remainder) = (numerator.div_euclid(denominator), numerator.rem_euclid(denominator));
    let twice = 2 * remainder;
    let up = twice > denominator || (twice == denominator && quotient % 2 != 0);
    i64::try_from(quotient + i128::from(up)).unwrap_or(i64::MAX)
}

/// `svg` (a page at its natural size) `width` pixels wide, and its height; `None` where
/// its root names no size in points.
#[must_use]
pub fn sized(svg: &str, frame: &PageFrame, width: i64) -> Option<(String, i64)> {
    let root_end = svg.find('>')?;
    let root = svg.get(..root_end)?;
    let start = root.find(" width=\"")?;
    let after_width = start + root.get(start..)?.find("pt\" height=\"")? + "pt\" height=\"".len();
    let end = after_width + root.get(after_width..)?.find("pt\"")? + "pt\"".len();
    let height = height_at(frame, width);
    let mut out = String::with_capacity(svg.len());
    out.push_str(svg.get(..start)?);
    out.push_str(&format!(" width=\"{width}\" height=\"{height}\""));
    out.push_str(svg.get(end..)?);
    Some((out, height))
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
