//! Owns: breaking one paragraph into lines that fit a width (ooxml-common's
//! `drawingml.wrap`): break units (spaces, Latin words, each East Asian character), kerning
//! across the joins of one run's units, a word longer than the line broken by character,
//! and Japanese line-start and line-end rules (push-out).
//!
//! Must not: position or draw a line, or decide its height.

use crate::measure::{is_cjk, Faces, TextMeasurer};
use ooxml_drawingml::drawable::{Paragraph, RunProperties};

/// A sliver of tolerance on the available width (`WRAP_TOLERANCE_RATIO`).
const WRAP_TOLERANCE_RATIO: f64 = 1e-6;
const NOT_LINE_START: &str =
    "、。，．・：；？！゛゜ゝゞヽヾ々ー゠ぁぃぅぇぉっゃゅょゎゕゖァィゥェォッャュョヮヵヶ）］｝〕〉》」』】〗〙〟｠";
const NOT_LINE_END: &str = "（［｛〔〈《「『【〖〘〝｟";

/// A run's text on one line: the text and which run it came from (runs are told apart by
/// their place in the paragraph, as the Python tells their properties apart by object).
#[derive(Debug, Clone)]
pub struct Segment<'a> {
    /// The text.
    pub text: String,
    /// The run's index in its paragraph.
    pub run: usize,
    /// The run's properties.
    pub properties: &'a RunProperties,
}

/// One line.
#[derive(Debug, Clone, Default)]
pub struct Line<'a> {
    /// Its segments.
    pub segments: Vec<Segment<'a>>,
}

#[derive(Debug, Clone)]
struct Token<'a> {
    text: String,
    run: usize,
    properties: &'a RunProperties,
    width: f64,
    breakable: bool,
    force_break: bool,
}

/// A run's faces.
pub fn faces(p: &RunProperties) -> Faces<'_> {
    Faces {
        latin: p.font_family.as_deref(),
        east_asian: p.font_family_ea.as_deref(),
    }
}

fn is_space(ch: char) -> bool {
    matches!(ch, ' ' | '\t' | '\n' | '\r')
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Space,
    Cjk,
    Latin,
}

fn fragments(text: &str) -> Vec<(String, bool)> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut kind: Option<Kind> = None;
    for ch in text.chars() {
        if is_space(ch) {
            if !current.is_empty() && kind != Some(Kind::Space) {
                out.push((std::mem::take(&mut current), kind == Some(Kind::Cjk)));
            }
            kind = Some(Kind::Space);
            current.push(ch);
        } else if is_cjk(ch) {
            if !current.is_empty() {
                out.push((
                    std::mem::take(&mut current),
                    matches!(kind, Some(Kind::Cjk | Kind::Space)),
                ));
            }
            out.push((ch.to_string(), true));
            kind = Some(Kind::Cjk);
        } else {
            if !current.is_empty() && kind != Some(Kind::Latin) {
                out.push((std::mem::take(&mut current), kind == Some(Kind::Space)));
            }
            kind = Some(Kind::Latin);
            current.push(ch);
        }
    }
    if !current.is_empty() {
        out.push((current, matches!(kind, Some(Kind::Space | Kind::Cjk))));
    }
    out
}

struct Wrapper<'m> {
    measurer: &'m dyn TextMeasurer,
    default_size: f64,
    scale: f64,
}

impl Wrapper<'_> {
    fn size(&self, p: &RunProperties) -> f64 {
        match p.font_size {
            Some(s) if s != 0.0 => s * self.scale,
            _ => self.default_size,
        }
    }

    fn tokens<'a>(&self, paragraph: &'a Paragraph) -> Vec<Token<'a>> {
        let mut tokens = Vec::new();
        let mut first = true;
        for (run_index, run) in paragraph.runs.iter().enumerate() {
            if run.text.is_empty() {
                continue;
            }
            let p = &run.properties;
            let size = self.size(p);
            for (index, part) in run.text.split('\n').enumerate() {
                if index > 0 {
                    tokens.push(Token {
                        text: String::new(),
                        run: run_index,
                        properties: p,
                        width: 0.0,
                        breakable: true,
                        force_break: true,
                    });
                    first = false;
                }
                for (fragment, breakable) in fragments(part) {
                    let width = self.measurer.width(&fragment, size, p.bold, faces(p));
                    tokens.push(Token {
                        text: fragment,
                        run: run_index,
                        properties: p,
                        width,
                        breakable: !first && breakable,
                        force_break: false,
                    });
                    first = false;
                }
            }
        }
        tokens
    }

    fn join(&self, previous: Option<&Token<'_>>, token: &Token<'_>) -> f64 {
        let Some(previous) = previous else { return 0.0 };
        if previous.run != token.run || previous.text.is_empty() || token.text.is_empty() {
            return 0.0;
        }
        let p = token.properties;
        self.measurer
            .kern_between(&previous.text, &token.text, self.size(p), p.bold, faces(p))
    }

    fn line_width(&self, tokens: &[Token<'_>]) -> f64 {
        let mut total = 0.0;
        let mut previous = None;
        for token in tokens {
            total += self.join(previous, token);
            total += token.width;
            previous = Some(token);
        }
        total
    }

    fn by_chars<'a>(&self, token: &Token<'a>, available: f64) -> Vec<Vec<Token<'a>>> {
        let mut lines = Vec::new();
        let mut current: Vec<Token<'a>> = Vec::new();
        let mut current_width = 0.0;
        let p = token.properties;
        let size = self.size(p);
        for ch in token.text.chars() {
            let text = ch.to_string();
            let width = self.measurer.width(&text, size, p.bold, faces(p));
            let mut join = current.last().map_or(0.0, |last| {
                self.measurer.kern_between(&last.text, &text, size, p.bold, faces(p))
            });
            if current_width + join + width > available && !current.is_empty() {
                lines.push(std::mem::take(&mut current));
                current_width = 0.0;
                join = 0.0;
            }
            current.push(Token {
                text,
                run: token.run,
                properties: p,
                width,
                breakable: false,
                force_break: false,
            });
            current_width += join + width;
        }
        if !current.is_empty() {
            lines.push(current);
        }
        lines
    }
}

