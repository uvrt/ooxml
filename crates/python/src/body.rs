//! Lane K: a Word body's top-level blocks found in its part's bytes, and the blocks of the
//! next version found from them, as docx-agent's passes find them (`tools/touched.py`'s
//! `_scan`, `_blocks`, `_rescan` and `_kept_counts`, ported step for step).
//!
//! Every pass of a version (the touched sections, revalidate's body facts, the ids note, the
//! body's index, the owner's change set) reads the same change between the same two
//! versions' bytes: the bytes before the first that differs and after the last, the blocks
//! kept on either side, the blocks between found again by their tags.  One `BodySpans` per
//! version holds the blocks' spans (no Python object per block), and the step from one
//! version to the next is made once and kept (`advance`), so each pass after the first
//! takes it as it is.  The work is done with the interpreter lock let go.
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyList, PySlice, PyTuple};
use std::sync::{Arc, Mutex};

/// The blocks' tags: `<w:p`, `<w:tbl`, `<w:sdt`, `<w:customXml`, `<w:body` (and their end
/// tags), followed by white space, `/` or `>`.
const NAMES: [&[u8]; 5] = [b"p", b"tbl", b"sdt", b"customXml", b"body"];

/// Python's `\s` in a bytes pattern.
fn space(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c)
}

/// The tag at `lt` (a `<`) within `data[..stop]`: `(closing, name index, end of the name)`.
fn tag_at(data: &[u8], lt: usize, stop: usize) -> Option<(bool, usize, usize)> {
    let mut at = lt + 1;
    let closing = at < stop && data[at] == b'/';
    if closing {
        at += 1;
    }
    if at + 2 > stop || &data[at..at + 2] != b"w:" {
        return None;
    }
    at += 2;
    for (k, name) in NAMES.iter().enumerate() {
        let end = at + name.len();
        // the character after the name must be within the stretch (the pattern's end)
        if end < stop && &data[at..end] == *name && (space(data[end]) || data[end] == b'/' || data[end] == b'>') {
            return Some((closing, k, end));
        }
    }
    None
}

fn find_byte(data: &[u8], from: usize, byte: u8) -> Option<usize> {
    data.get(from..)?.iter().position(|&b| b == byte).map(|k| from + k)
}

/// `_blocks(data, begin, stop)`: the top-level blocks between `begin` and `stop`, by their
/// tags; `None` where the tags do not nest there (or the body's own tag is met).
pub fn blocks(data: &[u8], begin: usize, stop: usize) -> Option<Vec<(usize, usize)>> {
    let stop = stop.min(data.len());
    let mut depth: i64 = 0;
    let mut start = begin;
    let mut spans = Vec::new();
    let mut at = begin;
    while at < stop {
        let lt = match find_byte(&data[..stop], at, b'<') {
            Some(lt) => lt,
            None => break,
        };
        let (closing, name, end) = match tag_at(data, lt, stop) {
            Some(found) => found,
            None => {
                at = lt + 1;
                continue;
            }
        };
        at = end;
        if name == 4 {
            return None;
        }
        if closing {
            depth -= 1;
            if depth < 0 {
                return None;
            }
            if depth == 0 {
                spans.push((start, find_byte(data, end, b'>')? + 1));
            }
            continue;
        }
        let close = find_byte(data, end, b'>')?;
        if data[close - 1] == b'/' {
            if depth == 0 {
                spans.push((lt, close + 1));
            }
            continue;
        }
        if depth == 0 {
            start = lt;
        }
        depth += 1;
    }
    if depth == 0 {
        Some(spans)
    } else {
        None
    }
}

const W_DECLARED: &[u8] = b"xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\"";

fn find(data: &[u8], from: usize, what: &[u8]) -> Option<usize> {
    if from > data.len() || what.len() > data.len() - from {
        return None;
    }
    data[from..]
        .windows(what.len())
        .position(|w| w == what)
        .map(|k| from + k)
}

fn rfind(data: &[u8], what: &[u8]) -> Option<usize> {
    if what.len() > data.len() {
        return None;
    }
    data.windows(what.len()).rposition(|w| w == what)
}

/// Python's `data.find(b">", data.find(b"<w:document"))`: a missing `<w:document` searches
/// from the last byte.
fn root_end(data: &[u8]) -> Option<usize> {
    let from = match find(data, 0, b"<w:document") {
        Some(at) => at,
        None => data.len().saturating_sub(1),
    };
    find_byte(data, from, b'>')
}

