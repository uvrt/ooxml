//! Owns: resolution -- from what each level declares to what Word applies: the cascade
//! (`cascade`, `run`), tables (`table`), faces and sizes per character slot (`fonts`),
//! where each resolved value came from (`origin`: the `*_traced` resolutions) --
//! and [`Resolver`], which memoises resolved formats by their inputs (a document's
//! thousands of paragraphs and runs have a few dozen distinct ones) for as long as the
//! parts they resolve against are unchanged.
//!
//! Must not: read XML, or hold a document.

mod cascade;
mod fonts;
mod origin;
mod run;
mod table;

pub use cascade::{
    banded, merge, resolve_paragraph, resolve_paragraph_traced, ParagraphInput, Sources, AUTOSPACING_TWIPS,
};
pub use fonts::{
    character_format, complex_script, slot_for, theme_face, CharacterFormat, Slot, APPLICATION_HALF_POINTS,
};
pub use origin::{Holder, Level, Origin, Origins, Trace};
pub use run::{resolve_run, resolve_run_traced, RunInput};
pub use table::{
    border_key, border_twips, drawn, edge_winner, fitted_grid, resolve_table, resolve_table_traced, second_wins, Edge,
    FittedGrid, ResolvedCell, ResolvedRow, ResolvedTable, TableOrigins, SIDES,
};

use crate::document::{Context, ResolvedParagraph};
use crate::model::{ParaProps, Paragraph, Run, RunProps, Table};
use crate::value::{Props, Text};
use ooxml_derive::{CacheBudget, Memo, MemoStats, Weigh};
use std::sync::Arc;

/// A memoised value.
#[derive(Debug, Clone)]
struct Kept<T>(Arc<T>);

impl Weigh for Kept<Props> {
    fn weight(&self) -> usize {
        64 + self.0.len() * 64
    }
}

impl Weigh for Kept<ResolvedTable> {
    fn weight(&self) -> usize {
        128 + self.0.rows.iter().map(|r| 64 + r.cells.len() * 512).sum::<usize>()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ParagraphKey {
    props: Option<Arc<ParaProps>>,
    table_style: Option<Text>,
    conditions: Arc<[Text]>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct RunKey {
    paragraph_style: Option<Text>,
    table_style: Option<Text>,
    conditions: Arc<[Text]>,
    run: Option<Arc<RunProps>>,
    toc_link: bool,
    mark: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct TableKey {
    style_id: Option<Text>,
    properties: Props,
    row_properties: Arc<[Props]>,
    cell_properties: Arc<[Arc<[Props]>]>,
    conditions: Arc<[crate::model::RowConditions]>,
    grid: Arc<[i64]>,
}

/// Resolved formats memoised by their inputs, valid for one set of parts (styles,
/// numbering, settings, theme); [`Resolver::clear`] when they change.
pub struct Resolver {
    paragraphs: Memo<ParagraphKey, Kept<Props>>,
    runs: Memo<RunKey, Kept<Props>>,
    tables: Memo<TableKey, Kept<ResolvedTable>>,
}

impl std::fmt::Debug for Resolver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Resolver").field("stats", &self.stats()).finish()
    }
}

fn sources(cx: &Context) -> Sources<'_> {
    Sources {
        styles: cx.styles.as_deref(),
        numbering: &cx.numbering,
        settings: &cx.settings,
    }
}

impl Resolver {
    /// An empty resolver reserving from `budget`.
    pub fn new(budget: &Arc<dyn CacheBudget>) -> Resolver {
        Resolver {
            paragraphs: Memo::new(Arc::clone(budget)),
            runs: Memo::new(Arc::clone(budget)),
            tables: Memo::new(Arc::clone(budget)),
        }
    }

    /// Forget everything (the parts resolved against changed).
    pub fn clear(&mut self) {
        self.paragraphs.clear();
        self.runs.clear();
        self.tables.clear();
    }

    /// Drop the resolutions not used since the last sweep that nothing holds any more
    /// ([`Memo::sweep`]). Returns how many were dropped.
    pub fn sweep(&mut self) -> usize {
        self.paragraphs.sweep(|kept| Arc::strong_count(&kept.0) > 1)
            + self.runs.sweep(|kept| Arc::strong_count(&kept.0) > 1)
            + self.tables.sweep(|kept| Arc::strong_count(&kept.0) > 1)
    }

