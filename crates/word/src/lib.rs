//! Word's layout on the core: pagination (`paginate`), tables (`table`), incremental
//! relayout (`incr`), snapshots of a laid-out document (`snapshot`), and the engine behind a
//! document's owner with the field passes (`engine`) -- docx2svg's layout, exact.
//!
//! Moved in from the performance lanes' prototype (lane S: kernels; lane E: the owner).
// The prototype's shapes, kept as moved in: its tuples of exported numbers, its placing
// functions' many arguments and its large exported paragraph.  Named types come with the port.
#![allow(clippy::type_complexity, clippy::too_many_arguments, clippy::large_enum_variant)]
pub mod engine;
pub mod facts;
pub mod fieldpages;
pub mod incr;
pub mod linebreak;
pub mod paginate;
pub mod parse;
pub mod snapshot;
pub mod table;

// The core's modules, under the paths this crate's code was written against; `measure`
// is the core's with Word's line breaking (`linebreak`) beside it, as it was one module.
pub use ooxml_core::{model, owner, units, xml};
pub mod measure {
    pub use crate::linebreak::*;
    pub use ooxml_core::measure::*;
}

use measure::{measure, LineH, Tables};
use model::{Export, ItemIn};
use rayon::prelude::*;

/// Every item's measured line heights: a paragraph's one list, a table's one per cell paragraph.
pub fn measure_item(t: &Tables, it: &ItemIn) -> Vec<Vec<LineH>> {
    match it {
        ItemIn::Para(p) => vec![measure(t, &p.measure).heights],
        ItemIn::Table(tb) => tb
            .rows
            .iter()
            .flat_map(|r| r.cells.iter())
            .flat_map(|c| c.paragraphs.iter())
            .map(|p| measure(t, &p.measure).heights)
            .collect(),
    }
}

/// The flow measured by the Rust kernel (in parallel where `parallel`).
pub fn measured_flow(e: &Export, t: &Tables, items: &[usize], parallel: bool) -> Vec<paginate::It> {
    let one = |k: &usize| {
        let it = &e.items[*k];
        let hs = measure_item(t, it);
        paginate::It::new(paginate::item_from_export(&t.px, it, Some(&hs)))
    };
    if parallel {
        items.par_iter().map(one).collect()
    } else {
        items.iter().map(one).collect()
    }
}