/// `_scan(data)`'s blocks: `(spans, body, root_end)`, or `None` where the part does not
/// declare the main namespace as `w:` or the tags do not nest.
#[allow(clippy::type_complexity)]
pub fn scan(data: &[u8]) -> Option<(Vec<(usize, usize)>, (usize, usize), usize)> {
    let root_end = root_end(data)?;
    find(&data[..root_end], 0, W_DECLARED)?;
    // `<w:body(?=[\s>])` from root_end
    let mut at = root_end;
    let opening = loop {
        let k = find(data, at, b"<w:body")?;
        let next = k + 7;
        if next < data.len() && (space(data[next]) || data[next] == b'>') {
            break next;
        }
        at = k + 1;
    };
    let body_end = rfind(data, b"</w:body>")?;
    let body = (find_byte(data, opening, b'>')? + 1, body_end);
    let spans = blocks(data, body.0, body.1)?;
    Some((spans, body, root_end))
}

/// How many leading bytes `a` and `b` share.
pub fn common_prefix(a: &[u8], b: &[u8]) -> usize {
    let n = a.len().min(b.len());
    let mut at = 0;
    const STEP: usize = 1 << 12;
    while at + STEP <= n && a[at..at + STEP] == b[at..at + STEP] {
        at += STEP;
    }
    while at < n && a[at] == b[at] {
        at += 1;
    }
    at
}

/// How many trailing bytes `a` and `b` share, at most `limit`.
pub fn common_suffix(a: &[u8], b: &[u8], limit: usize) -> usize {
    let (la, lb) = (a.len(), b.len());
    let limit = limit.min(la).min(lb);
    let mut at = 0;
    const STEP: usize = 1 << 12;
    while at + STEP <= limit && a[la - at - STEP..la - at] == b[lb - at - STEP..lb - at] {
        at += STEP;
    }
    while at < limit && a[la - at - 1] == b[lb - at - 1] {
        at += 1;
    }
    at
}

/// `_kept_counts`: the blocks ending within the first `prefix` bytes, and how many of the
/// rest start at or after `suffix_at`.
pub fn kept_counts(spans: &[(usize, usize)], prefix: usize, suffix_at: usize) -> (usize, usize) {
    let n = spans.len();
    let before = spans.partition_point(|s| s.1 <= prefix);
    let starting = spans.partition_point(|s| s.0 < suffix_at);
    (before, n - before.max(starting))
}

fn tag_count(data: &[u8], tag: &[u8]) -> u32 {
    let mut n = 0;
    let mut at = 0;
    while let Some(k) = find(data, at, tag) {
        let next = k + tag.len();
        if next < data.len() && (space(data[next]) || data[next] == b'/' || data[next] == b'>') {
            n += 1;
        }
        at = k + 1;
    }
    n
}

/// docx-agent's `_span_counts(data, begin, end)`: the paragraphs, tables and rows in
/// `data[begin..end]`, not under an `mc:Fallback` (`<mc:Fallback(?=[\s>]).*?</mc:Fallback>`
/// taken out first).
pub fn span_counts(data: &[u8], begin: usize, end: usize) -> [u32; 3] {
    let piece = &data[begin.min(data.len())..end.min(data.len())];
    let owned;
    let piece = if find(piece, 0, b"<mc:Fallback").is_some() {
        let mut out = Vec::with_capacity(piece.len());
        let mut kept_from = 0;
        let mut at = 0;
        while let Some(k) = find(piece, at, b"<mc:Fallback") {
            let next = k + 12;
            if !(next < piece.len() && (space(piece[next]) || piece[next] == b'>')) {
                at = k + 1;
                continue;
            }
            match find(piece, next, b"</mc:Fallback>") {
                Some(close) => {
                    out.extend_from_slice(&piece[kept_from..k]);
                    kept_from = close + 14;
                    at = kept_from;
                }
                None => break,
            }
        }
        out.extend_from_slice(&piece[kept_from..]);
        owned = out;
        &owned[..]
    } else {
        piece
    };
    [
        tag_count(piece, b"<w:p"),
        tag_count(piece, b"<w:tbl"),
        tag_count(piece, b"<w:tr"),
    ]
}

