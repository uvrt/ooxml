//! Word's line breaking: a paragraph's runs -> pieces (`linebreak.pieces`) -> lines
//! (`linebreak.break_pieces`, greedy first fit, exact) -> each line's height
//! (`lines.shares_of`, then the core's `line_height`).  Split out of the prototype's
//! measure.rs, which the core keeps for font metrics, widths and line heights.

use crate::measure::*;
use crate::model::{GeometryIn, MeasureIn};
use crate::units::exact_div;
use num_integer::Integer;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Line {
    pub start: usize,
    pub end: usize,
    pub forced: bool,
    pub emergency: bool,
    pub hyphenated: bool,
}

impl Line {
    fn new(start: usize, end: usize) -> Line {
        Line { start, end, forced: false, emergency: false, hyphenated: false }
    }
    pub fn flags(&self) -> u8 {
        self.forced as u8 | (self.emergency as u8) << 1 | (self.hyphenated as u8) << 2
    }
}

/// `linebreak.Geometry`, in 1/`s` units.
pub struct Geometry {
    first_start: i64,
    start: i64,
    right: i64,
    tabs: Vec<(i64, u8)>,
    default_tab: i64,
    hanging_stop: Option<i64>,
    squeeze: bool,
    tab_origin: i64,
}

// Tab alignments, ordered as their names compare (Python's `min` over `(stop, alignment)`).
fn align_code(a: &str) -> u8 {
    match a {
        "center" => 1,
        "decimal" => 2,
        "end" => 3,
        "left" => 4,
        "num" => 5,
        "right" => 6,
        "start" => 7,
        _ => 0,
    }
}
const LEFT: u8 = 4;

impl Geometry {
    pub fn new(t: &Tables, g: &GeometryIn) -> Geometry {
        Geometry {
            first_start: t.w(g.first_start),
            start: t.w(g.start),
            right: t.w(g.right),
            tabs: g.tabs.iter().map(|(p, a)| (t.w(*p), align_code(a))).collect(),
            default_tab: t.w(g.default_tab),
            hanging_stop: g.hanging_stop.map(|v| t.w(v)),
            squeeze: g.squeeze,
            tab_origin: t.w(g.tab_origin),
        }
    }

    /// `linebreak.next_tab`: the stop, its alignment, whether it is a default stop.
    fn next_tab(&self, x: i64, first: bool) -> (i64, u8, bool) {
        let origin = self.tab_origin;
        let mut best: Option<(i64, u8)> = None;
        for &(p, a) in &self.tabs {
            if p + origin > x {
                let c = (p + origin, a);
                if best.map_or(true, |b| c < b) {
                    best = Some(c);
                }
            }
        }
        if first {
            if let Some(h) = self.hanging_stop {
                if h > x {
                    let c = (h, LEFT);
                    if best.map_or(true, |b| c < b) {
                        best = Some(c);
                    }
                }
            }
        }
        if let Some((s, a)) = best {
            return (s, a, false);
        }
        let last = self.tabs.iter().map(|t| t.0).max().unwrap_or(0) + origin;
        let base = x.max(last);
        let step = self.default_tab;
        (((base - origin).div_floor(&step) + 1) * step + origin, LEFT, true)
    }
}

fn is_space(c: char) -> bool {
    matches!(c, ' ' | '\u{2002}' | '\u{2003}' | '\u{2005}' | '\u{200b}')
}

