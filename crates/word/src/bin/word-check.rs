//! The differential checks against docx2svg (moved in from the prototype's `spike` driver).
//!
//!     word-check check  EXPORT.json                 measure + paginate vs docx2svg, timed
//!     word-check edits  BASE.json EDITED.json...     incremental re-measure + re-paginate vs docx2svg
//!     word-check scale  BASE.json COPIES EDITED.json 3,000-page flow from the base repeated, timed
//!
//! Prints one JSON object.

use ooxml_word::incr::{diff, repaginate, splice};
use ooxml_word::measure::{measure, Tables};
use ooxml_word::model::{load, Export, ItemIn, RefIn};
use ooxml_word::paginate::{flow_from_export, from_export_pages, item_from_export, paginate_all, Doc, FItem, Pagination, Pos};
use ooxml_word::snapshot::Snapshot;
use ooxml_word::{measure_item, measured_flow};
use rayon::prelude::*;
use serde_json::{json, Value};
use std::time::Instant;

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.0
}

fn best<T>(n: usize, mut f: impl FnMut() -> T) -> (f64, T) {
    let mut out = None;
    let mut b = f64::INFINITY;
    for _ in 0..n {
        let t = Instant::now();
        let v = f();
        b = b.min(ms(t));
        out = Some(v);
    }
    (b, out.unwrap())
}

fn cpu_ms() -> f64 {
    let mut u: libc_rusage = unsafe { std::mem::zeroed() };
    unsafe { getrusage(0, &mut u) };
    (u.ru_utime.0 as f64 + u.ru_stime.0 as f64) * 1000.0 + (u.ru_utime.1 as f64 + u.ru_stime.1 as f64) / 1000.0
}

fn max_rss_mb() -> f64 {
    let mut u: libc_rusage = unsafe { std::mem::zeroed() };
    unsafe { getrusage(0, &mut u) };
    u.ru_maxrss as f64 / (1024.0 * 1024.0) // bytes on macOS
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Tv(i64, i32);
#[repr(C)]
#[allow(non_camel_case_types)]
struct libc_rusage {
    ru_utime: Tv,
    ru_stime: Tv,
    ru_maxrss: i64,
    rest: [i64; 13],
}
extern "C" {
    fn getrusage(who: i32, usage: *mut libc_rusage) -> i32;
}

struct Compare {
    paragraphs: usize,
    pieces_ok: usize,
    lines_ok: usize,
    heights_ok: usize,
    examples: Vec<Value>,
}

fn compare_para(t: &Tables, c: &mut Compare, block: usize, m: &ooxml_word::model::MeasureIn, r: &RefIn, heights: &[ooxml_word::model::HeightIn]) {
    c.paragraphs += 1;
    let got = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| measure(t, m))) {
        Ok(g) => g,
        Err(err) => {
            let msg = err.downcast_ref::<String>().cloned().or_else(|| err.downcast_ref::<&str>().map(|s| s.to_string())).unwrap_or_default();
            if c.examples.len() < 12 {
                c.examples.push(json!({"block": block, "what": "rust stopped", "why": msg}));
            }
            return;
        }
    };
    let widths_ok = got.pieces.len() == r.widths.len()
        && got.pieces.iter().zip(&r.widths).zip(&r.kinds).all(|((p, w), k)| p.width == t.w(*w) && p.kind == *k);
    if widths_ok {
        c.pieces_ok += 1;
    } else if c.examples.len() < 12 {
        let k = got.pieces.iter().zip(&r.widths).position(|(p, w)| p.width != t.w(*w)).unwrap_or(0);
        c.examples.push(json!({"block": block, "what": "pieces", "n": [got.pieces.len(), r.widths.len()], "first": k}));
    }
    let lines: Vec<(usize, usize, u8)> = got.lines.iter().map(|l| (l.start, l.end, l.flags())).collect();
    if lines == r.lines {
        c.lines_ok += 1;
    } else if c.examples.len() < 12 {
        c.examples.push(json!({"block": block, "what": "lines", "rust": lines, "docx2svg": r.lines}));
    }
    let hs: Vec<(i64, i64, i64, u8, i64, i64)> = got.heights.iter().map(|h| (h.pitch, h.above, h.below, h.rule, h.line, h.natural)).collect();
    if hs == heights {
        c.heights_ok += 1;
    } else if c.examples.len() < 12 {
        c.examples.push(json!({"block": block, "what": "heights", "rust": hs.iter().take(3).collect::<Vec<_>>(), "docx2svg": heights.iter().take(3).collect::<Vec<_>>()}));
    }
}

