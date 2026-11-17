# Dependencies of the Rust workspace

Every dependency of the architecture's crates (docs/architecture/README.md, section 2.7), and why
it is there. New dependencies come from the approved list (section 14, Q7, decided 2026-11-16) or
are asked for first. The prototype crates (`crates/core`, `crates/word`, `crates/python`) keep
theirs until they are replaced.

| Dependency | Licence | Used by | Why |
|---|---|---|---|
| `thiserror` 2 | MIT or Apache-2.0 | ooxml-base, ooxml-xml, ooxml-zip, ooxml-opc, ooxml-derive, ooxml-store, ooxml-word-model | One typed error enum per crate (section 2.5). Approved (Q7). |
| `num-rational`, `num-integer`, `num-traits` | MIT or Apache-2.0 | ooxml-base | Exact rationals (`Q`) for layout arithmetic, as the prototype's `units.rs` (which `ooxml_base::units` comes from) uses them; already in the workspace. |
| `flate2` 1, `zlib-rs` backend only (pulls `zlib-rs`, Zlib) | MIT or Apache-2.0 | ooxml-zip | Raw deflate and inflate of ZIP entries, in Rust (no C zlib). Approved (Q7: "flate2 with zlib-rs"). |
| `arc-swap` 1 | MIT or Apache-2.0 | ooxml-store | The published head version: readers load it without a lock and never wait for a writer (section 6). Approved (Q7). |
| `serde_json` 1 (with `serde`; `preserve_order` pulls `indexmap`) | MIT or Apache-2.0 | ooxml-oracle (test support only) | The differential harness's Rust side writes the Word model as JSON for `crates/ooxml-oracle/python/word_model_diff.py` to compare with docx2svg's, field by field; already in the workspace (the prototype's JSON boundary). No shipped crate depends on it. |
| `loom` 0.7 (pulls `generator`, `scoped-tls`, `tracing`, `tracing-subscriber` and their dependencies) | MIT (its dependencies MIT, Apache-2.0 or Unlicense) | ooxml-store, only when built with `RUSTFLAGS="--cfg loom"` | Model-checks every interleaving of the commit path (section 12, acceptance 3 of section 15); no ordinary build compiles it. Its Windows dependencies bring a second `syn` major version into Cargo.lock for that build only. |
| `libfuzzer-sys` 0.4 (pulls `arbitrary`, `cc`) | MIT or Apache-2.0, NCSA | `fuzz/` only (its own workspace) | cargo-fuzz's harness for the fuzz targets section 11 asks for; never part of a shipped crate. |

Considered and not taken:
- `imbl` (persistent collections; on the approved list as the alternative to an in-house rope) is
  MPL-2.0, which section 2.7 allows only at file level for the MTX decoder port. ooxml-xml's child
  rope, ooxml-opc's part map and ooxml-store's per-key write stamps
  (`crates/ooxml-store/src/stamps.rs`) are in-house (a measurement of the rope against
  `imbl::Vector` is in ADR-003).
