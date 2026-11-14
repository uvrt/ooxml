//! A PyO3 binding over one snapshot: Python threads ask it at once, the interpreter lock
//! released for the Rust work.
use ooxml_word::measure::Tables;
use ooxml_word::model::load;
use ooxml_word::paginate::{paginate_all, Doc};
use ooxml_word::snapshot::Snapshot;
use pyo3::prelude::*;
use std::sync::Arc;

#[pyclass(frozen)]
struct Snap {
    inner: Arc<Snapshot>,
}

#[pymethods]
impl Snap {
    /// Load an export, measure it with the Rust kernel, paginate it (lock released).
    #[staticmethod]
    fn load(py: Python<'_>, path: &str, copies: usize) -> PyResult<Snap> {
        let snap = py.allow_threads(|| {
            let e = load(path);
            let t = Tables::new(&e);
            let doc = Doc::new(&e);
            let n = e.items.len();
            let all: Vec<usize> = (0..n * copies).map(|k| k % n).collect();
            let mut flow = ooxml_word::measured_flow(&e, &t, &all, true);
            let span = flow.iter().take(n).map(|i| i.block()).max().unwrap_or(0) + 1000;
            for (k, it) in flow.iter_mut().enumerate() {
                let add = span * (k / n) as u32;
                match std::sync::Arc::make_mut(it) {
                    ooxml_word::paginate::FItem::P(p) => p.block += add,
                    ooxml_word::paginate::FItem::T(t) => t.block += add,
                }
            }
            let pag = paginate_all(&doc, &flow);
            Snapshot::new(doc, flow, pag)
        });
        Ok(Snap { inner: Arc::new(snap) })
    }
    fn pages(&self) -> usize {
        self.inner.pages()
    }
    /// The page of a block, lock held (the call is shorter than releasing it).
    fn page_of_block(&self, block: u32) -> Option<u32> {
        self.inner.page_of_block(block)
    }
    /// The same, lock released.
    fn page_of_block_released(&self, py: Python<'_>, block: u32) -> Option<u32> {
        let s = self.inner.clone();
        py.allow_threads(move || s.page_of_block(block))
    }
    /// Facts over pages `[from, to)`, placed lazily, lock released.
    fn facts(&self, py: Python<'_>, from: usize, to: usize) -> (u64, u64, u64, u64) {
        let s = self.inner.clone();
        let f = py.allow_threads(move || s.facts(from, to));
        (f.lines, f.past_bottom, f.keep_next_broken, f.keep_lines_broken)
    }
    /// The same, lock held.
    fn facts_held(&self, from: usize, to: usize) -> (u64, u64, u64, u64) {
        let f = self.inner.facts(from, to);
        (f.lines, f.past_bottom, f.keep_next_broken, f.keep_lines_broken)
    }
}

/// Lane E: a document's layout owner -- its own thread, commits coalesced, snapshots shared.
#[pyclass(frozen)]
struct Owner {
    inner: ooxml_word::engine::Owner,
}

/// One snapshot, held to compare a later one with (`since_last_check`).
#[pyclass(frozen)]
struct SnapshotE {
    inner: Arc<ooxml_word::engine::Snap>,
}

#[pymethods]
impl SnapshotE {
    #[getter]
    fn version(&self) -> u64 {
        self.inner.version
    }
    #[getter]
    fn pages(&self) -> usize {
        self.inner.page_count()
    }
    /// Each laid-out page's start `(item, line, part)` (tests).
    fn starts(&self) -> Vec<(u32, u32, Vec<u32>)> {
        self.inner.pag.starts.iter().map(|p| (p.item, p.line, p.part.clone())).collect()
    }
    /// Each page's `(number, section, section_first, story, blank)` (tests).
    fn info(&self) -> Vec<(i64, u32, bool, u8, bool)> {
        self.inner.pag.info.iter().map(|i| (i.number, i.section, i.section_first, i.story, i.blank)).collect()
    }
    /// Every computed field's text now: `(item, paragraph, run, keyword, text)` (tests).
    fn field_texts(&self) -> Vec<(usize, usize, usize, String, String)> {
        let mut out = Vec::new();
        for (i, a) in self.inner.aux.iter().enumerate() {
            for (pi, p) in a.paras.iter().enumerate() {
                for (run, kw, _, _, text) in &p.fields {
                    out.push((i, pi, *run, kw.clone(), text.clone()));
                }
            }
        }
        out
    }
    /// Every item's lines' heights `[pitch...]` (tests).
    fn pitches(&self) -> Vec<Vec<i64>> {
        self.inner.flow.iter().map(|it| match &**it {
            ooxml_word::paginate::FItem::P(p) => p.lines.iter().map(|l| l.pitch).collect(),
            ooxml_word::paginate::FItem::T(t) => t.rows.iter().map(|r| r.height).collect(),
        }).collect()
    }
}

