# Development

## Running the tests

docx2svg's tests need the Rust core's module (`ooxml_core`): install `python/ooxml-core`
(maturin) into the environment, then `python -m pytest`.  They hold the thin layer to the
Rust core's own answers (the same SVG as `LaidPages.svg`, the options it takes and refuses,
the withdrawn names); the layout itself is tested in the Rust crates and against Word in
`crates/ooxml-oracle`.

## Constraints

No Microsoft font file is committed: tests that need an Office face skip where it is absent.