fn all_paragraphs(e: &Export) -> Vec<(usize, &ooxml_word::model::MeasureIn)> {
    let mut out = Vec::new();
    for it in &e.items {
        match it {
            ItemIn::Para(p) => out.push((p.block, &p.measure)),
            ItemIn::Table(tb) => {
                for r in &tb.rows {
                    for c in &r.cells {
                        for p in &c.paragraphs {
                            out.push((p.block, &p.measure));
                        }
                    }
                }
            }
        }
    }
    out
}

fn compare_pages(got: &Pagination, want: &[Pos], info: &[(i64, i64, bool, String, bool)]) -> Value {
    let same = got.starts.iter().zip(want).filter(|(a, b)| a == b).count();
    let first_diff = got.starts.iter().zip(want).position(|(a, b)| a != b);
    let kind = |s: &str| match s {
        "first" => 1u8,
        "even" => 2,
        _ => 0,
    };
    let info_ok = got.info.len() == info.len()
        && got.info.iter().zip(info).all(|(a, b)| a.number == b.0 && a.section as i64 == b.1 && a.section_first == b.2 && a.story == kind(&b.3) && a.blank == b.4);
    json!({
        "pages_rust": got.starts.len(), "pages_docx2svg": want.len(), "starts_equal": same,
        "all_equal": same == want.len() && got.starts.len() == want.len(), "info_equal": info_ok,
        "first_diff": first_diff.map(|k| json!({"page": k, "rust": format!("{:?}", got.starts[k]), "docx2svg": format!("{:?}", want[k])})),
    })
}

fn check(path: &str) -> Value {
    let t0 = Instant::now();
    let e = load(path);
    let load_ms = ms(t0);
    let t = Tables::new(&e);
    std::panic::set_hook(Box::new(|_| {}));
    let mut c = Compare { paragraphs: 0, pieces_ok: 0, lines_ok: 0, heights_ok: 0, examples: vec![] };
    for it in &e.items {
        match it {
            ItemIn::Para(p) => compare_para(&t, &mut c, p.block, &p.measure, &p.reference, &p.heights),
            ItemIn::Table(tb) => {
                for r in &tb.rows {
                    for cell in &r.cells {
                        for p in &cell.paragraphs {
                            compare_para(&t, &mut c, p.block, &p.measure, &p.reference, &p.heights);
                        }
                    }
                }
            }
        }
    }
    let paras: Vec<_> = all_paragraphs(&e)
        .into_iter()
        .filter(|(_, m)| std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| measure(&t, m))).is_ok())
        .collect();
    let pieces: usize = paras.iter().map(|(_, m)| ooxml_word::measure::pieces(&t, m).len()).sum();
    let (single, _) = best(3, || paras.iter().map(|(_, m)| measure(&t, m).lines.len()).sum::<usize>());
    let (parallel, _) = best(3, || paras.par_iter().map(|(_, m)| measure(&t, m).lines.len()).sum::<usize>());
    let breaks_only = {
        let prepared: Vec<_> = paras.iter().map(|(_, m)| (ooxml_word::measure::pieces(&t, m), ooxml_word::measure::Geometry::new(&t, &m.geometry), m.columns)).collect();
        best(3, || prepared.iter().map(|(p, g, col)| ooxml_word::measure::break_pieces(t.s, p, g, *col).len()).sum::<usize>()).0
    };
    if e.pages.is_empty() {
        return json!({"export": path, "items": e.items.len(), "measure": {"paragraphs": c.paragraphs, "pieces": pieces,
            "pieces_equal": c.pieces_ok, "lines_equal": c.lines_ok, "heights_equal": c.heights_ok, "examples": c.examples,
            "all_ms_single": single, "all_ms_parallel": parallel}});
    }
    // pagination from docx2svg's own heights, and from the Rust measure's
    let want = from_export_pages(&e);
    let doc = Doc::new(&e);
    let flow = flow_from_export(&e);
    let cpu0 = cpu_ms();
    let (pag_ms, pag) = best(5, || paginate_all(&doc, &flow));
    let pag_cpu = (cpu_ms() - cpu0) / 5.0;
    let all: Vec<usize> = (0..e.items.len()).collect();
    let mflow = measured_flow(&e, &t, &all, true);
    let mpag = paginate_all(&doc, &mflow);
    // rows: Rust stack and row heights vs docx2svg's
    let mut rows_ok = 0;
    let mut rows = 0;
    for (a, b) in mflow.iter().zip(&flow) {
        if let (FItem::T(x), FItem::T(y)) = (&**a, &**b) {
            for (r1, r2) in x.rows.iter().zip(&y.rows) {
                rows += 1;
                if r1.height == r2.height && r1.cells.iter().zip(&r2.cells).all(|(c1, c2)| c1.after == c2.after && c1.stack.iter().map(|s| s.h).eq(c2.stack.iter().map(|s| s.h))) {
                    rows_ok += 1;
                }
            }
        }
    }
    json!({
        "export": path, "load_ms": load_ms, "items": e.items.len(),
        "measure": {"paragraphs": c.paragraphs, "pieces": pieces, "pieces_equal": c.pieces_ok, "lines_equal": c.lines_ok,
                    "heights_equal": c.heights_ok, "examples": c.examples,
                    "rows": rows, "rows_equal": rows_ok,
                    "all_ms_single": single, "all_ms_parallel": parallel, "break_only_ms_single": breaks_only,
                    "us_per_paragraph_single": single * 1000.0 / paras.len() as f64,
                    "threads": rayon::current_num_threads()},
        "paginate_docx2svg_heights": compare_pages(&pag, &want, &e.info),
        "paginate_rust_heights": compare_pages(&mpag, &want, &e.info),
        "paginate_ms_best_of_5": pag_ms, "paginate_cpu_ms_mean": pag_cpu,
    })
}

