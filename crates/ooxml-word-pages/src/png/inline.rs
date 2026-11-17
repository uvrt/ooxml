//! Owns: the way in of a page's SVG to its rasteriser that docx2svg's `svg_to_png` takes --
//! a picture of a metafile drawn as SVG (its root marked `data-ooxml-metafile`) written
//! inline, as a nested `<svg>`, instead of as an `<image>` of an SVG, so its text is drawn
//! with the page's fonts (resvg draws no text in an SVG it reads through `<image>`); the
//! `<image>`'s own attributes but its box move to a group round it. ooxml-common's
//! `inline_metafile_images`, character for character.
//!
//! Must not: change the SVG handed out (only the copy drawn), or touch another picture.

use std::borrow::Cow;

/// The marker a metafile picture's root carries.
const MARKER: &str = "data-ooxml-metafile";
const DATA: &str = "data:image/svg+xml;base64,";
/// A payload past this many characters is left as it is.
const LIMIT: usize = 64 * 1024 * 1024;
/// The characters of a payload read first, for its root.
const HEAD: usize = 1024;

fn word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// `svg` with every metafile picture written inline.
pub fn inline_metafile_images(svg: &str) -> Cow<'_, str> {
    if !svg.contains(MARKER) && !svg.contains(DATA) {
        return Cow::Borrowed(svg);
    }
    let mut out = String::with_capacity(svg.len());
    let mut done = 0;
    let mut from = 0;
    let mut changed = false;
    while let Some(found) = svg.get(from..).and_then(|s| s.find("<image")) {
        let start = from + found;
        let after = start + "<image".len();
        from = after;
        if svg.get(after..).and_then(|s| s.chars().next()).is_some_and(word) {
            continue;
        }
        let Some(close) = svg.get(after..).and_then(|s| s.find('>')) else {
            break;
        };
        let end = after + close;
        let tag = svg.get(after..end).unwrap_or("");
        let Some(inner) = tag.strip_suffix('/') else {
            continue;
        };
        let Some((before, payload, rest)) = split_href(inner) else {
            continue;
        };
        let Some(written) = inline_parts(payload).map(|(view_box, content)| {
            let mut pairs = attributes(&format!("{before} {rest}"));
            let mut boxed = Vec::new();
            for name in ["x", "y", "width", "height"] {
                if let Some(at) = pairs.iter().position(|(n, _)| n == name) {
                    let (n, v) = pairs.remove(at);
                    boxed.push(format!("{n}=\"{v}\""));
                }
            }
            pairs.retain(|(n, _)| n != "preserveAspectRatio");
            let outer: String = pairs.iter().map(|(n, v)| format!(" {n}=\"{v}\"")).collect();
            let view = view_box.map(|v| format!(" viewBox=\"{v}\"")).unwrap_or_default();
            format!(
                "<g{outer}><svg {}{view} preserveAspectRatio=\"none\" overflow=\"hidden\">{content}</svg></g>",
                boxed.join(" ")
            )
        }) else {
            continue;
        };
        out.push_str(svg.get(done..start).unwrap_or(""));
        out.push_str(&written);
        done = end + 1;
        from = done;
        changed = true;
    }
    if !changed {
        return Cow::Borrowed(svg);
    }
    out.push_str(svg.get(done..).unwrap_or(""));
    Cow::Owned(out)
}

/// The tag's text before `href="data:image/svg+xml;base64,`, the payload, and the text
/// after its closing quote: the first such `href` (a whole word) followed by a payload.
fn split_href(tag: &str) -> Option<(&str, &str, &str)> {
    let needle = format!("href=\"{DATA}");
    let mut from = 0;
    while let Some(found) = tag.get(from..).and_then(|t| t.find(&needle)) {
        let at = from + found;
        from = at + 1;
        if tag.get(..at).and_then(|t| t.chars().last()).is_some_and(word) {
            continue;
        }
        let payload_at = at + needle.len();
        let rest = tag.get(payload_at..)?;
        let length = rest
            .bytes()
            .take_while(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'/' | b'='))
            .count();
        if length == 0 || rest.as_bytes().get(length) != Some(&b'"') {
            continue;
        }
        return Some((tag.get(..at)?, rest.get(..length)?, rest.get(length + 1..)?));
    }
    None
}