fn merge<'a>(tokens: &[Token<'a>]) -> Vec<Segment<'a>> {
    let mut out: Vec<Segment<'a>> = Vec::new();
    for token in tokens {
        if let Some(last) = out.last_mut() {
            if last.run == token.run {
                last.text.push_str(&token.text);
                continue;
            }
        }
        out.push(Segment {
            text: token.text.clone(),
            run: token.run,
            properties: token.properties,
        });
    }
    out
}

/// Python's `str.rstrip()`.
pub fn rstrip(text: &str) -> &str {
    text.trim_end_matches(|c: char| c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c))
}

fn trim_trailing(mut segments: Vec<Segment<'_>>) -> Vec<Segment<'_>> {
    while let Some(last) = segments.last_mut() {
        let trimmed = rstrip(&last.text);
        if !trimmed.is_empty() {
            let trimmed = trimmed.to_owned();
            last.text = trimmed;
            return segments;
        }
        segments.pop();
    }
    segments
}

fn pushback(current: &[Token<'_>], token: &Token<'_>) -> usize {
    let mut moved = 0;
    loop {
        let kept = current.len() - moved;
        if kept <= 1 {
            return 0;
        }
        let head = if moved > 0 {
            current.get(kept).map_or("", |t| t.text.as_str())
        } else {
            token.text.as_str()
        };
        let tail = current.get(kept - 1).map_or("", |t| t.text.as_str());
        let starts = head.chars().next().is_some_and(|c| NOT_LINE_START.contains(c));
        let ends = tail.chars().last().is_some_and(|c| NOT_LINE_END.contains(c));
        if starts || ends {
            moved += 1;
            continue;
        }
        return moved;
    }
}

fn push_line<'a>(lines: &mut Vec<Line<'a>>, tokens: &[Token<'a>]) {
    let segments = trim_trailing(merge(tokens));
    if !segments.is_empty() {
        lines.push(Line { segments });
    }
}

impl Wrapper<'_> {
    fn layout<'a>(&self, tokens: Vec<Token<'a>>, available: f64) -> Vec<Line<'a>> {
        let mut lines: Vec<Line<'a>> = Vec::new();
        let mut current: Vec<Token<'a>> = Vec::new();
        let mut width = 0.0;
        let tolerance = available * WRAP_TOLERANCE_RATIO;
        for token in tokens {
            if token.force_break {
                lines.push(Line {
                    segments: trim_trailing(merge(&current)),
                });
                current.clear();
                width = 0.0;
                continue;
            }
            let join = self.join(current.last(), &token);
            if width + join + token.width <= available + tolerance {
                width += join + token.width;
                current.push(token);
                continue;
            }
            if current.is_empty() {
                if token.text.chars().all(is_space) {
                    continue;
                }
                let mut chunks = self.by_chars(&token, available);
                let last = chunks.pop().unwrap_or_default();
                for chunk in &chunks {
                    push_line(&mut lines, chunk);
                }
                width = self.line_width(&last);
                current = last;
                continue;
            }
            let moved = pushback(&current, &token);
            let carried = current.split_off(current.len() - moved);
            push_line(&mut lines, &current);
            current = carried;
            if !(token.breakable && token.text.chars().all(is_space)) {
                current.push(token);
            }
            width = self.line_width(&current);
        }
        if !current.is_empty() {
            push_line(&mut lines, &current);
        }
        if lines.is_empty() {
            lines.push(Line::default());
        }
        lines
    }
}

/// `paragraph` broken into lines that fit `available` px.
pub fn wrap_paragraph<'a>(
    paragraph: &'a Paragraph,
    available: f64,
    default_size: f64,
    scale: f64,
    measurer: &dyn TextMeasurer,
) -> Vec<Line<'a>> {
    if paragraph.runs.iter().all(|r| r.text.is_empty()) {
        return vec![Line::default()];
    }
    let wrapper = Wrapper {
        measurer,
        default_size,
        scale,
    };
    let tokens = wrapper.tokens(paragraph);
    if tokens.is_empty() {
        return vec![Line::default()];
    }
    wrapper.layout(tokens, available.max(1.0))
}