fn digests(e: &Export) -> Vec<Option<String>> {
    e.items
        .iter()
        .map(|it| match it {
            ItemIn::Para(p) => p.digest.clone(),
            ItemIn::Table(t) => t.digest.clone(),
        })
        .collect()
}

fn edits(base_path: &str, edited: &[String]) -> Value {
    let base = load(base_path);
    let t = Tables::new(&base);
    let doc = Doc::new(&base);
    let all: Vec<usize> = (0..base.items.len()).collect();
    let flow = measured_flow(&base, &t, &all, true);
    let pag = paginate_all(&doc, &flow);
    let base_ok = pag.starts == from_export_pages(&base);
    let mut rows = Vec::new();
    for path in edited {
        let e = load(path);
        // the change set: what the edit side would send
        let ch = diff(&digests(&base), &digests(&e));
        let te = Tables::new(&e);
        assert_eq!(te.s, t.s);
        let changed: Vec<&ItemIn> = e.items[ch.a..ch.b_new].iter().collect();
        let mut stats = None;
        let mut new_pag = Pagination::default();
        let mut measure_ms = f64::INFINITY;
        let mut paginate_ms = f64::INFINITY;
        let mut splice_ms = f64::INFINITY;
        for _ in 0..5 {
            let t0 = Instant::now();
            let items: Vec<FItem> = changed.iter().map(|it| item_from_export(&te.px, it, Some(&measure_item(&te, it)))).collect();
            measure_ms = measure_ms.min(ms(t0));
            let t1 = Instant::now();
            let new_flow = splice(&flow, ch, items);
            splice_ms = splice_ms.min(ms(t1));
            let t2 = Instant::now();
            let (p, s) = repaginate(&doc, &pag, &new_flow, ch);
            paginate_ms = paginate_ms.min(ms(t2));
            new_pag = p;
            stats = Some(s);
        }
        let want = from_export_pages(&e);
        // the kernel alone on the edited document (docx2svg's heights), full
        let full = paginate_all(&Doc::new(&e), &flow_from_export(&e));
        let changed_heights_ok = changed.iter().all(|it| {
            let hs = measure_item(&te, it);
            match it {
                ItemIn::Para(p) => hs[0].iter().map(|h| (h.pitch, h.above, h.below, h.rule, h.line, h.natural)).eq(p.heights.iter().copied()),
                ItemIn::Table(tb) => {
                    let refs: Vec<_> = tb.rows.iter().flat_map(|r| r.cells.iter()).flat_map(|c| c.paragraphs.iter()).collect();
                    refs.iter().zip(&hs).all(|(p, h)| h.iter().map(|h| (h.pitch, h.above, h.below, h.rule, h.line, h.natural)).eq(p.heights.iter().copied()))
                }
            }
        });
        let s = stats.unwrap();
        rows.push(json!({
            "edit": path, "change": [ch.a, ch.b_old, ch.b_new], "changed_items": changed.len(),
            "changed_heights_equal": changed_heights_ok,
            "measure_ms": measure_ms, "splice_ms": splice_ms, "repaginate_ms": paginate_ms,
            "restart_page": s.restart_page, "pages_laid": s.pages_laid, "joined_old_page": s.joined_old_page,
            "incremental_vs_docx2svg": compare_pages(&new_pag, &want, &e.info),
            "full_rust_vs_docx2svg": compare_pages(&full, &want, &e.info),
        }));
    }
    json!({"base": base_path, "base_pagination_equal": base_ok, "rows": rows})
}

