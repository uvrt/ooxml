//! Incremental pagination: a checkpoint at every page start (its position, page index and the
//! number before it); restart at the first page whose decision read the first changed item,
//! stop where a page starts at an old page's start again (past the change), keep the rest.

use crate::paginate::{paginate, Doc, FItem, It, Pagination, Pos};

/// Items `[a, b_old)` of the old flow are `[a, b_new)` of the new one.
#[derive(Clone, Copy, Debug)]
pub struct Change {
    pub a: usize,
    pub b_old: usize,
    pub b_new: usize,
}

/// The change between two flows by their items' content digests: common prefix and suffix.
pub fn diff(old: &[Option<String>], new: &[Option<String>]) -> Change {
    let mut a = 0;
    while a < old.len() && a < new.len() && old[a].is_some() && old[a] == new[a] {
        a += 1;
    }
    let (mut bo, mut bn) = (old.len(), new.len());
    while bo > a && bn > a && old[bo - 1].is_some() && old[bo - 1] == new[bn - 1] {
        bo -= 1;
        bn -= 1;
    }
    Change {
        a,
        b_old: bo,
        b_new: bn,
    }
}

#[derive(Clone, Debug, Default)]
pub struct Stats {
    pub restart_page: usize,
    pub pages_laid: usize,
    /// The old page the new pages lined up with again (`None`: ran to the end).
    pub joined_old_page: Option<usize>,
}

fn shift(p: &Pos, by: i64) -> Pos {
    Pos {
        item: (p.item as i64 + by) as u32,
        line: p.line,
        part: p.part.clone(),
    }
}

/// The new flow's pagination from the old one's: the pages before the change kept, the
/// change's pages laid out again, the rest kept (moved) once the starts line up.
pub fn repaginate(doc: &Doc, old: &Pagination, new_items: &[It], ch: Change) -> (Pagination, Stats) {
    let target = Pos::at(ch.a as u32, 0);
    let mut p = old.reach.partition_point(|r| *r < target);
    if p >= old.starts.len() {
        p = old.starts.len().saturating_sub(1);
    }
    let resume = (old.starts[p].clone(), old.page_at[p], old.number_before[p]);
    let delta = ch.b_new as i64 - ch.b_old as i64;
    let mut joined: Option<usize> = None;
    let mut until = |pos: &Pos, _page: u32, number: i64| -> bool {
        if (pos.item as usize) < ch.b_new {
            return false;
        }
        let old_pos = shift(pos, -delta);
        if let Ok(q) = old.starts.binary_search(&old_pos) {
            if !doc.even_odd || (old.number_before[q] - number) % 2 == 0 {
                joined = Some(q);
                return true;
            }
        }
        false
    };
    let (fresh, stop) = paginate(doc, new_items, Some(resume), &mut until);
    let mut out = Pagination::default();
    let first_page = old.page_at[p] as usize;
    out.starts.extend_from_slice(&old.starts[..p]);
    out.page_at.extend_from_slice(&old.page_at[..p]);
    out.number_before.extend_from_slice(&old.number_before[..p]);
    out.reach.extend_from_slice(&old.reach[..p]);
    out.info.extend_from_slice(&old.info[..first_page]);
    let pages_laid = fresh.starts.len();
    out.starts.extend(fresh.starts);
    out.page_at.extend(fresh.page_at);
    out.number_before.extend(fresh.number_before);
    out.reach.extend(fresh.reach);
    out.info.extend(fresh.info);
    if let (Some(q), Some((_, page, number))) = (joined, stop) {
        let page_shift = page as i64 - old.page_at[q] as i64;
        let number_shift = number - old.number_before[q];
        for k in q..old.starts.len() {
            out.starts.push(shift(&old.starts[k], delta));
            out.page_at.push((old.page_at[k] as i64 + page_shift) as u32);
            out.number_before.push(old.number_before[k] + number_shift);
            out.reach.push(shift(&old.reach[k], delta));
        }
        for info in &old.info[old.page_at[q] as usize..] {
            let mut i = info.clone();
            i.number += number_shift;
            out.info.push(i);
        }
    }
    (
        out,
        Stats {
            restart_page: first_page,
            pages_laid,
            joined_old_page: joined,
        },
    )
}

/// Splice the changed items into the old flow.
pub fn splice(old: &[It], ch: Change, changed: Vec<FItem>) -> Vec<It> {
    let mut out = Vec::with_capacity(old.len() + changed.len());
    out.extend_from_slice(&old[..ch.a]);
    out.extend(changed.into_iter().map(It::new));
    out.extend_from_slice(&old[ch.b_old..]);
    out
}