    /// The memos' counters: paragraphs, runs and tables together.
    pub fn stats(&self) -> MemoStats {
        let (a, b, c) = (self.paragraphs.stats(), self.runs.stats(), self.tables.stats());
        MemoStats {
            hits: a.hits + b.hits + c.hits,
            misses: a.misses + b.misses + c.misses,
            evictions: a.evictions + b.evictions + c.evictions,
            entries: a.entries + b.entries + c.entries,
            bytes: a.bytes + b.bytes + c.bytes,
        }
    }

    /// A paragraph's own resolved properties.
    pub fn paragraph_props(&mut self, cx: &Context, p: &Paragraph) -> Arc<Props> {
        let key = ParagraphKey {
            props: p.properties.clone(),
            table_style: p.table_style_id.clone(),
            conditions: Arc::clone(&p.table_conditions),
        };
        let empty = Props::new();
        let input = ParagraphInput {
            style_id: p.properties.as_ref().and_then(|x| x.style_id.as_deref()),
            table_style: p.table_style_id.as_deref(),
            conditions: &p.table_conditions,
            direct: p.properties.as_ref().map_or(&empty, |x| &x.declared),
        };
        self.paragraphs
            .get_or_compute(&key, || {
                Kept(Arc::new(cascade::resolve_paragraph(&sources(cx), &input)))
            })
            .0
    }

    fn run_props(
        &mut self,
        cx: &Context,
        p: &Paragraph,
        run: Option<&Arc<RunProps>>,
        toc_link: bool,
        mark: bool,
    ) -> Arc<Props> {
        let paragraph_style = p.properties.as_ref().and_then(|x| x.style_id.clone());
        let key = RunKey {
            paragraph_style: paragraph_style.clone(),
            table_style: p.table_style_id.clone(),
            conditions: Arc::clone(&p.table_conditions),
            run: run.cloned(),
            toc_link,
            mark,
        };
        let empty = Props::new();
        let input = RunInput {
            paragraph_style: paragraph_style.as_deref(),
            table_style: p.table_style_id.as_deref(),
            conditions: &p.table_conditions,
            character_style: if mark {
                None
            } else {
                run.and_then(|r| r.style_id.as_deref())
            },
            toc_link,
            direct: run.map_or(&empty, |r| &r.declared),
        };
        self.runs
            .get_or_compute(&key, || Kept(Arc::new(run::resolve_run(&sources(cx), &input))))
            .0
    }

    /// A run's resolved properties.
    pub fn run(&mut self, cx: &Context, p: &Paragraph, run: &Run) -> Arc<Props> {
        self.run_props(cx, p, run.properties.as_ref(), run.toc_link, false)
    }

    /// A paragraph mark's resolved run properties.
    pub fn mark(&mut self, cx: &Context, p: &Paragraph) -> Arc<Props> {
        let mark = p.properties.as_ref().and_then(|x| x.mark.clone());
        self.run_props(cx, p, mark.as_ref(), false, true)
    }

    /// A paragraph's resolved formats: its own, its mark's and each run's.
    pub fn paragraph(&mut self, cx: &Context, p: &Paragraph) -> ResolvedParagraph {
        ResolvedParagraph {
            paragraph: self.paragraph_props(cx, p),
            mark: self.mark(cx, p),
            runs: p.runs.iter().map(|r| self.run(cx, p, r)).collect(),
        }
    }

    /// A table's resolved formats.
    pub fn table(&mut self, cx: &Context, t: &Table) -> Arc<ResolvedTable> {
        let key = TableKey {
            style_id: t.style_id.clone(),
            properties: t.properties.clone(),
            row_properties: Arc::clone(&t.row_properties),
            cell_properties: Arc::clone(&t.cell_properties),
            conditions: Arc::clone(&t.conditions),
            grid: Arc::clone(&t.grid),
        };
        self.tables
            .get_or_compute(&key, || Kept(Arc::new(table::resolve_table(&sources(cx), t))))
            .0
    }
}
