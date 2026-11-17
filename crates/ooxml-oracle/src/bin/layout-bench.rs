//! `layout-bench <file.docx> <fonts.json> [--rounds N] [--check]`: the Word layout's
//! benchmark (`ooxml_oracle::layout_bench`).

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Err(e) = ooxml_oracle::layout_bench::run(&args) {
        eprintln!("{e}");
        std::process::exit(2);
    }
}
