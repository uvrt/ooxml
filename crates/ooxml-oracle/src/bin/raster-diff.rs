//! `raster-diff CASES_DIR...`: the PNG differential -- every call to resvg-py that
//! `crates/ooxml-oracle/python/raster_record.py` recorded, drawn again by `ooxml-raster`
//! and compared pixel for pixel. Prints each difference, a summary and the times (resvg-py's
//! per call against the font set's and the drawing's here); exits 1 when a call differs.

use ooxml_oracle::raster::{compare, read_call, replay};
use ooxml_raster::{Families, FontLoader};

fn main() {
    let mut system = FontLoader::new(Families::platform_default());
    system.system();
    let system = system.build();
    let (mut same, mut different) = (0usize, 0usize);
    let (mut python, mut fonts, mut draw) = (0.0f64, 0.0f64, 0.0f64);
    for dir in std::env::args().skip(1) {
        let mut stems: Vec<_> = match std::fs::read_dir(&dir) {
            Ok(entries) => entries
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.extension().is_some_and(|e| e == "json"))
                .map(|p| p.with_extension(""))
                .collect(),
            Err(e) => {
                eprintln!("{dir}: {e}");
                std::process::exit(2);
            }
        };
        stems.sort();
        for stem in stems {
            let result = read_call(&stem).and_then(|call| replay(&call, &system).map(|r| (call, r)));
            let (call, replayed) = match result {
                Ok(found) => found,
                Err(e) => {
                    different += 1;
                    println!("DIFF {} [error] {e}", stem.display());
                    continue;
                }
            };
            python += call.python_seconds;
            fonts += replayed.fonts_seconds;
            draw += replayed.draw_seconds;
            println!(
                "TIME {} {}#{} python {:.4} fonts {:.4} draw {:.4}",
                stem.display(),
                call.document,
                call.number,
                call.python_seconds,
                replayed.fonts_seconds,
                replayed.draw_seconds
            );
            match compare(&replayed.png, &call.png) {
                Some(d) if d.pixels == 0 => same += 1,
                Some(d) => {
                    different += 1;
                    println!(
                        "DIFF {} {}#{} {} pixels, largest channel difference {}",
                        stem.display(),
                        call.document,
                        call.number,
                        d.pixels,
                        d.largest
                    );
                }
                None => {
                    different += 1;
                    println!("DIFF {} [size] {}#{}", stem.display(), call.document, call.number);
                }
            }
        }
    }
    println!("{same} calls pixel-identical, {different} different");
    println!("seconds: resvg-py {python:.3}, here fonts {fonts:.3} + draw {draw:.3}");
    std::process::exit(i32::from(different > 0));
}
