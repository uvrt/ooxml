//! Owns: the pictures a Markdown text shows, read as docx-agent's `_pictures` reads them
//! before anything changes: each source once, in the order the blocks and then the footnote
//! definitions show them; a `data:` URI decoded (base64 as Python's `b64decode` reads it,
//! else percent-decoded), a remote one refused (nothing is fetched), any other source the
//! host's to hand over ([`read_pictures`]); each checked as a picture (its format and size).
//! And a picture written as docx-agent's `Writer.picture` writes it: an inline drawing in a
//! run of its own, its bytes a media part, its `docPr` numbered past every drawing the
//! document has had, its name the Markdown's title (else `Picture <number>`), its size the
//! natural one scaled down to the text's width where wider.
//!
//! Must not: look a source up anywhere but in what the host hands over, or write but
//! through the sink.

use super::parse;
use super::writer::{walk_inline, Writer};
use crate::drawings::{image_format, inline_xml, natural_size, PictureFrame};
use crate::review::xml::make;
use crate::sink::EditSink;
use crate::work::{Id, Work};
use crate::{EditError, Result};
use ooxml_base::pyfmt::repr_str;
use ooxml_base::Limits;
use ooxml_word_model::markdown::model::{Block, Inline};
use std::collections::BTreeMap;
use std::sync::Arc;

/// Each picture's bytes, by its source as the Markdown gives it.
pub type Pictures = BTreeMap<String, Arc<[u8]>>;

/// Why the pictures could not be read: the host's refusal of a source it holds, or the
/// edit's (a remote source, an unreadable `data:` URI, bytes that are no picture).
#[derive(Debug)]
pub enum PictureRefusal<E> {
    /// The host's.
    Host(E),
    /// The edit's.
    Edit(EditError),
}

impl<E> From<EditError> for PictureRefusal<E> {
    fn from(error: EditError) -> Self {
        Self::Edit(error)
    }
}

/// The pictures `markdown` shows, each read and checked in order: a `data:` URI decoded, a
/// remote source refused, any other source's bytes as `hosted` hands them over.
///
/// # Errors
/// The first refusal: `hosted`'s, or the edit's.
pub fn read_pictures<E>(
    markdown: &str,
    mut hosted: impl FnMut(&str) -> std::result::Result<Arc<[u8]>, E>,
) -> std::result::Result<Pictures, PictureRefusal<E>> {
    let blocks = parse::parse_model(markdown)?;
    let mut out = Pictures::new();
    for (alt, src) in shown(&blocks) {
        if out.contains_key(src) {
            continue;
        }
        let data = match inline(src) {
            Some(read) => Arc::from(read?),
            None => hosted(src).map_err(PictureRefusal::Host)?,
        };
        checked(alt, src, &data)?;
        out.insert(src.to_owned(), data);
    }
    Ok(out)
}

/// Every picture's bytes `blocks` (and the footnote definitions among them) show, from
/// `given` or decoded from its `data:` URI (`_pictures` with a mapping of images).
///
/// # Errors
/// A source neither given nor readable here; bytes that are no picture.
pub(crate) fn given_pictures(blocks: &[Block], given: &Pictures) -> Result<Pictures> {
    let mut out = Pictures::new();
    for (alt, src) in shown(blocks) {
        if out.contains_key(src) {
            continue;
        }
        let data = match (given.get(src), inline(src)) {
            (Some(data), _) => Arc::clone(data),
            (None, Some(read)) => Arc::from(read?),
            (None, None) => {
                return Err(EditError::Refused(format!(
                    "{} is not among the images given",
                    repr_str(src)
                )))
            }
        };
        checked(alt, src, &data)?;
        out.insert(src.to_owned(), data);
    }
    Ok(out)
}