#[pymethods]
impl Owner {
    /// The owner of a document from its export (JSON), laid out whole (lock released).
    #[staticmethod]
    fn open(py: Python<'_>, export: String) -> PyResult<(Owner, String)> {
        let (o, st) = py.allow_threads(move || {
            let t = std::time::Instant::now();
            let e: ooxml_word::model::Export = serde_json::from_str(&export).expect("export");
            let parse = t.elapsed().as_secs_f64() * 1000.0;
            let o = ooxml_word::engine::Owner::open(e);
            let mut st = ooxml_word::engine::stat_json(&o.first, 0.0);
            st["parse_ms"] = serde_json::json!(parse);
            (o, st.to_string())
        });
        Ok((Owner { inner: o }, st))
    }
    /// Hand a commit (JSON) to the owner's thread; returns at once.
    fn commit(&self, commit: String) {
        self.inner.commit(commit);
    }
    /// Wait (lock released) for the first snapshot of `version` or later, then answer the
    /// query (JSON) on it: one call per check.  Returns `(answer JSON, snapshot)`.
    fn check(&self, py: Python<'_>, version: u64, query: String) -> PyResult<(String, SnapshotE, f64, f64)> {
        let (s, snap, waited, read) = py.allow_threads(|| {
            let t = std::time::Instant::now();
            let snap = self.inner.wait(version);
            let waited = t.elapsed().as_secs_f64() * 1000.0;
            let t2 = std::time::Instant::now();
            let q: ooxml_word::engine::Query = serde_json::from_str(&query).expect("query");
            let out = snap.query(&q).to_string();
            (out, snap, waited, t2.elapsed().as_secs_f64() * 1000.0)
        });
        Ok((s, SnapshotE { inner: snap }, waited, read))
    }
    /// The pages each key is drawn on, on the first snapshot of `version` or later (lock
    /// released): `([[page...]...], snapshot version)`.
    fn pages_of(&self, py: Python<'_>, version: u64, keys: Vec<String>) -> (Vec<Vec<usize>>, u64) {
        py.allow_threads(|| {
            let snap = self.inner.wait(version);
            (snap.pages_of(&keys), snap.version)
        })
    }
    /// The paragraphs whose first page moved from `before` to `now` (lock released).
    #[staticmethod]
    fn moved(py: Python<'_>, before: &SnapshotE, now: &SnapshotE) -> Vec<(String, usize, usize)> {
        let (a, b) = (before.inner.clone(), now.inner.clone());
        py.allow_threads(move || b.moved(&a))
    }
    /// The relayouts so far (JSON list), and clears them.
    fn stats(&self) -> String {
        let mut g = self.inner.core.shared.passes.lock().unwrap();
        let out: Vec<serde_json::Value> = g.iter().map(ooxml_word::engine::pass_json).collect();
        g.clear();
        serde_json::Value::Array(out).to_string()
    }
    fn latest(&self) -> u64 {
        self.inner.core.latest().version
    }
    fn stop(&self, py: Python<'_>) {
        py.allow_threads(|| self.inner.stop());
    }
}

// -- one module for every format: documents by id ------------------------------------------
//
// Lane E: the binding's shape for several formats in one process.  A process-wide registry
// holds each open document's owner by its id (the toolbox's `d1`, `d2`...), whatever its
// format; Python hands over ids, versions and JSON (change sets, requests) and gets JSON
// back -- one call per tool call, the interpreter lock let go for all of it.  A format plugs
// in by implementing `AnyOwner`; a call across formats (a workbook's range filled into a
// document's table) would be one function taking both ids and staying inside Rust.

trait AnyOwner: Send + Sync {
    fn kind(&self) -> &'static str;
    fn commit(&self, text: String);
    /// Wait for the first snapshot holding `version`, then answer `request` on it.
    fn call(&self, version: u64, request: &str) -> String;
    fn stop(&self);
}