/// `linebreak.pieces` (with `label_pieces` first).
pub fn pieces(t: &Tables, m: &MeasureIn) -> Vec<Piece> {
    let mut out: Vec<Piece> = Vec::with_capacity(256);
    if let Some(label) = &m.label {
        let mut lab = Vec::new();
        for (ch, f) in &label.chars {
            let f = t.fmts[*f];
            let font = &t.fonts[f.font as usize];
            let c = ch.chars().next().unwrap();
            lab.push(Piece {
                kind: GLYPH,
                ch: c,
                width: t.units(font.advance(c), font.upm, f.hp),
                kern_next: 0,
                kkey: -1,
                source: -1,
                hyphen: 0,
                run: -1,
                hp: f.hp,
            });
        }
        let total: i64 = lab.iter().map(|p| p.width).sum();
        let shift = match label.align.as_str() {
            "center" => exact_div(total as i128, 2),
            "right" => total,
            _ => 0,
        };
        if shift != 0 {
            out.push(Piece { kind: GLYPH, ch: '\0', width: -shift, kern_next: 0, kkey: -1, source: -1, hyphen: 0, run: -1, hp: 0 });
        }
        out.extend(lab);
        match label.suffix.as_str() {
            "tab" => out.push(Piece { kind: TAB, ch: '\t', width: 0, kern_next: 0, kkey: -1, source: -1, hyphen: 0, run: -1, hp: 0 }),
            "space" => {
                let f = t.fmts[label.space_fmt];
                let font = &t.fonts[f.font as usize];
                out.push(Piece {
                    kind: GLYPH,
                    ch: ' ',
                    width: t.units(font.advance(' '), font.upm, f.hp),
                    kern_next: 0,
                    kkey: -1,
                    source: -1,
                    hyphen: 0,
                    run: -1,
                    hp: 0,
                });
            }
            _ => {}
        }
    }
    let mut source: i32 = 0;
    for (ri, run) in m.runs.iter().enumerate() {
        let first_new = out.len();
        let n = run.text.chars().count() as i32;
        if run.vanish {
            source += n;
            continue;
        }
        let glyph = |c: char, kind: u8, index: i32, f: usize| -> Piece {
            let f = t.fmts[f];
            let drawn = if run.caps { c.to_uppercase().next().unwrap() } else { c };
            let font = &t.fonts[f.font as usize];
            let mut width = t.units(font.advance(drawn), font.upm, f.whp);
            if run.scale != 100 && run.scale != 0 {
                width = exact_div(width as i128 * run.scale as i128, 100);
            }
            if run.tracking != 0 {
                width += run.tracking * t.s;
            }
            let kkey = if run.kern != 0 && f.hp >= run.kern { f.font * 100_000 + f.whp } else { -1 };
            Piece { kind, ch: drawn, width, kern_next: 0, kkey, source: index, hyphen: 0, run: ri as i32, hp: f.hp }
        };
        let mark = |kind: &str, out: &mut Vec<Piece>| match kind {
            "tab" => out.push(Piece { kind: TAB, ch: '\t', width: 0, kern_next: 0, kkey: -1, source, hyphen: 0, run: -1, hp: 0 }),
            "br" | "textWrapping" | "page" | "column" | "cr" => {
                let ch = match kind {
                    "page" => '\u{c}',
                    "column" => '\u{b}',
                    _ => '\n',
                };
                out.push(Piece { kind: BREAK, ch, width: 0, kern_next: 0, kkey: -1, source, hyphen: 0, run: -1, hp: 0 })
            }
            "softHyphen" => {
                let h = glyph('-', GLYPH, source, run.hyphen_fmt);
                out.push(Piece { kind: SOFT, ch: '\u{ad}', width: 0, kern_next: 0, kkey: -1, source, hyphen: h.width, run: -1, hp: 0 })
            }
            "noBreakHyphen" => {
                let mut p = glyph('-', GLYPH, source, run.hyphen_fmt);
                p.ch = '\u{2011}';
                out.push(p)
            }
            other => panic!("mark {other} not ported"),
        };
        let uniform = run.fmts.len() == 1;
        let mut mi = 0;
        let marks = &run.marks;
        for (offset, c) in run.text.chars().enumerate() {
            while mi < marks.len() && marks[mi].0 == offset {
                mark(&marks[mi].1, &mut out);
                mi += 1;
            }
            let f = if uniform { run.fmts[0] } else { run.fmts[offset] };
            out.push(glyph(c, if is_space(c) { SPACE } else { GLYPH }, source + offset as i32, f));
        }
        while mi < marks.len() {
            assert!(marks[mi].0 == n as usize, "marks out of order");
            mark(&marks[mi].1, &mut out);
            mi += 1;
        }
        for p in &mut out[first_new..] {
            p.run = ri as i32;
        }
        source += n;
    }
    // kern pairs (`linebreak._kern(piece, next)`)
    for j in 0..out.len().saturating_sub(1) {
        let (a, b) = (out[j], out[j + 1]);
        if b.kkey >= 0 && a.kkey == b.kkey {
            let f = (b.kkey / 100_000) as usize;
            let hp = b.kkey % 100_000;
            let font = &t.fonts[f];
            let v = font.kern(a.ch, b.ch);
            if v != 0 {
                out[j].kern_next = t.units(v, font.upm, hp);
            }
        }
    }
    out
}