/// `(alt, src)` of every picture: the blocks' (footnote definitions left out), then each
/// footnote definition's (the last of a label, where the label first came).
fn shown(blocks: &[Block]) -> Vec<(&str, &str)> {
    let mut notes: Vec<(&str, &Block)> = Vec::new();
    for block in blocks {
        if let Block::Footnote(label, _) = block {
            match notes.iter_mut().find(|(l, _)| l == label) {
                Some(entry) => entry.1 = block,
                None => notes.push((label.as_str(), block)),
            }
        }
    }
    let mut inline = Vec::new();
    for block in blocks
        .iter()
        .filter(|b| !matches!(b, Block::Footnote(..) | Block::Comment(_)))
    {
        walk_inline(std::slice::from_ref(block), &mut inline);
    }
    for (_, note) in &notes {
        walk_inline(std::slice::from_ref(*note), &mut inline);
    }
    inline
        .into_iter()
        .filter_map(|item| match item {
            Inline::Image { alt, src, .. } => Some((alt.as_str(), src.as_str())),
            _ => None,
        })
        .collect()
}

impl Writer<'_> {
    /// The run of the picture `![alt](src "title")` (its bytes read before anything changed).
    pub(super) fn picture(
        &mut self,
        sink: &mut EditSink<'_, '_>,
        work: &mut Work,
        src: &str,
        alt: &str,
        title: Option<&str>,
    ) -> Result<Id> {
        let data = self
            .pictures
            .get(src)
            .cloned()
            .ok_or_else(|| EditError::Refused(format!("{} is not among the images given", repr_str(src))))?;
        let rid = sink.media(&self.part, &data, image_format(&data)?)?;
        let number = sink.draw_drawings(1)?.first().copied().unwrap_or(1);
        let (mut width, mut height) = natural_size(&data)?;
        #[allow(clippy::cast_precision_loss)] // a text width in twentieths of a point
        let limit = self.text_width(sink)? as f64 / 20.0;
        if width > limit {
            (width, height) = (limit, height * limit / width);
        }
        let ids = sink.draw(&format!("{}\0picture", self.part), 2)?;
        let name = match title.filter(|t| !t.is_empty()) {
            Some(title) => title.to_owned(),
            None => format!("Picture {number}"),
        };
        let frame = PictureFrame {
            rid: &rid,
            number,
            name: &name,
            alt: Some(alt),
            size: (width, height),
            anchor_id: ids.first().map_or("", String::as_str),
            edit_id: ids.get(1).map_or("", String::as_str),
        };
        let parsed = ooxml_xml::parse(inline_xml(&frame).as_bytes(), &Limits::default().xml)
            .map_err(|e| EditError::Refused(e.to_string()))?;
        let mut scope = self.scope.clone();
        let drawing = crate::namespaces::strip_redundant(parsed.root(), &mut scope);
        let drawing = work.adopt(&drawing).ok_or_else(|| EditError::Path(self.part.clone()))?;
        let run = make(work, "r", &[]);
        work.append(run, drawing);
        self.drew = true;
        Ok(run)
    }
}

/// `![alt](src)`'s bytes checked as a picture (`image_format`, `natural_size`).
fn checked(alt: &str, src: &str, data: &[u8]) -> Result<()> {
    image_format(data)
        .and_then(|_| natural_size(data))
        .map(|_| ())
        .map_err(|error| match error {
            EditError::Refused(why) => EditError::Refused(format!("![{alt}]({src}): {why}")),
            other => other,
        })
}

/// A source read here (`_read_image`'s `data:` and remote schemes), `None` for the host's.
fn inline(src: &str) -> Option<Result<Vec<u8>>> {
    let scheme = scheme(src).to_ascii_lowercase();
    if matches!(scheme.as_str(), "http" | "https" | "ftp") {
        return Some(Err(EditError::Refused(format!(
            "{} is remote: pass fetch= to read it (nothing is fetched otherwise)",
            repr_str(src)
        ))));
    }
    if scheme != "data" {
        return None;
    }
    let (header, payload) = src.split_once(',').unwrap_or((src, ""));
    if !header.ends_with(";base64") {
        return Some(Ok(unquote_to_bytes(payload)));
    }
    let unquoted = super::percent_decode(payload);
    Some(b64decode(&unquoted).ok_or_else(|| {
        let shown: String = src.chars().take(40).collect();
        EditError::Refused(format!("{}... is not a readable data URI", repr_str(&shown)))
    }))
}