/// The stretches of `data` within `body` outside the blocks at `spans`, in order.
fn gaps<'a>(data: &'a [u8], spans: &'a [(usize, usize)], body: (usize, usize)) -> impl Iterator<Item = &'a [u8]> + 'a {
    let starts = std::iter::once(body.0).chain(spans.iter().map(|s| s.1));
    let ends = spans.iter().map(|s| s.0).chain(std::iter::once(body.1));
    starts.zip(ends).map(move |(a, b)| &data[a..b])
}

/// One step from a version's blocks to the next's.
struct Step {
    /// The next version's bytes (held: the step is found by them).
    data: Py<PyBytes>,
    next: Py<BodySpans>,
    before: usize,
    after: usize,
    begin: usize,
    stop: usize,
}

/// A version of a body: its part's bytes, its top-level blocks' spans in them, the body's
/// content span and where the root's start tag ends.
#[pyclass(frozen, module = "ooxml_core")]
pub struct BodySpans {
    data: Py<PyBytes>,
    spans: Arc<Vec<(usize, usize)>>,
    body: (usize, usize),
    root_end: usize,
    /// The steps to the next versions made from this one, the latest last (two kept: an
    /// edit's version and an undo's, say).
    steps: Mutex<Vec<Step>>,
    /// Each block's paragraphs, tables and rows (`_span_counts`), once asked; a step from a
    /// version that has them reads only the new blocks'.
    counts: Mutex<Option<Arc<Vec<[u32; 3]>>>>,
}

impl BodySpans {
    fn counts_of(&self, py: Python<'_>) -> Arc<Vec<[u32; 3]>> {
        if let Some(found) = self.counts.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
            return found.clone();
        }
        // (counted with the lock let go and the mutex not held: a thread waiting for the
        // mutex may hold the interpreter lock)
        let data = self.data.bind(py).as_bytes();
        let spans = self.spans.clone();
        let found: Arc<Vec<[u32; 3]>> =
            Arc::new(py.detach(|| spans.iter().map(|&(a, b)| span_counts(data, a, b)).collect()));
        let mut held = self.counts.lock().unwrap_or_else(|e| e.into_inner());
        held.get_or_insert(found).clone()
    }
}

impl BodySpans {
    pub fn spans(&self) -> &[(usize, usize)] {
        &self.spans
    }
}

#[pymethods]
impl BodySpans {
    /// The blocks of `data` (`_scan`), or `None`.
    #[staticmethod]
    fn scan(data: Bound<'_, PyBytes>) -> Option<BodySpans> {
        let py = data.py();
        let bytes = data.as_bytes();
        let (spans, body, root_end) = py.detach(|| scan(bytes))?;
        Some(BodySpans {
            data: data.unbind(),
            spans: Arc::new(spans),
            body,
            root_end,
            steps: Mutex::new(Vec::new()),
            counts: Mutex::new(None),
        })
    }

