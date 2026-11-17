//! `metafile-bench CASES_DIR [ROUNDS]`: draws every recorded metafile case (as `metafile-diff`
//! reads them) and writes its SVG, `ROUNDS` times (3 by default); prints the number drawn and
//! the best round's wall time as JSON. The process's CPU time is the caller's to measure.

use ooxml_metafile::{draw, Options};
use ooxml_oracle::metafile::{read_case, Case};
use std::time::Instant;

fn main() {
    let mut args = std::env::args().skip(1);
    let Some(dir) = args.next() else {
        eprintln!("metafile-bench CASES_DIR [ROUNDS]");
        std::process::exit(2);
    };
    let rounds: usize = args.next().and_then(|r| r.parse().ok()).unwrap_or(3);
    let mut paths: Vec<_> = std::fs::read_dir(&dir)
        .map(|entries| entries.filter_map(|e| e.ok().map(|e| e.path())).collect())
        .unwrap_or_default();
    paths.sort();
    let data: Vec<Vec<u8>> = paths
        .iter()
        .filter_map(|p| std::fs::read(p).ok())
        .filter_map(|b| match read_case(&b) {
            Ok(Case::Metafile { data, .. }) => Some(data),
            _ => None,
        })
        .collect();
    let options = Options::default();
    let mut best = f64::INFINITY;
    let mut bytes = 0usize;
    for _ in 0..rounds {
        let started = Instant::now();
        bytes = data
            .iter()
            .filter_map(|d| draw(d, &options))
            .map(|d| d.svg().len())
            .sum();
        best = best.min(started.elapsed().as_secs_f64());
    }
    println!(
        "{{\"metafiles\": {}, \"svg_bytes\": {bytes}, \"rounds\": {rounds}, \"best_wall_s\": {best:.4}}}",
        data.len()
    );
}