/// `urllib.parse.urlsplit(src).scheme` as written: leading C0 controls and spaces and every
/// tab and line break dropped, then the letters before the first `:` when they make a
/// scheme (a letter, then letters, digits, `+`, `-` or `.`).
fn scheme(src: &str) -> String {
    let cleaned: String = src
        .trim_start_matches(|c: char| c <= ' ')
        .chars()
        .filter(|c| !matches!(c, '\t' | '\r' | '\n'))
        .collect();
    let Some((head, _)) = cleaned.split_once(':') else {
        return String::new();
    };
    let mut chars = head.chars();
    let starts = chars.next().is_some_and(|c| c.is_ascii_alphabetic());
    let rest = chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'));
    if starts && rest {
        head.to_owned()
    } else {
        String::new()
    }
}

/// `urllib.parse.unquote_to_bytes`: the text's UTF-8 with `%XX` sequences decoded.
fn unquote_to_bytes(text: &str) -> Vec<u8> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut k = 0;
    while let Some(&b) = bytes.get(k) {
        let hex = |i: usize| bytes.get(i).and_then(|c| char::from(*c).to_digit(16));
        match (b, hex(k + 1), hex(k + 2)) {
            (b'%', Some(h), Some(l)) => {
                out.push(u8::try_from(h * 16 + l).unwrap_or(b'?'));
                k += 3;
            }
            _ => {
                out.push(b);
                k += 1;
            }
        }
    }
    out
}

/// Python's `base64.b64decode(text)` (not validating): characters outside the alphabet
/// skipped, decoding done at the padding that completes a quad; `None` where Python raises
/// (text not ASCII, a quad left one character long, padding missing).
fn b64decode(text: &str) -> Option<Vec<u8>> {
    if !text.is_ascii() {
        return None;
    }
    let value = |c: u8| -> Option<u32> {
        Some(u32::from(match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        }))
    };
    let mut out = Vec::with_capacity(text.len() * 3 / 4);
    let (mut quad, mut left, mut pads) = (0u32, 0u32, 0u32);
    for c in text.bytes() {
        if c == b'=' {
            pads += 1;
            if quad >= 2 && quad + pads >= 4 {
                return Some(out);
            }
            continue;
        }
        let Some(v) = value(c) else { continue };
        pads = 0;
        let byte = |n: u32| u8::try_from(n & 0xff).unwrap_or_default();
        match quad {
            0 => (quad, left) = (1, v),
            1 => {
                out.push(byte((left << 2) | (v >> 4)));
                (quad, left) = (2, v & 0x0f);
            }
            2 => {
                out.push(byte((left << 4) | (v >> 2)));
                (quad, left) = (3, v & 0x03);
            }
            _ => {
                out.push(byte((left << 6) | v));
                (quad, left) = (0, 0);
            }
        }
    }
    (quad == 0).then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_reads_as_python_does() {
        assert_eq!(b64decode("aGk=").as_deref(), Some(&b"hi"[..]));
        assert_eq!(b64decode("aG k=\n").as_deref(), Some(&b"hi"[..]));
        assert_eq!(b64decode("aGk=trailing").as_deref(), Some(&b"hi"[..]));
        assert_eq!(b64decode("aGVsbG8"), None);
        assert_eq!(b64decode("a"), None);
        assert_eq!(b64decode(""), Some(Vec::new()));
        assert_eq!(b64decode("\u{e9}"), None);
    }

    #[test]
    fn schemes_read_as_urlsplit_does() {
        assert_eq!(scheme(" HTTP://x"), "HTTP");
        assert_eq!(scheme("da\nta:x"), "data");
        assert_eq!(scheme("b1"), "");
        assert_eq!(scheme("1a:b"), "");
        assert_eq!(scheme("C:/x.png"), "C");
    }

    #[test]
    fn sources_read_here_or_left_to_the_host() {
        assert!(inline("b1").is_none());
        assert!(matches!(inline("https://x/a.png"), Some(Err(_))));
        assert_eq!(inline("data:,a%20b").and_then(Result::ok).as_deref(), Some(&b"a b"[..]));
        assert_eq!(
            inline("data:image/png;base64,aGk=").and_then(Result::ok).as_deref(),
            Some(&b"hi"[..])
        );
        assert!(matches!(inline("data:;base64,a"), Some(Err(_))));
    }
}