fn may_break(before: &Piece, after: &Piece) -> bool {
    if after.kind == SPACE {
        return false;
    }
    if before.kind == SPACE || after.kind == TAB || before.kind == SOFT {
        return true;
    }
    before.kind == GLYPH && matches!(before.ch, '-' | '\u{2013}' | '\u{2014}') && before.source >= 0
}

/// `linebreak.squeeze`: a quarter of a space, half up to the unit.
fn squeeze(s: i64, p: &Piece) -> i64 {
    (p.width + 2 * s).div_floor(&(4 * s)) * s
}

fn squeezes(over: i64, room: i64, spaces: i64, slack: i64, spaces_before: i64) -> bool {
    if over > room {
        return false;
    }
    spaces_before == 0 || 2 * over as i128 * spaces_before as i128 <= slack as i128 * spaces as i128
}

const INF: i64 = i64::MAX / 4;

/// `linebreak.break_line`.
pub fn break_line(s: i64, items: &[Piece], i: usize, g: &Geometry, first: bool) -> Line {
    let n = items.len();
    let mut x = if first { g.first_start } else { g.start };
    let start = i;
    let mut last_break: Option<usize> = None;
    let mut text_end = x;
    let mut limit = g.right;
    let (mut room, mut spaces, mut before_spaces) = (0i64, 0i64, 0i64);
    let mut before_x = x;
    let mut j = i;
    while j < n {
        let piece = &items[j];
        if j > start && may_break(&items[j - 1], piece) {
            let before = &items[j - 1];
            if before.kind != SOFT || x + before.hyphen <= g.right {
                last_break = Some(j - 1);
            }
        }
        if piece.kind == BREAK {
            return Line { start, end: j + 1, forced: true, emergency: false, hyphenated: false };
        }
        let x_new;
        if piece.kind == TAB {
            let (stop, align, default) = g.next_tab(x, first);
            if matches!(align, 6 | 1 | 2) {
                x_new = x;
                limit = if stop <= g.right { g.right } else if align != 1 { stop } else { 2 * stop - x };
            } else if stop > g.right && !default {
                x_new = stop;
                limit = INF;
            } else {
                x_new = stop;
                limit = g.right;
            }
            room = 0;
            spaces = 0;
            before_spaces = 0;
            before_x = x_new;
        } else {
            x_new = x + piece.width + piece.kern_next;
            if piece.kind == SPACE && g.squeeze {
                if j == start || items[j - 1].kind != SPACE {
                    before_x = x;
                    before_spaces = spaces;
                }
                room += squeeze(s, piece);
                spaces += 1;
            }
        }
        if piece.kind != SPACE
            && piece.kind != SOFT
            && x_new > limit
            && j > start
            && !(g.squeeze && squeezes(x_new - limit, room, spaces, limit - before_x, before_spaces))
        {
            return match last_break {
                Some(at) => Line { start, end: at + 1, forced: false, emergency: false, hyphenated: items[at].kind == SOFT },
                None => Line { start, end: j, forced: false, emergency: true, hyphenated: false },
            };
        }
        if piece.kind == GLYPH || piece.kind == TAB {
            text_end = x_new;
        }
        x = x_new;
        j += 1;
    }
    let _ = text_end;
    Line::new(start, n)
}