impl AnyOwner for ooxml_word::engine::Owner {
    fn kind(&self) -> &'static str {
        "docx"
    }
    fn commit(&self, text: String) {
        ooxml_word::engine::Owner::commit(self, text)
    }
    fn call(&self, version: u64, request: &str) -> String {
        let snap = self.wait(version);
        let q: ooxml_word::engine::Query = serde_json::from_str(request).expect("request");
        snap.query(&q).to_string()
    }
    fn stop(&self) {
        ooxml_word::engine::Owner::stop(self)
    }
}

fn registry() -> &'static std::sync::RwLock<std::collections::HashMap<String, Arc<dyn AnyOwner>>> {
    static R: std::sync::OnceLock<std::sync::RwLock<std::collections::HashMap<String, Arc<dyn AnyOwner>>>> =
        std::sync::OnceLock::new();
    R.get_or_init(|| std::sync::RwLock::new(std::collections::HashMap::new()))
}

fn owner_of(doc: &str) -> PyResult<Arc<dyn AnyOwner>> {
    registry()
        .read()
        .unwrap()
        .get(doc)
        .cloned()
        .ok_or_else(|| pyo3::exceptions::PyKeyError::new_err(doc.to_string()))
}

/// Open a document of `kind` under `doc` from its first input (a Word export).
#[pyfunction]
fn open_document(py: Python<'_>, doc: String, kind: &str, first: String) -> PyResult<String> {
    if kind != "docx" {
        return Err(pyo3::exceptions::PyValueError::new_err(format!("no {kind} format in this prototype")));
    }
    let (owner, stat) = py.allow_threads(move || {
        let e: ooxml_word::model::Export = serde_json::from_str(&first).expect("export");
        let o = ooxml_word::engine::Owner::open(e);
        let st = ooxml_word::engine::stat_json(&o.first, 0.0).to_string();
        (o, st)
    });
    registry().write().unwrap().insert(doc, Arc::new(owner));
    Ok(stat)
}

/// A change set for `doc` (returns at once).
#[pyfunction]
fn commit(doc: &str, text: String) -> PyResult<()> {
    owner_of(doc)?.commit(text);
    Ok(())
}

/// One batched request on `doc` at `version` or later (the lock let go throughout).
#[pyfunction]
fn call(py: Python<'_>, doc: &str, version: u64, request: String) -> PyResult<String> {
    let owner = owner_of(doc)?;
    Ok(py.allow_threads(move || owner.call(version, &request)))
}

#[pyfunction]
fn close_document(py: Python<'_>, doc: &str) -> PyResult<()> {
    let owner = registry().write().unwrap().remove(doc);
    if let Some(o) = owner {
        py.allow_threads(move || o.stop());
    }
    Ok(())
}

#[pyfunction]
fn documents() -> Vec<(String, &'static str)> {
    registry().read().unwrap().iter().map(|(k, v)| (k.clone(), v.kind())).collect()
}

#[pyfunction]
fn noop() {}

fn busy(micros: u64) -> u64 {
    let t = std::time::Instant::now();
    let mut x = 0u64;
    while t.elapsed().as_micros() < micros as u128 {
        x = x.wrapping_add(1);
    }
    x
}

/// Busy for `micros`, lock released.
#[pyfunction]
fn spin(py: Python<'_>, micros: u64) -> u64 {
    py.allow_threads(move || busy(micros))
}

/// Busy for `micros`, lock held.
#[pyfunction]
fn spin_held(micros: u64) -> u64 {
    busy(micros)
}

#[pymodule]
fn ooxml_core(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<Snap>()?;
    m.add_class::<Owner>()?;
    m.add_class::<SnapshotE>()?;
    m.add_function(wrap_pyfunction!(noop, m)?)?;
    m.add_function(wrap_pyfunction!(open_document, m)?)?;
    m.add_function(wrap_pyfunction!(commit, m)?)?;
    m.add_function(wrap_pyfunction!(call, m)?)?;
    m.add_function(wrap_pyfunction!(close_document, m)?)?;
    m.add_function(wrap_pyfunction!(documents, m)?)?;
    m.add_function(wrap_pyfunction!(spin, m)?)?;
    m.add_function(wrap_pyfunction!(spin_held, m)?)?;
    Ok(())
}