    /// The blocks of `data`, another version of this body, from these (`_rescan`): `(next,
    /// before, after, begin, stop)` -- the blocks kept before and after, and the stretch of
    /// `data` read again -- or `None` where the change reaches outside the body or the tags
    /// there do not nest.  The step is kept: asked again for the same bytes, the same
    /// answer and the same `next`.
    #[allow(clippy::type_complexity)]
    fn advance<'py>(
        slf: &Bound<'py, Self>,
        data: Bound<'py, PyBytes>,
    ) -> PyResult<Option<(Py<BodySpans>, usize, usize, usize, usize)>> {
        let py = slf.py();
        let me = slf.get();
        {
            let steps = me.steps.lock().unwrap_or_else(|e| e.into_inner());
            for step in steps.iter() {
                if step.data.as_ptr() == data.as_ptr() {
                    return Ok(Some((
                        step.next.clone_ref(py),
                        step.before,
                        step.after,
                        step.begin,
                        step.stop,
                    )));
                }
            }
        }
        let old = me.data.bind(py).as_bytes();
        let new = data.as_bytes();
        let spans = me.spans.clone();
        let old_counts = me.counts.lock().unwrap_or_else(|e| e.into_inner()).clone();
        let (body_at, body_end) = me.body;
        let root_end = me.root_end;
        #[allow(clippy::type_complexity)]
        let found = py.detach(|| -> Option<(Vec<(usize, usize)>, usize, usize, usize, usize, usize, Option<Vec<[u32; 3]>>)> {
            let prefix = common_prefix(old, new);
            let suffix = common_suffix(old, new, old.len().min(new.len()) - prefix);
            if prefix < body_at || old.len() - suffix > body_end {
                return None;
            }
            let delta = new.len() as i64 - old.len() as i64;
            let shift = |at: usize| (at as i64 + delta) as usize;
            let n = spans.len();
            let (before, after) = kept_counts(&spans, prefix, old.len() - suffix);
            let begin = if before > 0 { spans[before - 1].1 } else { body_at };
            let stop = if after > 0 { shift(spans[n - after].0) } else { shift(body_end) };
            let middle = blocks(new, begin, stop)?;
            let mut out = Vec::with_capacity(before + middle.len() + after);
            out.extend_from_slice(&spans[..before]);
            out.extend_from_slice(&middle);
            out.extend(spans[n - after..].iter().map(|&(s, e)| (shift(s), shift(e))));
            let counts = old_counts.map(|old| {
                let mut c = Vec::with_capacity(out.len());
                c.extend_from_slice(&old[..before]);
                c.extend(middle.iter().map(|&(a, b)| span_counts(new, a, b)));
                c.extend_from_slice(&old[n - after..]);
                c
            });
            Some((out, before, after, begin, stop, shift(body_end), counts))
        });
        let (out, before, after, begin, stop, new_end, counts) = match found {
            Some(found) => found,
            None => return Ok(None),
        };
        // (the root's start tag is before the body: the same bytes, the same end)
        let next = Py::new(
            py,
            BodySpans {
                data: data.clone().unbind(),
                spans: Arc::new(out),
                body: (body_at, new_end),
                root_end,
                steps: Mutex::new(Vec::new()),
                counts: Mutex::new(counts.map(Arc::new)),
            },
        )?;
        let mut steps = me.steps.lock().unwrap_or_else(|e| e.into_inner());
        steps.push(Step {
            data: data.unbind(),
            next: next.clone_ref(py),
            before,
            after,
            begin,
            stop,
        });
        let excess = steps.len().saturating_sub(2);
        steps.drain(..excess);
        Ok(Some((next, before, after, begin, stop)))
    }

    /// The paragraphs, tables and rows of each block `a..b` (`_span_counts`): a list of
    /// `(p, t, r)`.
    fn counts(&self, py: Python<'_>, a: usize, b: usize) -> Vec<(u32, u32, u32)> {
        let all = self.counts_of(py);
        let b = b.min(all.len());
        all[a.min(b)..b].iter().map(|c| (c[0], c[1], c[2])).collect()
    }

    /// Their sums over the blocks `a..b`: `(p, t, r)`.
    fn count_sum(&self, py: Python<'_>, a: usize, b: usize) -> (u64, u64, u64) {
        let all = self.counts_of(py);
        let b = b.min(all.len());
        let mut sum = (0u64, 0u64, 0u64);
        for c in &all[a.min(b)..b] {
            sum.0 += c[0] as u64;
            sum.1 += c[1] as u64;
            sum.2 += c[2] as u64;
        }
        sum
    }

    /// `_span_counts(data, begin, end)` of this version's bytes.
    fn stretch_counts(&self, py: Python<'_>, begin: usize, end: usize) -> (u32, u32, u32) {
        let data = self.data.bind(py).as_bytes();
        let c = py.detach(|| span_counts(data, begin, end));
        (c[0], c[1], c[2])
    }

    /// Whether the bytes of the body outside its blocks (before the first, between them,
    /// after the last), taken as one stream, are `other`'s: what docx-agent's touched
    /// sections compare as that stream's digest (`tools/touched._rest`), compared here
    /// whole, the lock let go.
    fn same_gaps(&self, py: Python<'_>, other: &BodySpans) -> bool {
        let (a, b) = (self.data.bind(py).as_bytes(), other.data.bind(py).as_bytes());
        let (sa, sb) = (self.spans.clone(), other.spans.clone());
        let (ba, bb) = (self.body, other.body);
        py.detach(|| gaps(a, &sa, ba).flatten().eq(gaps(b, &sb, bb).flatten()))
    }

    /// `_kept_counts(spans, prefix, suffix_at)`.
    fn kept_counts(&self, prefix: usize, suffix_at: usize) -> (usize, usize) {
        kept_counts(&self.spans, prefix, suffix_at)
    }

    #[getter]
    fn data(&self, py: Python<'_>) -> Py<PyBytes> {
        self.data.clone_ref(py)
    }

    /// The body's content: `(after its start tag, at its end tag)`.
    #[getter]
    fn body(&self) -> (usize, usize) {
        self.body
    }

    #[getter]
    fn root_end(&self) -> usize {
        self.root_end
    }

    fn __len__(&self) -> usize {
        self.spans.len()
    }

    /// A span (`(start, end)`), or a list of them for a slice.
    fn __getitem__<'py>(&self, py: Python<'py>, key: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
        let n = self.spans.len();
        if let Ok(slice) = key.cast::<PySlice>() {
            let ix = slice.indices(n as isize)?;
            let mut out = Vec::with_capacity(ix.slicelength);
            let mut at = ix.start;
            for _ in 0..ix.slicelength {
                out.push(PyTuple::new(
                    py,
                    [self.spans[at as usize].0, self.spans[at as usize].1],
                )?);
                at += ix.step;
            }
            return Ok(PyList::new(py, out)?.into_any());
        }
        let mut k: isize = key.extract()?;
        if k < 0 {
            k += n as isize;
        }
        if k < 0 || k as usize >= n {
            return Err(pyo3::exceptions::PyIndexError::new_err("span index out of range"));
        }
        Ok(PyTuple::new(py, [self.spans[k as usize].0, self.spans[k as usize].1])?.into_any())
    }
}