/// `name="value"` pairs as a regular expression finds them, a name met again keeping its
/// first place and its last value.
fn attributes(text: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let mut k = 0;
    while k < chars.len() {
        let name_char = |c: char| word(c) || c == ':' || c == '-';
        let run = chars.iter().skip(k).take_while(|(_, c)| name_char(*c)).count();
        if run == 0 {
            k += 1;
            continue;
        }
        let (Some(&(name_at, _)), after) = (chars.get(k), chars.get(k + run)) else {
            break;
        };
        let value_from = after.map(|(i, _)| *i).unwrap_or(text.len());
        let quoted = text.get(value_from..).is_some_and(|t| t.starts_with("=\""));
        let close = quoted
            .then(|| text.get(value_from + 2..).and_then(|t| t.find('"')))
            .flatten();
        let Some(close) = close else {
            k += 1;
            continue;
        };
        let name = text.get(name_at..value_from).unwrap_or("").to_owned();
        let value = text.get(value_from + 2..value_from + 2 + close).unwrap_or("").to_owned();
        match out.iter_mut().find(|(n, _)| *n == name) {
            Some(pair) => pair.1 = value,
            None => out.push((name, value)),
        }
        let next = value_from + 2 + close + 1;
        k = chars.iter().position(|(i, _)| *i >= next).unwrap_or(chars.len());
    }
    out
}

/// A metafile picture's `(viewBox, the content inside its root)` from its base64
/// payload; `None` for any other picture.
fn inline_parts(encoded: &str) -> Option<(Option<String>, String)> {
    if encoded.len() > LIMIT {
        return None;
    }
    if encoded.len() > HEAD && !drawn_here(encoded.get(..HEAD)?) {
        return None;
    }
    let data = ooxml_render::image::base64_decode(encoded)?;
    let (root_at, root_end) = root(&data)?;
    let root_tag = data.get(root_at..root_end)?;
    let trimmed = trim_end(&data);
    if root_at > 64 || !contains(root_tag, MARKER.as_bytes()) || !trimmed.ends_with(b"</svg>") {
        return None;
    }
    let close = trimmed.len() - b"</svg>".len();
    let view_box = view_box(root_tag);
    let content = String::from_utf8_lossy(data.get(root_end..close)?).into_owned();
    Some((view_box, content))
}

/// Whether a payload starting `head` may be a metafile picture (`_drawn_here`).
fn drawn_here(head: &str) -> bool {
    let Some(data) = ooxml_render::image::base64_decode(head) else {
        return false;
    };
    match root(&data) {
        None => contains(&data, b"<svg"),
        Some((at, end)) => at <= 64 && data.get(at..end).is_some_and(|t| contains(t, MARKER.as_bytes())),
    }
}

/// The first `<svg ...>` tag (`<svg\b[^>]*>`): its start and end.
fn root(data: &[u8]) -> Option<(usize, usize)> {
    let mut from = 0;
    loop {
        let at = from + find(data.get(from..)?, b"<svg")?;
        from = at + 1;
        let next = data.get(at + 4).copied();
        if next.is_some_and(|b| b.is_ascii_alphanumeric() || b == b'_') {
            continue;
        }
        let end = at + find(data.get(at..)?, b">")? + 1;
        return Some((at, end));
    }
}

fn view_box(tag: &[u8]) -> Option<String> {
    let at = find(tag, b"viewBox=\"")? + b"viewBox=\"".len();
    let rest = tag.get(at..)?;
    let close = find(rest, b"\"")?;
    Some(String::from_utf8_lossy(rest.get(..close)?).into_owned())
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    find(hay, needle).is_some()
}

/// `bytes.rstrip()`: ASCII whitespace off the end.
fn trim_end(data: &[u8]) -> &[u8] {
    let keep = data.iter().rposition(|b| !matches!(b, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c));
    data.get(..keep.map_or(0, |k| k + 1)).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encode(text: &str) -> String {
        const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = String::new();
        for chunk in text.as_bytes().chunks(3) {
            let n = chunk.iter().enumerate().fold(0u32, |n, (i, b)| n | u32::from(*b) << (16 - 8 * i));
            for k in 0..4 {
                if k <= chunk.len() {
                    out.push(char::from(ALPHABET[(n >> (18 - 6 * k) & 63) as usize]));
                } else {
                    out.push('=');
                }
            }
        }
        out
    }

    #[test]
    fn a_metafile_picture_goes_inline_and_another_stays() {
        let drawn = encode(r#"<svg xmlns="http://www.w3.org/2000/svg" data-ooxml-metafile="emf" viewBox="0 0 10 5"><text>A</text></svg>"#);
        let other = encode(r#"<svg xmlns="http://www.w3.org/2000/svg"><rect/></svg>"#);
        let svg = format!(
            r#"<svg><image data-docx-path="p" x="1" y="2" width="3" height="4" preserveAspectRatio="none" href="data:image/svg+xml;base64,{drawn}"/><image href="data:image/svg+xml;base64,{other}"/></svg>"#
        );
        let out = inline_metafile_images(&svg);
        assert_eq!(
            out,
            format!(
                r#"<svg><g data-docx-path="p"><svg x="1" y="2" width="3" height="4" viewBox="0 0 10 5" preserveAspectRatio="none" overflow="hidden"><text>A</text></svg></g><image href="data:image/svg+xml;base64,{other}"/></svg>"#
            )
        );
        assert!(matches!(inline_metafile_images("<svg/>"), Cow::Borrowed(_)));
    }
}