/// `linebreak.break_pieces`.
pub fn break_pieces(s: i64, items: &[Piece], g: &Geometry, columns: bool) -> Vec<Line> {
    let n = items.len();
    let mut lines: Vec<Line> = Vec::new();
    let mut i = 0;
    while i < n || lines.is_empty() {
        let line = break_line(s, items, i, g, lines.is_empty());
        lines.push(line);
        i = line.end;
        if i >= n {
            if line.forced {
                let c = items[n - 1].ch;
                if c == '\n' || (columns && c == '\u{b}') {
                    lines.push(Line::new(n, n));
                }
            }
            break;
        }
    }
    lines
}

pub struct Measured {
    pub pieces: Vec<Piece>,
    pub lines: Vec<Line>,
    pub heights: Vec<LineH>,
}

/// `lines.shares_of` -> per line the formats that set its height, then `line_height`.
pub fn measure(t: &Tables, m: &MeasureIn) -> Measured {
    let items = pieces(t, m);
    let g = Geometry::new(t, &m.geometry);
    let lines = break_pieces(t.s, &items, &g, m.columns);
    // which line each character is on
    let total: usize = m.runs.iter().map(|r| r.text.chars().count()).sum();
    let mut line_of: Vec<i32> = vec![-1; total + 1]; // a mark at the end carries the index after the last character
    for (number, line) in lines.iter().enumerate() {
        for p in &items[line.start..line.end] {
            if p.source >= 0 && p.kind != TAB && p.kind != BREAK {
                let slot = &mut line_of[p.source as usize];
                if *slot < 0 {
                    *slot = number as i32;
                }
            }
        }
    }
    // per line: formats of its non-space characters (deduplicated by what sets a height), or none
    let mut per_line: Vec<Vec<usize>> = vec![Vec::new(); lines.len()];
    let mut any_text: Vec<bool> = vec![false; lines.len()];
    let mut current = 0usize;
    let mut index = 0usize;
    for run in &m.runs {
        let n = run.text.chars().count();
        if !run.vanish {
            let uniform = run.fmts.len() == 1;
            for (offset, c) in run.text.chars().enumerate() {
                if line_of[index + offset] >= 0 {
                    current = line_of[index + offset] as usize;
                }
                if c != ' ' {
                    any_text[current] = true;
                    let f = if uniform { run.fmts[0] } else { run.fmts[offset] };
                    let key = height_key(&t.fmts[f]);
                    if !per_line[current].iter().any(|k| height_key(&t.fmts[*k]) == key) {
                        per_line[current].push(f);
                    }
                }
            }
        }
        index += n;
    }
    let h = Heights { t };
    let mut label: Vec<usize> = Vec::new();
    for f in &m.label_height_fmts {
        if !label.iter().any(|k| height_key(&t.fmts[*k]) == height_key(&t.fmts[*f])) {
            label.push(*f);
        }
    }
    let mut cache: Vec<(Vec<usize>, bool, LineH)> = Vec::new();
    let mut heights = Vec::with_capacity(lines.len());
    for number in 0..lines.len() {
        let text: Vec<usize> = if any_text[number] { per_line[number].clone() } else { vec![m.mark_fmt] };
        let first = number == 0;
        if let Some((_, _, v)) = cache.iter().find(|(k, f, _)| k == &text && (*f == first || label.is_empty())) {
            heights.push(*v);
            continue;
        }
        let v = h.line_height(&text, if first { &label } else { &[] }, m.rule, m.line).expect("no face metrics");
        cache.push((text, first, v));
        heights.push(v);
    }
    Measured { pieces: items, lines, heights }
}