/// `_blocks(data, begin, stop)` for Python (the oracle's tests), the lock let go.
#[pyfunction]
pub fn body_blocks(py: Python<'_>, data: &[u8], begin: usize, stop: usize) -> Option<Vec<(usize, usize)>> {
    py.detach(|| blocks(data, begin, stop))
}

/// How many leading bytes, and then how many trailing bytes of the rest, `a` and `b`
/// share: `(prefix, suffix)`, the lock let go.
#[pyfunction]
pub fn common_ends(py: Python<'_>, a: &[u8], b: &[u8]) -> (usize, usize) {
    py.detach(|| {
        let prefix = common_prefix(a, b);
        (prefix, common_suffix(a, b, a.len().min(b.len()) - prefix))
    })
}

/// `_common_prefix(a, b)`, the lock let go.
#[pyfunction]
pub fn common_prefix_of(py: Python<'_>, a: &[u8], b: &[u8]) -> usize {
    py.detach(|| common_prefix(a, b))
}

/// `_common_suffix(a, b, limit)`, the lock let go.
#[pyfunction]
pub fn common_suffix_of(py: Python<'_>, a: &[u8], b: &[u8], limit: usize) -> usize {
    py.detach(|| common_suffix(a, b, limit))
}

/// Lane K: where each child of a part's body begins in its bytes (a written part's, as
/// `unit_starts` reads them), held in Rust: the next write's are made from them without a
/// Python object per child.
#[pyclass(frozen, module = "ooxml_core")]
pub struct Starts {
    at: Arc<Vec<usize>>,
}

#[pymethods]
impl Starts {
    #[new]
    fn new(at: Vec<usize>) -> Self {
        Starts { at: Arc::new(at) }
    }

    fn __len__(&self) -> usize {
        self.at.len()
    }

    fn __getitem__(&self, k: isize) -> PyResult<usize> {
        let n = self.at.len() as isize;
        let k = if k < 0 { k + n } else { k };
        if k < 0 || k >= n {
            return Err(pyo3::exceptions::PyIndexError::new_err("start index out of range"));
        }
        Ok(self.at[k as usize])
    }

    fn list(&self) -> Vec<usize> {
        self.at.to_vec()
    }
}

/// The children a write must make again: for `new` units (16-byte digests, joined) after
/// `old`'s, `(head, tail, missing)` -- the children alike at the start and the end, and
/// the places (in `new`) of the children between whose digests are none of `old`'s.
#[pyfunction]
pub fn write_plan(py: Python<'_>, old: &[u8], new: &[u8]) -> (usize, usize, Vec<usize>) {
    py.detach(|| {
        let plan = plan(old, new);
        (
            plan.head,
            plan.tail,
            plan.region
                .iter()
                .enumerate()
                .filter(|(_, j)| j.is_none())
                .map(|(i, _)| plan.head + i)
                .collect(),
        )
    })
}

struct Plan {
    head: usize,
    tail: usize,
    /// Each child between: the old child it is, or `None`.
    region: Vec<Option<usize>>,
}

