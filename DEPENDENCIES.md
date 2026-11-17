# Dependencies of the Rust workspace

Every dependency of the architecture's crates (docs/architecture/README.md, section 2.7), and why
it is there. New dependencies come from the approved list (section 14, Q7, decided 2026-11-16) or
are asked for first. The prototype crates (`crates/core`, `crates/word`, `crates/python`) keep
theirs until they are replaced.

| Dependency | Licence | Used by | Why |
|---|---|---|---|
| `thiserror` 2 | MIT or Apache-2.0 | ooxml-base, ooxml-xml, ooxml-zip, ooxml-opc, ooxml-derive, ooxml-store, ooxml-word-model, ooxml-word-layout, ooxml-colour | One typed error enum per crate (section 2.5). Approved (Q7). |
| `num-rational`, `num-integer`, `num-traits` | MIT or Apache-2.0 | ooxml-base | Exact rationals (`Q`) for layout arithmetic, as the prototype's `units.rs` (which `ooxml_base::units` comes from) uses them; already in the workspace. |
| `flate2` 1, `zlib-rs` backend only (pulls `zlib-rs`, Zlib) | MIT or Apache-2.0 | ooxml-zip | Raw deflate and inflate of ZIP entries, in Rust (no C zlib). Approved (Q7: "flate2 with zlib-rs"). |
| `arc-swap` 1 | MIT or Apache-2.0 | ooxml-store | The published head version: readers load it without a lock and never wait for a writer (section 6). Approved (Q7). |
| `loom` 0.7 (pulls `generator`, `scoped-tls`, `tracing`, `tracing-subscriber` and their dependencies) | MIT (its dependencies MIT, Apache-2.0 or Unlicense) | ooxml-store, only when built with `RUSTFLAGS="--cfg loom"` | Model-checks every interleaving of the commit path (section 12, acceptance 3 of section 15); no ordinary build compiles it. Its Windows dependencies bring a second `syn` major version into Cargo.lock for that build only. |
| `read-fonts` 0.47 (fontations; pulls `font-types`, `bytemuck` with `bytemuck_derive`, `yoke` with `yoke-derive`, `zerofrom` with `zerofrom-derive`, `stable_deref_trait`, `synstructure`) | MIT or Apache-2.0 (`bytemuck`: Zlib, Apache-2.0 or MIT; `yoke`, `zerofrom`: Unicode-3.0) | ooxml-fonts | The one sfnt reader (section 2.2): font collections and table directories, and the bounds-checked big-endian reads every table field is taken with. Approved (Q7, "the fontations crates"). `skrifa` is not taken: no outline or shaping is read yet. |
| `flate2` (as above) | | ooxml-text | Reading the gzip-compressed English hyphenation word list docx2svg ships (`patterns/en-moby.txt.gz`), compiled in. |
| `num-rational`, `num-integer`, `num-traits` (as above) | | ooxml-text | Exact widths in 1/4096 pt (`Units`): an advance at a size is a rational wherever the face's units per em does not divide it. |
| `num-integer` (as above) | | ooxml-word-layout | Exact device px (`Px`, docx2svg's `Fraction` px): floor division in Word's roundings to the layout unit and to whole pixels. No new crate. |
| `serde_json` 1 (with `serde`; `preserve_order` pulls `indexmap`; already in the workspace) | MIT or Apache-2.0 | ooxml-oracle (test support only) | Reading docx2svg's differential records (the text engine's), and writing the Word model as JSON for `crates/ooxml-oracle/python/word_model_diff.py` to compare with docx2svg's field by field, and DrawingML's model and resolved colours for `crates/ooxml-oracle/python/drawingml_diff.py` to compare with ooxml-common's. Accepted as test support (2026-11-17). No shipped crate depends on it. |
| `serde_json` 1 (with `serde`, `preserve_order`; already in the workspace and `Cargo.lock`) | MIT or Apache-2.0 | ooxml-tools | Tool calls are JSON in and JSON out (section 9's decision; spike 2 measured serde at 4.1 µs a call): a call's arguments are read, validated and normalised as JSON, and a schema, a result envelope and an export are written as JSON key for key in the order today's Python writes them (`preserve_order`). Approved by the user (2026-11-17), added to the Q7 list; the first shipped crate to depend on it. |
| `thiserror` (as above) | | ooxml-tools | `ToolError` and `ArgError` (section 2.5). |
| `serde_json` (as above) | | ooxml-tools-word, ooxml-session | A tool's reply data and a call's arguments are JSON (section 9); the session takes a call's arguments as JSON and returns the envelope `ooxml-tools` writes. |
| `thiserror` (as above) | | ooxml-session | `SessionError` (section 2.5). |
| `pyo3` 0.30 (already in the workspace and `Cargo.lock` through `crates/python`) | MIT or Apache-2.0 | ooxml-python-session | The Python module's session API: `Session`, `CallContext`, `CancelToken`, `tool_specs`, with every Rust call made with the interpreter lock released (`py.detach`). Approved (Q7). Its features (`extension-module`, `abi3-py310` for the GIL wheel) are chosen by the module crate that links it, `crates/python`. |
| `serde_json` (as above) | | ooxml-python-session | The specs' export and a reply as JSON text. |
| `serde_json` (as above) | | xtask | Reading `cargo metadata`'s JSON for `check-layers`. Build tooling, never shipped. |
| `syn` 2.0.119 (feature `full`), `quote` 1.0.47, `proc-macro2` 1.0.107 (pulls `unicode-ident`; all already in `Cargo.lock` through `thiserror`'s derive) | MIT or Apache-2.0 (`unicode-ident`: also Unicode-3.0) | ooxml-tools-derive (a proc-macro crate; build time only) | `#[derive(ToolArgs)]` and `#[derive(ArgEnum)]`: reading a tool's struct, its fields' attributes and doc comments, and writing the trait impls (section 9). Expected by the lane's brief. |
| `thiserror` (as above) | | ooxml-metafile | `ReadError`, why a record cannot be read (section 2.5). |
| `flate2` (as above, `zlib-rs` backend) | | ooxml-metafile | The PNGs a metafile's bitmaps and path gradients are embedded as (zlib at level 9, as ooxml-common writes them), and reading back an EMF+ PNG's pixels to pad it. zlib-rs's deflate stream is not always the C zlib's byte for byte at level 9 (measured: 14 of the 14,711 recorded and mutated cases of lane P3b1's gate): the pixels are equal, and the differential compares embedded PNGs by their pixels, as ooxml-common's goldens do. |
| `ooxml-metafile`, `ooxml-scene` (workspace crates) | | ooxml-oracle (test support only) | The metafile players' differential (`metafile-diff`, `tests/metafile_cases.rs`). |
| `thiserror` (as above) | | ooxml-numfmt | `LocaleError`, why a locale or language tag is refused (section 2.5). |
| `ooxml-chart`, `ooxml-numfmt` (workspace crates); `serde_json`'s `float_roundtrip` feature (no new crate) | | ooxml-oracle (test support only) | The chart differential (`chart-model`, `python/chart_diff.py`, `tests/chart_layout.rs`). `float_roundtrip` makes serde_json read the frozen Python answers' numbers exactly: its default reading can be a unit in the last place off, which a byte-for-byte comparison reports. |
| `libfuzzer-sys` 0.4 (pulls `arbitrary`, `cc`) | MIT or Apache-2.0, NCSA | `fuzz/` only (its own workspace) | cargo-fuzz's harness for the fuzz targets section 11 asks for; never part of a shipped crate. |

Considered and not taken:
- `imbl` (persistent collections; on the approved list as the alternative to an in-house rope) is
  MPL-2.0, which section 2.7 allows only at file level for the MTX decoder port. ooxml-xml's child
  rope, ooxml-opc's part map and ooxml-store's per-key write stamps
  (`crates/ooxml-store/src/stamps.rs`) are in-house (a measurement of the rope against
  `imbl::Vector` is in ADR-003).
