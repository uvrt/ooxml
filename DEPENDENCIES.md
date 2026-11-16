# Dependencies of the Rust workspace

Every dependency of the architecture's crates (docs/architecture/README.md, section 2.7), and why
it is there. New dependencies come from the approved list (section 14, Q7, decided 2026-11-16) or
are asked for first. The prototype crates (`crates/core`, `crates/word`, `crates/python`) keep
theirs until they are replaced.

| Dependency | Licence | Used by | Why |
|---|---|---|---|
| `thiserror` 2 | MIT or Apache-2.0 | ooxml-base, ooxml-xml, ooxml-zip, ooxml-opc | One typed error enum per crate (section 2.5). Approved (Q7). |
| `num-rational`, `num-integer`, `num-traits` | MIT or Apache-2.0 | ooxml-base | Exact rationals (`Q`) for layout arithmetic, as the prototype's `units.rs` (which `ooxml_base::units` comes from) uses them; already in the workspace. |
| `flate2` 1, `zlib-rs` backend only (pulls `zlib-rs`, Zlib) | MIT or Apache-2.0 | ooxml-zip | Raw deflate and inflate of ZIP entries, in Rust (no C zlib). Approved (Q7: "flate2 with zlib-rs"). |
| `libfuzzer-sys` 0.4 (pulls `arbitrary`, `cc`) | MIT or Apache-2.0, NCSA | `fuzz/` only (its own workspace) | cargo-fuzz's harness for the fuzz targets section 11 asks for; never part of a shipped crate. |

Considered and not taken:
- `imbl` (persistent collections; on the approved list as the alternative to an in-house rope) is
  MPL-2.0, which section 2.7 allows only at file level for the MTX decoder port. ooxml-xml's child
  rope and ooxml-opc's part map are in-house (a measurement of the rope against `imbl::Vector` is
  in ADR-003).