fn plan(old: &[u8], new: &[u8]) -> Plan {
    let (n, m) = (old.len() / 16, new.len() / 16);
    let least = n.min(m);
    let head = (common_prefix(old, new) / 16).min(least);
    let tail = common_suffix(old, new, 16 * (least - head)) / 16;
    let count = m - tail - head;
    let mut region = Vec::with_capacity(count);
    // (beyond a few children, the old ones looked up by a table of their digests)
    let table: Option<std::collections::HashMap<&[u8], usize>> = if count > 32 {
        let mut t = std::collections::HashMap::with_capacity(n);
        for j in 0..n {
            t.entry(&old[16 * j..16 * j + 16]).or_insert(j);
        }
        Some(t)
    } else {
        None
    };
    for k in head..m - tail {
        let digest = &new[16 * k..16 * k + 16];
        let j = match &table {
            Some(t) => t.get(digest).copied(),
            None => (0..n).find(|&j| &old[16 * j..16 * j + 16] == digest),
        };
        region.push(j);
    }
    Plan { head, tail, region }
}

/// The next write of a part: `old_data`'s body children (where `old_starts` says each
/// begins, its content ending at `old_end`) kept where `new` (the children's digests now)
/// has them, `pieces` (the children made again, in order: those `write_plan` names) put in
/// between: `(data, starts, end)`.  `None` where the pieces are not as many as needed.
#[pyfunction]
#[allow(clippy::too_many_arguments)]
pub fn write_assemble<'py>(
    py: Python<'py>,
    old_data: &Bound<'py, PyBytes>,
    old_starts: &Starts,
    old_end: usize,
    old: &[u8],
    new: &[u8],
    pieces: Vec<Bound<'py, PyBytes>>,
) -> PyResult<Option<(Bound<'py, PyBytes>, Starts, usize)>> {
    let data = old_data.as_bytes();
    let starts = old_starts.at.clone();
    let n = starts.len();
    if n != old.len() / 16 {
        return Ok(None);
    }
    let pieces: Vec<&[u8]> = pieces.iter().map(|p| p.as_bytes()).collect();
    #[allow(clippy::type_complexity)]
    let found = py.detach(|| -> Option<(Vec<(usize, usize, bool)>, Vec<usize>, usize, usize)> {
        let plan = plan(old, new);
        let span = |k: usize| (starts[k], if k + 1 < n { starts[k + 1] } else { old_end });
        let first = if n > 0 { starts[0] } else { old_end };
        // segments: (start, end, from a piece) in order; a piece's start is its index
        let mut segments: Vec<(usize, usize, bool)> = vec![(0, first, false)];
        let mut new_starts: Vec<usize> = Vec::with_capacity(new.len() / 16);
        new_starts.extend_from_slice(&starts[..plan.head]);
        let mut at = first;
        if plan.head > 0 {
            let end = span(plan.head - 1).1;
            segments.push((first, end, false));
            at = end;
        }
        let mut next_piece = 0;
        for j in &plan.region {
            new_starts.push(at);
            match j {
                Some(j) => {
                    let (a, b) = span(*j);
                    segments.push((a, b, false));
                    at += b - a;
                }
                None => {
                    let piece = pieces.get(next_piece)?;
                    segments.push((next_piece, next_piece + 1, true));
                    next_piece += 1;
                    at += piece.len();
                }
            }
        }
        if next_piece != pieces.len() {
            return None;
        }
        if plan.tail > 0 {
            let a = span(n - plan.tail).0;
            new_starts.extend(starts[n - plan.tail..].iter().map(|&s| s + at - a));
            segments.push((a, old_end, false));
            at += old_end - a;
        }
        let end = at;
        segments.push((old_end, data.len(), false));
        Some((segments, new_starts, end, at + data.len() - old_end))
    });
    let (segments, new_starts, end, total) = match found {
        Some(found) => found,
        None => return Ok(None),
    };
    // (the new bytes filled with the lock let go: no one else sees them yet)
    let out = PyBytes::new_with(py, total, |buf| {
        py.detach(|| {
            let mut at = 0;
            for &(a, b, piece) in &segments {
                let src: &[u8] = if piece { pieces[a] } else { &data[a..b] };
                buf[at..at + src.len()].copy_from_slice(src);
                at += src.len();
            }
        });
        Ok(())
    })?;
    Ok(Some((
        out,
        Starts {
            at: Arc::new(new_starts),
        },
        end,
    )))
}