/// The base flow repeated `copies` times (blocks renumbered): pagination-and-measure timing at scale.
fn scale(base_path: &str, copies: usize, edited: &[String]) -> Value {
    let rss0 = max_rss_mb();
    let base = load(base_path);
    let t = Tables::new(&base);
    let doc = Doc::new(&base);
    let n = base.items.len();
    let span = base.items.iter().map(|it| match it {
        ItemIn::Para(p) => p.block + 1,
        ItemIn::Table(tb) => tb.rows.iter().flat_map(|r| r.cells.iter()).flat_map(|c| c.paragraphs.iter()).map(|p| p.block + 1).max().unwrap_or(tb.block + 1),
    }).max().unwrap() as u32;
    // measure the repeated input: every paragraph of every copy measured again (no cache)
    let idx: Vec<usize> = (0..n * copies).map(|k| k % n).collect();
    let renumber = |mut it: FItem, copy: usize| -> FItem {
        let add = span * copy as u32;
        match &mut it {
            FItem::P(p) => p.block += add,
            FItem::T(tb) => tb.block += add,
        }
        it
    };
    let measure_all = |parallel: bool| -> Vec<ooxml_word::paginate::It> {
        let one = |(k, i): (usize, &usize)| renumber((*measured_flow(&base, &t, &[*i], false).pop().unwrap()).clone(), k / n);
        if parallel {
            idx.par_iter().enumerate().map(|x| ooxml_word::paginate::It::new(one(x))).collect()
        } else {
            idx.iter().enumerate().map(|x| ooxml_word::paginate::It::new(one(x))).collect()
        }
    };
    let c0 = cpu_ms();
    let (measure_single, _) = best(1, || measure_all(false).len());
    let measure_single_cpu = cpu_ms() - c0;
    let c1 = cpu_ms();
    let (measure_par, flow) = best(1, || measure_all(true));
    let measure_par_cpu = cpu_ms() - c1;
    let c2 = cpu_ms();
    let (pag_ms, pag) = best(3, || paginate_all(&doc, &flow));
    let pag_cpu = (cpu_ms() - c2) / 3.0;
    let lines: usize = flow.iter().map(|i| match &**i {
        FItem::P(p) => p.lines.len(),
        FItem::T(tb) => tb.rows.iter().flat_map(|r| r.cells.iter()).map(|c| c.stack.len()).sum(),
    }).sum();
    // the snapshot's own size, counted
    let bytes: usize = flow.iter().map(|i| std::mem::size_of::<FItem>() + 16 + match &**i {
        FItem::P(p) => p.lines.capacity() * 16 + p.page_breaks.capacity() * 4,
        FItem::T(tb) => tb.rows.iter().map(|r| std::mem::size_of_val(r) + r.cells.iter().map(|c| std::mem::size_of_val(c) + c.stack.capacity() * std::mem::size_of::<ooxml_word::paginate::Stk>()).sum::<usize>()).sum(),
    }).sum::<usize>() + pag.starts.len() * (std::mem::size_of::<Pos>() * 2 + 12) + pag.info.len() * 32;
    // incremental edits at the start, the middle and the end of the long flow: each edited
    // export's changed items, put in copy 0, copy copies/2 and the last copy
    let mut rows = Vec::new();
    let snap = Snapshot::new(doc.clone(), flow, pag);
    for path in edited {
        let e = load(path);
        let ch0 = diff(&digests(&base), &digests(&e));
        let te = Tables::new(&e);
        for copy in [0, copies / 2, copies - 1] {
            let ch = ooxml_word::incr::Change { a: ch0.a + copy * n, b_old: ch0.b_old + copy * n, b_new: ch0.b_new + copy * n };
            let mut m_ms = f64::INFINITY;
            let mut p_ms = f64::INFINITY;
            let mut s_ms = f64::INFINITY;
            let mut st = None;
            let mut pages = 0;
            for _ in 0..3 {
                let t0 = Instant::now();
                let items: Vec<FItem> = e.items[ch0.a..ch0.b_new].iter().map(|it| renumber(item_from_export(&te.px, it, Some(&measure_item(&te, it))), copy)).collect();
                m_ms = m_ms.min(ms(t0));
                let t1 = Instant::now();
                let nf = splice(&snap.items, ch, items);
                s_ms = s_ms.min(ms(t1));
                let t2 = Instant::now();
                let (np, s) = repaginate(&doc, &snap.pag, &nf, ch);
                p_ms = p_ms.min(ms(t2));
                pages = np.starts.len();
                st = Some(s);
            }
            let s = st.unwrap();
            rows.push(json!({"edit": path, "copy": copy, "measure_ms": m_ms, "splice_ms": s_ms, "repaginate_ms": p_ms,
                             "restart_page": s.restart_page, "pages_laid": s.pages_laid, "joined": s.joined_old_page, "pages": pages}));
        }
    }
    // queries on the snapshot
    let pages = snap.pages();
    let (q_ms, _) = best(3, || (0..100_000u32).map(|b| snap.page_of_block((b * 7919) % (span * copies as u32)).unwrap_or(0) as u64).sum::<u64>());
    let mut facts = Vec::new();
    for from in [0, pages / 2, pages - 20] {
        let (f_ms, f) = best(5, || snap.facts(from, from + 20));
        facts.push(json!({"from": from, "ms": f_ms, "lines": f.lines, "past_bottom": f.past_bottom, "keep_next_broken": f.keep_next_broken, "keep_lines_broken": f.keep_lines_broken}));
    }
    let (all_facts_ms, all) = best(1, || snap.facts(0, pages));
    json!({
        "base": base_path, "copies": copies, "items": snap.items.len(), "lines": lines, "pages": pages,
        "measure_all_ms_single": measure_single, "measure_all_cpu_ms_single": measure_single_cpu,
        "measure_all_ms_parallel": measure_par, "measure_all_cpu_ms_parallel": measure_par_cpu, "threads": rayon::current_num_threads(),
        "paginate_full_ms_best_of_3": pag_ms, "paginate_full_cpu_ms_mean": pag_cpu,
        "snapshot_bytes_counted": bytes, "max_rss_mb_before": rss0, "max_rss_mb": max_rss_mb(),
        "edits": rows, "page_of_block_100k_queries_ms": q_ms, "facts_20_pages": facts,
        "facts_all_pages_ms": all_facts_ms, "facts_all": {"lines": all.lines, "past_bottom": all.past_bottom, "keep_next_broken": all.keep_next_broken, "keep_lines_broken": all.keep_lines_broken},
    })
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let out = match args.get(1).map(String::as_str) {
        Some("check") => check(&args[2]),
        Some("edits") => edits(&args[2], &args[3..]),
        Some("scale") => scale(&args[2], args[3].parse().unwrap(), &args[4..]),
        _ => panic!("usage: spike check|edits|scale ..."),
    };
    println!("{}", serde_json::to_string_pretty(&out).unwrap());
}
