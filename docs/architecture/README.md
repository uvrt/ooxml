# One shared Rust core for Word, PowerPoint and Excel, with Python as a thin shim: architecture

Status: **approved 2026-11-16**; the decisions on its open questions are in [Decisions](#decisions) at the end. Phase 0
(§15) is under way. Written at main e39128d9 (lanes J and K were open then and are merged since). The design spikes
were throwaway code kept outside this repository; their figures are quoted below. Spike 1's byte equality against the
Python edit path was run at 295 pages only.

Every later lane is held to this document. Where it and the code differ, fix one of them in the same PR.

---

## 0. Summary

**The design rests on six decisions (ADRs below):**
- **Document model:** one lossless, immutable XML tree with structural sharing. It is a green tree: names are interned, and wide elements keep their children in a persistent chunked vector.
- **Write contract:** a touched part is written in the form libxml2 writes; an untouched part keeps its stored bytes. The Rust writer matched lxml on 620 of 620 distinct corpus parts. In 386 of those, lxml's output differs from the stored bytes.
- **Edits:** every tool operation is a command run as an MVCC transaction. It is validated, prepared on a snapshot with a recorded read set, and committed in a critical section of a few µs, then redone if it conflicts.
- **Derived facts:** hand-rolled incremental structures, not salsa. Facts are memoised by content per block, and order-dependent facts are kept in monoid trees. Measured on list labels at 3,000 pages: 19 µs after an edit, against 12.4 ms with salsa.
- **Python boundary:** one PyO3 module, JSON tool calls in and out, with the interpreter lock released. A call costs 4 µs (12 µs counting Python's `json`). Four threads used 3.9 of 4 logical CPUs; with the lock held they used 1.23.
- **Migration:** a strangler, Word first. From phase 2 Rust owns the document; Python tools not yet ported run through an explicit, metered bridge.

**Effort:** about 88 agent lanes in 9 phases, which is about 38 waves with at most 3 agents at once. Phase 0 (the foundation) is ready to brief (§15). **Priority (2026-11-16):** the Word layout moves to Rust right after the foundation (phase PL, §13), before the edit phases.

**The user's decisions** on §14's questions are in [Decisions](#decisions).

---

## 1. Goals, non-goals, quality attributes

**Goals**
1. All document work runs in one Rust core: models and edits for Word, PowerPoint and Excel, Word pagination, slide and chart layout, metafiles, fonts and text, SVG and PNG.
2. Python only declares the tools to providers, adapts to each provider, and plumbs sessions.
3. CPU per document stays bounded and proportional to the change, on 3.5-CPU machines running many documents.
4. Results are the same as today. During migration the Python path is the byte-for-byte oracle.
5. The code stays maintainable for years: layered crates, typed errors, and no switches that select parallel code paths.

**Non-goals**
- New Office features during the port; a behaviour change needs its own approved item.
- An MCP server, or Python escape hatches for running code.
- A formula evaluator for Excel (there is none today).
- PyPI.
- Depending on free-threaded Python for speed: everything heavy runs outside the interpreter lock on either build. Free-threaded 3.15t stays a supported target (Decisions, 2).

**Budgets.** "Today" is measured, from lane K's measurements (docx-agent 0.1.123) unless marked. The targets are design budgets, not measurements. "3k" means long-61.docx: 2,988 pages, a 20 MB document.xml, 70,273 paragraphs and 1,292 tables.

| Operation | Today, 295 pages | Today, 3k | Target, 295 | Target, 3k |
|---|---|---|---|---|
| Open, ready to check | 3.3 s (first check) | 24.7 s | ≤ 0.15 s | ≤ 1.5 s (parse measured at 0.3 s with a naive tree; the owner's measure step was 0.74 s in lane F) |
| Markdown insert, p50 wall | 0.057 s | 0.18 s | ≤ 5 ms | ≤ 15 ms |
| set_text | 0.054 s | 0.19 s | ≤ 3 ms | ≤ 10 ms |
| Table set_cells | 0.145 s | 0.50 s | ≤ 5 ms | ≤ 15 ms |
| Undo / redo | 0.042 s | 0.16 s | ≤ 1 ms | ≤ 2 ms (spike 1: 0.4 µs, plus derived facts) |
| Batch of 2 edits | 0.19 s | 0.50 s | ≤ the two edits' sum | ≤ the two edits' sum |
| Check of a chapter after an edit | 0.077 s | 0.24 s | ≤ 20 ms | ≤ 50 ms (lane E reached 18 ms p50 in Rust) |
| Fields update | 1.3–2.7 s (lane I1) | 14–33 s (lane I1) | ≤ 50 ms | ≤ 300 ms |
| Render one page | not measured here | not measured here | SVG ≤ 20 ms, PNG ≤ 120 ms | SVG ≤ 30 ms, PNG ≤ 150 ms |
| Save | not measured here | not measured here | ≤ 60 ms | ≤ 400 ms (writing the 3k part measured 104 ms, plus deflate) |
| 24 writers on one document | edit p99 6.7 s, check p99 0.59 s | edit p99 25.7 s, check p99 0.70 s, 5.2 GB peak | edit p99 ≤ 100 ms, check p99 ≤ 50 ms | edit p99 ≤ 250 ms, check p99 ≤ 100 ms |
| Memory per open document | ~300 MB process peak | 2–5 GB | ≤ 60 MB | ≤ 400 MB with 50 undo versions |
| Memory per version | ~2 MB (lane H1) | ~2 MB (lane H1) | ≤ 10 KB | ≤ 20 KB per paragraph edit, ≤ 150 KB per insert (spike 1: 9 KB and 128 KB) |
| CPU per edit, including its share of relayout | 0.12–0.8 s | 0.5–4.7 s | ≤ 10 ms | ≤ 30 ms |

Targets at production size for the other formats:
- **Decks of 100 slides:** open ≤ 300 ms; edit ≤ 5 ms; render a slide to SVG ≤ 40 ms and to PNG ≤ 150 ms; check one slide's fit ≤ 20 ms.
- **Workbooks of 200k rows × 20 columns:** streaming read of a range ≤ 50 ms; open with lazy sheets ≤ 300 ms; cell edit ≤ 5 ms; save ≤ 1.5 s, with untouched parts copied raw.

**Quality attributes**
- **Correctness:** every ported tool gives the same results and the same saved part bytes as the Python path. Shown by differential tests and golden transcripts.
- **Concurrency:** serialisable, i.e. every run equals a serial replay in commit order. Shown by a random multi-writer test and by loom on the store.
- **Maintainability:** an acyclic crate graph; no module over 800 lines (generated code exempt); one code path per concept. Checked in CI.
- **Testability:** the core is pure. I/O (fonts, zip, clock, caches) sits behind ports, so every crate can be tested in memory.
- **Robustness:** no panic crosses the FFI boundary, untrusted input is bounded, and fuzzing runs in CI.

---

## 2. Engineering principles and standards (enforced, not advisory)

1. **Layering.**
   - Each crate has a layer number (§3) and may depend only on lower layers.
   - `xtask check-layers` reads `cargo metadata` and fails CI on an upward or sideways edge not in an allow-list. The allow-list starts empty.
   - Cargo already rules out cycles; the layer check also rules out shortcuts.
2. **Single responsibility.**
   - One concept per crate, and one owner per concept: one sfnt reader, one line-breaking loop with per-application rules as data, one colour engine, one address parser per format.
   - The surveys found duplicates that each become one: three font-table readers, two or three GPOS/outline parsers, two line breakers, two sheet readers, two properties writers, and four or five copies of `font_dirs_of`/`locale_of`.
3. **Ports and adapters.**
   - The core is pure. Five traits are injected through a `Context`: `FontSource`, `PartStore`, `Clock`, `CacheBudget` and `Cancel`.
   - Adapters live in leaf crates: file-system fonts, the Office bundle and embedded faces, zip on disk or in memory, the system clock, a test clock.
4. **No global mutable state.**
   - Configuration (fonts, locale, limits, roles, the layout memory budget, the timeout policy) is a value in the session or document `Context`. It is never an environment variable read deep inside the code.
   - CI denies `static` items holding a `Mutex`, `RefCell` or `OnceLock` with mutable content, except in `ooxml-python`. That crate's module object holds the session table, the only process-level state.
   - Caches belong to a document or a session and count against its budget.
5. **Typed errors.**
   - Each crate has a `thiserror` enum. Errors that reach a tool map to `ToolError {code, message, valid_options, try_call, field, details}`.
   - In library code, `unwrap`, `expect`, `panic!`, `todo!` and unchecked indexing in parsers are denied (`clippy::unwrap_used`, `expect_used`, `panic` and `indexing_slicing` set to deny in `[workspace.lints]`). They are allowed in tests and xtask only.
   - The FFI layer wraps every entry point in `catch_unwind`. A panic becomes `ToolError {code: internal}` with a trace id and never aborts the host.
6. **Unsafe policy.**
   - `#![forbid(unsafe_code)]` in every crate except `ooxml-python` (PyO3 needs none of ours there) and possibly `ooxml-xml` for an arena.
   - Any unsafe needs an ADR naming each block, a `// SAFETY:` comment, and Miri in CI.
   - Never read another library's private memory layout.
7. **Dependency and licence policy.**
   - `cargo deny` allows MIT, Apache-2.0, BSD-2/3, ISC, Zlib, Unicode-3.0, and MPL-2.0 at file level only (the MTX decoder port). Copyleft is refused.
   - RustSec advisories are checked, and there is one major version per crate.
   - Every new dependency gets a line in `DEPENDENCIES.md` saying why.
   - The MSRV is pinned and `Cargo.lock` is committed.
8. **Lint, format, docs.**
   - rustfmt with the repo's config.
   - `clippy -D warnings`, with some pedantic lints on: `too_many_lines` at 80, and `cognitive_complexity`.
   - `missing_docs` on public items, and `cargo doc -D warnings`.
   - Every module's header says what it owns and what it must not do.
9. **Size limits.**
   - Files ≤ 800 lines; functions ≤ 80 lines and ≤ 7 arguments. No `allow(too_many_arguments)`, which today's prototype has.
   - Types ≤ 12 fields unless they get a constructor helper.
   - Generated files are exempt and marked `@generated`.
10. **Code generation where OOXML repeats.**
    - Typed element views and constructors (child order, attributes, enums, defaults) are generated from the ECMA-376 transitional XSDs plus the MS-DOCX/PPTX/XLSX extension schemas, pinned by SHA-256.
    - The generated code is checked in so a diff shows it. `xtask codegen` regenerates it, and CI fails if the result differs.
    - Measured Office data (font metrics, kerning classes, preset geometry, built-in table styles, style names, symbol maps) is produced by the existing generators as compact binary tables, loaded with `include_bytes!`. It is never a 13k-line Rust literal.
11. **No feature switches that select code paths.**
    - A behaviour has one code path.
    - Oracles (a full pass checked against the incremental pass) live in tests and in a `debug-oracle` Cargo feature, never behind runtime environment variables.

**Today's hacks that this design removes:**
1. Reading lxml's private memory layout from Rust: `crates/python/src/tree.rs` casts `_Element` to `struct LxmlElement`/`xmlNode`.
2. A step-for-step port of `difflib.SequenceMatcher` (`crates/core/src/seqmatch.rs`) to keep `touched`'s opcodes the same.
   - `touched` comes from the command's change set instead, so it is known rather than rediscovered.
   - The port stays only as an oracle until equality is proven (Q6).
3. Parallel Python and Rust paths chosen by a growing set of switches:
   - `DOCX_AGENT_RUST`, `_RUST_FIELDS`, `_INCREMENTAL`, `_FULL_VALIDATE`, `_DRAFTS`, `_LAYOUT_MEMORY`, `_RUST_STATS`;
   - `OOXML_EDIT_VERIFY_SERIALIZED`, the `block_edits` modes, `batch_defers`;
   - `PPTX2SVG_OFFICE_FONTS`, `paginate.CARRY_ON`/`FIT`, xlsx's `SKIM`.
4. The Python mirror that feeds the owner:
   - the mirror itself, the JSON exporter, `model.rs`'s tuples;
   - the `Resolver` trait that answers JSON-string questions from Python;
   - the document held twice, as an lxml `Document` and in the Rust owner.
5. In today's Rust:
   - a busy-wait: sleeping 200 µs in a loop for the parser's lock (`crates/python/src/lib.rs`, `held`);
   - `panic!("owner failed")` and `unwrap()` across the FFI boundary;
   - a process-global registry of owners keyed by string.
6. ooxml-edit's layered serialisation caches (`_serialized`, `_settled`, `_pieces`, `_changes`, `serializing(keep_settled)`) and byte `Pieces`. They exist only because versions were bytes of mutable trees.
7. Undo that restores by re-parsing bytes; drafts and read copies rebuilt from committed bytes.
8. Writing zipfile's private fields, and sniffing zlib-ng by test-compressing 4,000 rows.
9. Regex byte scanners over XML: docx2svg's `relayout.scan_body`, xlsx's stream skimmer, lane K's `body.rs`, and pptx2svg's live-view and `raster.pass_svg` rewrites of SVG strings.
10. Workarounds for Python's process model:
    - four runner kinds with automatic fallbacks;
    - the `__main__` check and exit code 86;
    - pickled layouts passed between processes;
    - killing workers at the deadline.
11. pptx2svg's 29 `sys.modules` alias modules and the `_SharedFonts` module-class swap.
12. Reliance on private names: `Fraction._from_coprime_ints`, the `ContextVar` for theme colours, and cross-module private imports (`office._cmap_subtable` and others).
13. resvg's text path re-implemented in Python only to predict resvg: `outlines/fontdb.py`, `shaper.py`, `svgtext.py`, `fonts/rasterdb.py`.
14. God objects:
    - `docx_agent.Document`, built from 24 mixins;
    - pptx-agent's `Document`, with 287 methods;
    - `dispatch.py` at 4.3k lines;
    - `DocumentFormat`, with about 40 optional hooks;
    - a chart layout class of about 6,900 lines.
15. Intent acceptance spread across call sites, plus hidden schema keywords (`orSingle`, `orMap`, `valueAliases` and others).
16. pptx-agent's seen-id sets, kept outside the package because undo did not restore them.
17. Spike code shipped in `src`: `pptx_agent/edit/svgprofile.py`.

---

## 3. Crate map (containers and components)

Only downward dependencies are allowed.

```
L6  ooxml-python (cdylib, PyO3: sessions, handles, JSON calls)        xtask (codegen, layers, data tables)
L5  ooxml-session (sessions, documents, refs, datasets, blobs, outputs, scheduler, budgets)
L4  ooxml-tools (contract: specs, strict-schema derive, validation, results, refusals, budgets, batch)
    ooxml-tools-word   ooxml-tools-slides   ooxml-tools-sheets   (tool handlers = commands)
L3  ooxml-word  ooxml-word-layout   ooxml-slides  ooxml-slides-layout   ooxml-sheets
L2  ooxml-drawingml  ooxml-chart  ooxml-metafile  ooxml-scene  ooxml-render (SVG writer)  ooxml-raster (PNG via resvg)
L1  ooxml-opc  ooxml-schema (generated shared views: rels, mc, a:, c:)  ooxml-text  ooxml-fonts
    ooxml-colour (theme, transforms)  ooxml-numfmt (formats, dates, locales)  ooxml-markdown
L0  ooxml-base (units, exact arithmetic, ids, digests, errors, Context ports)  ooxml-xml (lossless tree)
    ooxml-zip  ooxml-store (versions, transactions, MVCC, history; format-neutral)
    ooxml-derive (incremental: content memo, monoid trees, invalidation)
test support: ooxml-oracle (differential harness), ooxml-testdata (made-up generators), fuzz/
```

`ooxml-base` is the bottom of layer 0: the other L0 crates depend on it (for its limits, errors and
digests), and no other edge inside a layer is allowed.

What each crate owns, the outline of its API, and what it must not do:

- **`ooxml-base`**
  - Owns: units (EMU, twips, half-points, 1/4096 pt), exact rationals and fixed point, the `Context` ports, digests.
  - API: `Emu`, `Q`, `Digest`, `Context {fonts, clock, cancel, limits, locale}`.
  - Must not: do I/O.
- **`ooxml-xml`**
  - Owns: the lossless immutable tree; the parser (end-of-line and attribute normalisation, CDATA kept as text, DTD references as opaque nodes); the libxml2-form writer; cursors that know their parents (red nodes); patches.
  - API: `parse(&[u8], &Limits) -> Result<Tree>`, `Tree::write()`, `Cursor`, `Patch`.
  - Must not: know any OOXML name.
- **`ooxml-zip`**
  - Owns: entries, raw copy, deflate, zip-bomb limits.
  - Must not: parse XML.
- **`ooxml-opc`**
  - Owns: package, parts, relationships, content types, lazy part trees.
  - API: a persistent `Package`, a map from part to `PartVersion`.
- **`ooxml-store`**
  - Owns: versions, `Txn`, read and write sets, commit and redo, history (undo, redo, scoped undo), snapshots.
  - API: `Store<D: DocModel>`.
  - Must not: know any format.
- **`ooxml-derive`**
  - Owns: `Memo<K, V>` keyed by content, `MonoidTree<T>`, budgets.
- **`ooxml-schema`**
  - Owns: generated views and constructors for the shared namespaces (relationships, `mc:`, DrawingML `a:`, charts `c:`).
  - Must not: contain hand-written code.
- **`ooxml-fonts`**
  - Owns: one sfnt reader (built on the fontations crates), the measured Office metric tables, the Word and PowerPoint lookup orders, substitution grades, EOT/MTX.
- **`ooxml-text`**
  - Owns: shaping, break opportunities, one break loop with Word-compatibility and DrawingML rule sets, hyphenation.
  - Must not: know about paragraphs or styles.
  - Reads faces through its own port (`Glyphs`: an advance, a kern pair, script metrics, the fonts epoch), so neither L1 crate depends on the other; the Word layout (L3) adapts `ooxml-fonts` to it (lane L1).
- **`ooxml-colour`, `ooxml-numfmt`, `ooxml-markdown`**
  - Own: theme colours and transforms; number formats, dates and locales; the Markdown dialect with ids.
- **`ooxml-drawingml`**
  - Owns: shapes, generated presets, fills, lines, effects, text bodies (model, inheritance, resolution).
  - Must not: write SVG.
- **`ooxml-chart`**
  - Owns: the chart model and its layout to a scene. The 10.6k-line chart layout class splits into axis, legend, plot, label and 3-D modules.
- **`ooxml-metafile`**
  - Owns: players for EMF, EMF+ and WMF that produce a scene, with Windows as the reference; DIB.
- **`ooxml-scene`**
  - Owns: the display list (paths, glyph runs, images, groups, clips, live-view ids). Two forms (lane P3c): the metafile players' typed nodes, and the DrawingML painter's `markup` -- elements in SVG's vocabulary with typed attribute values kept in their producer's order, because ooxml-common's string-built SVG fixes that order and the goldens are byte for byte.
- **`ooxml-render`**
  - Owns: the DrawingML painter -- drawables (a chart's, a slide's, a document's) lowered onto the scene under Word's or PowerPoint's drawing rules as data, text measured through `ooxml-fonts` (lane P3c; `ooxml-drawingml` and `ooxml-scene` share a layer, so the lowering sits above both) -- and the pieces a page painter calls directly (lane P3d): fills, outlines and effects at the page's dpi, an element's place on its page (`ShapeFrame`), Word's drawn arrowheads, a geometry as path data with its ends and cuts, symbol faces drawn as Unicode; the scene's typed SVG writer with stable ids and exact numbers; outline text.
  - Must not: re-parse SVG.
- **`ooxml-raster`** (lane P3d; beside `ooxml-render`, layer 2)
  - Owns: PNG -- the typed writer's SVG drawn by resvg and tiny-skia in process, with a font database built once from the files, folders and system fonts resvg-py is handed (`FontSet`) and shared by every drawing after.
  - Must not: write or rewrite SVG; the SVG it is given is the one handed out (decision 11).
  - Re-parses SVG in memory: the one exception to `ooxml-render`'s rule (decision 11).
- **`ooxml-word`**
  - Owns: generated `w:` views; the styles cascade; numbering and labels; fields; sections; one address resolver; the id ledger; the tracked-change policy; validation rules; Markdown in and out; the edit commands.
  - Must not: lay out pages.
- **`ooxml-word-layout`**
  - Owns: measure, paginate, field passes, geometry, paint, and layout facts.
  - Must not: edit.
- **`ooxml-slides`, `ooxml-slides-layout`**
  - Own: the deck model, one master/layout/slide inheritance resolver used by both render and edit, slide commands, fit and overflow facts.
- **`ooxml-sheets`**
  - Owns: the workbook model, a streaming pull reader, shared strings and styles, one A1 and formula tokenizer, regions, derive and query.
  - Must not: evaluate formulas.
- **`ooxml-tools`**
  - Owns: `ToolSpec`, `#[derive(ToolArgs)]` generating the strict-subset schema, intent rules, validation, `ToolResult` and `ToolError`, budgets and truncation, batch and read_batch, roles.
  - Must not: know about providers.
- **`ooxml-tools-*`**
  - Own: each format's tools as commands, with descriptions as Markdown files beside them (`include_str!`).
- **`ooxml-session`**
  - Owns: sessions (documents, blobs, outputs, datasets, refs per version, roles, views); a per-document scheduler (commit queue, derived and layout owner, coalescing); the memory budget; deadlines.
- **`ooxml-python`**
  - Owns: the PyO3 `Session` and `Document` handles; `call(name, json) -> (json, images)`; the specs export; the bridge from tracing to logging.
  - Must not: hold logic beyond conversion.

**What stays in Python** (the same seven distributions, as shims):
- the provider adapters (`to_anthropic`, `to_openai_responses`, `to_openai_chat`, results with images, the tool-search and defer plans, strict planning by provider limits);
- the `Toolbox` and `Session` façades, with today's signatures;
- assembly of `system_prompt`;
- `.pyi` stubs;
- thin library handles.

That is about 5k lines, down from about 19k, because the reply wording moves to Rust with the results.

---

## 4. The document model

**The tree (ADR-003).** Every XML part is an immutable lossless tree.
- **Green nodes** are `Arc`'d. Each holds its interned qualified name as written (plus the resolved namespace), its namespace declarations and attributes in order, and its children.
- **Children:** text is decoded. Comments and PIs are kept. CDATA becomes text (lxml's `strip_cdata`). Undefined DTD references stay as opaque nodes.
- **Child storage:** an element's one child is held inline; up to 64 children are one shared slice; wider elements (a body of 29k blocks, a sheet's rows) keep a persistent B-tree rope of 64-child leaves and 32-chunk branches (P0a).
- **Red cursors** give parent and position on demand.
- **Edits are path copies.** Spike 1 at 3k pages:

| Measurement at 3k pages | Flat children | Persistent children |
|---|---|---|
| Replace one paragraph | 705 KB and 1.3 ms per version | 9 KB and 16 µs |
| Insert 5 blocks | — | 128 KB and 0.6 ms |
| 100 mixed versions held | 70 MB | 6.5 MB |

- **Insert cost:** imbl's RRB vector copies more than needed on insert. The in-house rope (P0a) copies one leaf and the branches above it; at 29,183 children it takes 3.4 KB to insert five nodes (imbl: 124 KB) and 3.1 KB to replace one (imbl: 4.8 KB), and 200 splices take 0.7 ms (imbl: 23 ms).
- **Memory:** the naive tree is 5.7× the XML (113 MB for 20 MB). The target is ≤ 2.5×, reached by interning attribute values (most are short and repeated, like `w:val="24"`), hash-consing small identical subtrees as rowan's node cache does, and compact strings. P0a reached 2.07× (41 MB for 20 MB): short strings and element heads are shared, a single child is held inline, and small identical subtrees are hash-consed, all through fixed-size direct-mapped caches while a part is parsed (sharing is best-effort; a lookup costs the same whatever the input, and no table grows with the part).

**Packages.**
- A `PackageVersion` is a persistent map from part name to a `PartVersion`.
- A `PartVersion` is either `Raw(stored entry)` or `Tree(green root)`. A raw part is untouched: it is never parsed unless read, and it is written by raw copy, compressed bytes included.
- Relationships and content types are typed and persistent.
- Directory entries (names ending in `/`) are not parts: they are kept and copied raw, where `ooxml_edit` leaves them out of a save. Two entries that name one part are refused. A raw copy keeps an entry's local record byte for byte, so an unchanged package saves as it was read when its entries lie end to end (bytes between entries, which no Office writer leaves, are not kept).

**The write contract (ADR-004).**
- A touched part is written exactly as `ooxml_edit.xml.serialize` writes it today, i.e. libxml2's xmlsave:
  - the declaration `<?xml version='1.0' encoding='UTF-8' standalone='yes'?>`;
  - namespace declarations before attributes;
  - `& < > \r` escaped in text, and `& < > " \n \r \t` in attributes;
  - `<a/>` for an empty element;
  - no trailing newline.
- Spike 1 matched lxml byte for byte on 620 of 620 distinct parts from the repo's 80 test packages, including the 386 that lxml rewrites. It also matched after all four edits at 295 pages, and on the 3k round trip.
- **Recommendation: keep this contract permanently.** Document it as an algorithm in `ooxml-xml` so that no library defines it. It keeps every golden file and saved byte as they are, and Word accepts it.
- Keeping the original bytes of untouched elements inside touched parts would give smaller diffs but no fidelity gain. That stays a later option needing the user's approval (Q1).
- **Zip:** the oracle compares entries (name, order, metadata, uncompressed bytes), not compressed streams; those already differ between zlib and zlib-ng today. Raw-copied entries stay byte-identical.
- **UTF-16 parts** are decoded as libxml2 decodes them and written back as UTF-8. This is in phase 0's tests; the spike handled UTF-8 only. US-ASCII and ISO-8859-1 are read too; other declared encodings are refused.

**Typed views (ADR-005).**
- Generated views (`w::P`, `w::Tbl`, `w::RPr` and so on) give typed getters, enums with defaults, and constructors that insert children in schema order. They replace ooxml-edit's hand-kept child-order tables and `insert_in_order`.
- Hand-written domain layers sit on top: `Paragraph`, `Table`, `Cell`, `Section`, `Story`.
- Unknown content (extensions, `mc:AlternateContent`, custom XML) survives, because views never rebuild what they do not change.

**Ids.**
- Semantic ids (paraId/textId, wp14 ids, cNvPr, comment durable ids, bookmark names, revision ids) are read through views.
- One address resolver per format handles every form: `p:`, `t:X/c1,1`, `s:`, `bm:`, spans `p:A..t:B`, ranges `p:A@3:9`, `$ref`. It is a pure function from a snapshot to an address, or to a refusal carrying `valid_options`.
- The id ledger (every value any state had, plus deleted block ids with the change that removed them) moves into the version as a persistent set, so undo and redo never lose it.
- Its export stays the same `docx-ids.1.` string, so stored states keep working across reopen.
- Stamping rules and the deterministic value sequence are unchanged.

**Tracked changes are a strategy, not branches.**
- Commands write through an `EditSink` of primitives: insert text, delete a range, set a property, insert, delete or move a block, change table structure.
- A `RevisionPolicy` (`Untracked` or `Tracked {author, date, rsid}`) turns those primitives into `w:ins`, `w:del`, `w:rPrChange`, `w:pPrChange`, moves and table marks.
- Accept and reject are commands. The final view (Accept All) is a derived view.
- Each command is written once.

**Undo.**
- `History` is a persistent list of (seq, version, change summary, caller).
- The seq is monotonic and never reused. Undo makes a new seq whose content is an earlier version's, which keeps the commit-order rule for readers.
- Scoped undo inverts one step's change set, and refuses `Entangled` as today.
- Depth is 50, at about 10 KB per step.

**Other parts.**
- Word: styles, numbering, settings, headers and footers, notes, comments (with `commentsExtended`/`commentsIds`), charts with their embedded workbooks (each a nested `PackageVersion`), media (raw).
- Slides: masters, layouts, notes.
- Sheets: sheets, shared strings, styles, tables, pivot caches, calcChain.
- Commands keep cross-part invariants (relationships, content types, comment anchors), and validation checks them.

---

## 5. Edits as transactions (command pattern)

```rust
trait Command: Send {
    type Out;
    fn validate(&self, s: &Snapshot) -> Result<(), ToolError>;        // arguments against the document
    fn prepare(&self, tx: &mut Txn) -> Result<Self::Out, ToolError>;  // on a snapshot; reads recorded, writes staged
}
```

**The transaction.**
- A `Txn` wraps a snapshot and a private working version, which is free to make because versions are persistent.
- It records a **read set** of typed keys: block ids, parts, styles, numbering instances and abstract definitions, a list chain, bookmark names, field kinds, the ledger, sections.
- It produces a **change set**: tree patches per part, ledger additions, relationship and content-type changes, side outputs.

**Commit.**
- Take the per-document commit lock.
- Compare the read set with the write sets committed since the snapshot.
- If they are disjoint, apply the patches to the head (each rebased through the patches committed since its snapshot, which moves its path past theirs, and checked by node pointer: the nodes it removes must be the ones there) and publish.
- Otherwise redo `prepare` on the new head; after 3 tries, run with the lock held.
- Spike 2 measured the critical section at 3–5 µs with 24 writers.

**What an edit reads** (learned in lanes J and K, enforced by the random multi-writer oracle):
- the target blocks, plus neighbours where an ordering rule applies;
- the whole list chain for numbering;
- the styles and numbering parts when it resolves them;
- bookmarks and fields that point into or out of its range;
- the relationships it adds;
- the ledger. Allocation goes through the ledger, but the ledger is append-only and merged at commit, so it is not a conflict point.

**Results** come from the change set and the snapshot after commit: created, renamed and removed ids, `touched`, the validation delta (derived facts of the changed blocks), and the pages reflowed. Nothing re-reads the whole document.

**Batch.**
- One transaction: ops apply in order on the working version.
- A failing op returns to the savepoint (a pointer) unless the batch is `atomic`.
- It is one history step, and costs the sum of its ops.

**Validation.**
- Facts per block are memoised by content.
- Whole-document rules are kept incrementally (§7).
- The full validation is a test oracle only.

**As built (lane P2c7: reads by condition).**
- A transaction may read by condition (`ooxml_store::ReadPredicate`, `Txn::read_where`) as well as by key: at commit, the keys written since its snapshot (the journal's change sets, `DocModel::written`) conflict only where the condition holds on the head. The keys that met it on the snapshot and that the outcome used must be read by key too, so one that stops meeting it is a conflict through them. The history keeps such a read as a read of every key, so undo and redo find steps entangled as before.
- `replace_text` within paragraphs reads the paragraphs that hold a match (`HoldsMatch`: a written block whose paragraphs match on the head, or a story part written whole) and its matches by key; across paragraphs it still reads the whole document. Writers editing other paragraphs no longer make it conflict and run exclusively.
- `touched` cuts the changed stretch of the body at the blocks both versions hold (the same allocation, in order) and compares only the pieces between them, so an edit that rewrites scattered blocks (a removed style's uses repointed) no longer diffs the whole body.
- A part's paragraph texts (`PartTexts`) are kept in chunks of up to 64 lines, shared by every version that holds the same paragraphs there in order: a find after an edit reads the changed chunks and a pointer per other chunk, not a line per paragraph.

---

## 6. Concurrency model

**Threads only, in the caller's process.** All Rust work runs with the interpreter lock released (`py.detach`).
- Spike 2: four Python threads on one document used 3.9 cores (one thread 1.0, two threads 1.99).
- With the lock held: 1.23 cores, and a check's p99 of 200 ms against 65 ms.
- There are no processes and no pickles, so daemonic Celery children work as they are, and the four runner kinds go away.

**Readers never wait.**
- The head version is an `ArcSwap`; a check loads it without a lock.
- A check that must include its own commit waits for the first derived snapshot at or after that commit's seq (lane O's rule).

**Writers** prepare in parallel and commit as in §5.
- Locks are per document.
- Calls spanning several documents take locks in document-id order.

**Scheduling.**
- One global rayon pool, sized to the CPUs.
- A per-document owner is a logical serial task on that pool, not an OS thread. It takes new heads and runs the derived engine and layout.
- Commits are coalesced: all commits that land during a pass are taken by the next pass, so CPU per document does not grow with the number of writers.
- Layout runs eagerly while the document was checked recently (today's 30 s, now a configuration value), lazily otherwise.
- Measuring changed blocks is parallel within a pass.

**Memory.**
- A session `MemoryBudget` with per-document accounting.
- LRU eviction of derived data and page paint, which can always be recomputed.
- Versions are retained only by history depth and by readers' `Arc`s.
- As built (lane L4b): `ooxml_session::MemoryBudget` gives each document an account within one limit for what can be made again (page paint, the layouts a host keeps for readers), and counts beside it, never refused, what the versions in use hold (the model's and layout's memos). Memos are swept once a version is laid out: an entry not used since the last sweep that no version holds goes, so they stay at the newest version's working set. A version's own cost (`LaidVersion::weight`) is what it alone holds: its containers, and the measures, pages and blocks no other version or memo holds.
- As built (lane L4e): a measure keeps a piece per character, 104 bytes (was 208): its lengths as `CompactUnits` (inline `i64`/`u32` parts, the exact value boxed where they do not fit), its format shared by its run's pieces, its kern key derived when kerning. Measures at 350 / 2,988 pages: 353 -> 149 MB / 483 -> 311 MB; at 3k the rest is mostly line heights (192 bytes a line), geometries (352 bytes a paragraph) and tables' cell flows, all of exact 32-byte lengths, which the same compact form would about halve.
- As built (lane L4f): every exact length kept in quantity is one type, `ooxml_base::Compact<L>` (16 bytes, the same inline-or-boxed form; `CompactUnits` and `CompactPx` are it over `Units` and `Px`), read back for exact arithmetic. Lines' heights (96 bytes, were 192), cells' stacked lines (96, were 176, with no spare room) and painted spans' pens keep their lengths so; geometries are kept once per measurer (a document has a handful), and a measured cell shares the model's resolved cell (`ResolvedRow.cells` holds `Arc`s) rather than copying it. Measures at 350 / 2,988 pages: 149 -> 121 MB / 311 -> 192 MB; 350 pages painted: 109 -> 77 MB. What is left at 3k is mostly the pieces (99 MB), cell flows and blocks (17 and 12 MB, inline lengths and empty drawing lists) and measured paragraphs (10 MB).

**Deadlines.**
- Cooperative cancellation: `Cancel` is checked in every loop over blocks, pages and records, giving `timeout` results that change nothing.
- Every loop is bounded by input limits (§11), because a runaway cannot be killed the way a worker process can (Q8).

---

## 7. Derived data, incremental (ADR-008)

**The approach.** Every derived fact is a function of a version.
- **Per-block facts** are memoised under (block digest, context key). Green nodes are immutable, so a changed block simply has a new key; nothing is invalidated by hand.
- **Order-dependent facts** are kept as monoid aggregates in a segment or finger tree per sequence, with O(log n) updates and queries. Examples: list labels (counter transforms compose associatively), field and page numbering, heading numbers, the block-to-page index.
- **Whole-document facts** (validation counts, ids in use, styles in use) are counted multisets updated from change sets.
- **Layout** is its own staged engine (§8).

**Spike 3: list labels.** Setup:
- 70,273 blocks, 8,784 list items in 362 lists.
- Edits near block 5,000; labels then read for a late chapter, and again at the edit.
- Best of 3. Every variant was equal to a full recompute after every edit. A full recompute takes 0.83 ms.

| Edit | salsa (chunk queries, early cutoff) | Hand-rolled chunks | Hand-rolled monoid tree |
|---|---|---|---|
| Text only, no list change | 248 µs | 9 µs | 12 µs |
| Change an item's level, read late | 12,423 µs | 3,214 µs | 19 µs |
| Make a paragraph a list item, read late | 7,642 µs | 3,319 µs | 183 µs |
| Read at the edit | 532–788 µs | 4 µs | 11 µs |
| Build | 12.7 ms | 10.7 ms | 1.2 ms |
| Memory | 22.3 MB | 21.6 MB | 1.5 MB |
| Lines of code | ~65 | ~70 | ~150 |

What this shows about salsa:
- Its code is concise.
- The chunk-walk idiom it encourages is slower than a full recompute for prefix-dependent facts.
- Setting an input cancels queries in flight on other handles, which conflicts with "checks never wait".
- It hides cost and memory behind macros.

**Decision:** a small in-house `ooxml-derive` with two primitives (a content-keyed memo with budgets, and a monoid tree). Each fact declares its dependencies and ships two things:
- a from-scratch oracle, used in property tests;
- a statement of its kind: per block, aggregate, or whole document.

---

## 8. Layout and rendering engines

```
version ──► resolve (styles cascade, numbering, fields threading; memo per block)
        ──► measure (shape, break lines per paragraph; cells; memo per (block, context))
        ──► paginate (compact arrays; restart at the first changed page; stop at convergence;
            field passes until PAGE/NUMPAGES/PAGEREF/TOC settle)
        ──► geometry (per page, lazy: lines, tables, floats, notes, headers and footers)
        ──► paint (Scene display list per page, lazy, keyed by content)
        ──► adapters: SVG (typed writer), PNG (resvg in process, after SVG), live-view ids, outlines
```

- **Invariant: relayout equals full layout,** byte for byte in SVG. This is a property test (docx2svg's `test_relayout`, kept), with fixed seeds on PRs and many seeds nightly.
- **One shared text engine** (`ooxml-fonts` plus `ooxml-text`):
  - one sfnt reader and Office's measured tables;
  - Word's advances in 1/4096 pt, with the legacy kern charged to the left glyph;
  - DrawingML's GPOS kerning and grid rounding as rule sets;
  - one break loop, with Word's compatibility rules as data;
  - hyphenation;
  - substitution grades reported in coverage (rule 8).
- **Caches are keyed by content:** measures by (block digest, resolved format, width, fonts epoch, locale), page paint by page digest. There are no pickles.
- **Exact arithmetic:** layout uses integer rationals and fixed point, as docx2svg does. No floating point enters a pagination decision.
- **Charts and metafiles** lower to a `Scene`, never to SVG strings. EMF/WMF rendering follows Windows.
- **resvg, usvg and fontdb** are linked at the version resvg-py ships today, so PNGs stay the same (verified in lane P3d: every resvg-py call of the corpus' pages and decks drawn again pixel for pixel).
- **SVG is an output in its own right** (the live view shows it, the tools return it): the typed writer stays first-class, byte-identical and fast on its own, `data-*` ids included. PNG is one more step after it, never a replacement, and the SVG is never trimmed for usvg's sake: what usvg needs differently is changed on the way in (decision 11).
- **Today's `crates/word` engine** (paginate, line breaking and field passes, which matched docx2svg exactly) seeds `ooxml-word-layout`. It is refactored to named types, without tuples, `allow(too_many_arguments)` or JSON input.

A detailed design of the layout engine is a later lane. These boundaries fix its inputs (resolved, measured blocks of a version) and its outputs (pages, facts, scenes).

---

## 9. The tool contract and the Python shim

**One source of truth per tool, in Rust:**

```rust
/// word_set_text …  (description: tools/word/set_text.md)
#[derive(ToolArgs)]
#[tool(name = "word_set_text", group = "core", role = "edit", mutates, batchable)]
struct SetText {
    doc: DocId,
    #[arg(address(kinds = [Block, Cell, Section, Bookmark, Span]))] target: Address,
    #[arg(max_len = 200_000)] text: String,
    #[arg(empty_is_absent)] paragraph_style: Option<StyleName>,
}
```

**What the derive generates:**
- the canonical strict-subset schema, following today's `schema.py` rules;
- validation;
- intent rules declared once (`empty_is_absent`, `single_as_list`, `map_as_pairs`, value and argument aliases with a note, kind-neutral fields), replacing today's hidden keywords and the acceptance code scattered across call sites;
- the schema of the result.

**As built (lane P0c, `crates/ooxml-tools`, `crates/ooxml-tools-derive`).**
- A field's type gives its schema node (`String`, `i64`, `f64`, `bool`, `Option<T>` optional or, as an item, `orNull`, `Vec<T>`, `IntOrStr<MIN>` for `orInteger`, `NumOrStr` for `orNumber`, a free-form `Map`, a nested `#[derive(ToolArgs)]` struct, a `#[derive(ArgEnum)]` enum); its doc comment is its description.
- `#[arg(...)]` declares bounds (`minimum`, `maximum`, `min_length`, `max_length`, `min_items`, `max_items`, `format`), the intent rules (`empty_is_absent`, `single_as_list`, `map_as_pairs`, `value_alias(a, b)`, `alias = "..."`, `default = ...` on a required field, `kinds(...)`, `kind_neutral(...)`, `kind_value(...)`, `kind_description(...)`) and what its strings hold (`refs`, `datasets`, `sources`, `facts`); the paths today's `Tool` lists (`items[].target`) follow from the structure.
- `#[tool(name, description = include_str!("tool.md"), group, mutates, strict, batchable, read_batchable, retry_key, role, priority, route, documents(...), exactly_one(...))]`.
- Address types (`Address`, `DocId`, `StyleName` above) come with the format tool crates; until then an address is a `String`.
- A refusal from the contract carries `valid_options` where there is a closed set, and otherwise a `try_call`: the same call with a value at the field that would pass, `fill` naming the fields whose value only shows the kind wanted. Today's Python sends no way out for a missing, mistyped or out-of-range argument; that difference is the rule-2 change, and the only one in the replies.

**Descriptions** are Markdown files. A test counts characters and estimated tokens per tool and per group against budgets; the main session's online count stays the authority.

**Results.**
- `ToolResult<T>` is serialised to JSON in Rust, within the reply budget. Nothing is cut silently: what did not fit is listed with the call that would ask for it (rule 4).
- `touched`, timing and budget stay application-only fields.

**Refusals carry the way out** (rule 2). `not_found`, `invalid_arguments`, `ambiguous` and `refused` must carry `valid_options` or a `try_call`. A test walks every refusal site to check this.

**No-ops** return `changed: false` with a note (rule 3).

**Cost of crossing the boundary (spike 2):**

| Call | Cost |
|---|---|
| Empty call | 0.07 µs |
| Empty call, lock released | 0.15 µs |
| JSON call (serde) | 4.1 µs |
| JSON call, including Python's `json` | 12.1 µs |
| Typed call | 2.2 µs |

All are under 0.1 % of the edit budgets.

**Decision: JSON in and out for tool calls.**
- One schema, which is what the providers send anyway, and calls can be logged and replayed.
- Images travel as `bytes` beside the JSON.
- Typed handles serve the library API.

**The Python shim.**
- `toolbox.dispatch(...)` calls `session.call(name, json)` with the lock released.
- The adapters (kept in Python, as briefed) read the specs exported by `ooxml_core.tool_specs(kind)`.
- `definitions`, `render_results`, `system_prompt`, roles and views stay in Python.
- The clock becomes a `Clock` port in Rust.

**As built (lane P0d, `crates/ooxml-session`, `crates/ooxml-tools-word`, `crates/ooxml-python-session`).**
- `Session` holds documents by `DocId` (`d1`, never given twice), each an `ooxml-store` `Store<PackageDoc>` over its OPC package: `open(bytes, kind, name)`, `save`, `close` (the session lets go at once; a reader's snapshot stays valid until dropped), `snapshot`, and `update(doc, parts)`, the bridge's way in (section 13): a version's touched parts as bytes, XML parsed on a snapshot, committed as one step through the store (put, remove, and undo's exact restore of a part: `PartChange`).
- `call(name, arguments, context)`: the arguments taken as meant (`ToolSpec::accept`), the document found by the spec's first document argument, the kind rules applied, the handler run on the head snapshot; never panics, never fails: refusals carry their way out (an unknown tool, a document not open, a kind the tool does not serve yet).
- The `Registry` makes each `ToolSpec` once and maps tools to the kinds they serve; `ReadHandler<T>` makes a reading tool of a `#[derive(ToolArgs)]` struct and a function of a version. `ooxml-tools-word` holds `describe` over package parts.
- `CallContext {caller, cancel, deadline}`: a call stops at a safe point (dispatch, each part `describe` reads, each part an update parses) when its `CancelToken` is set, the session's own `Cancel` is, or the deadline read from the session's `Clock` has passed, with `timeout` and no change.
- One native module: `crates/python`'s `ooxml_core` registers `Session`, `CallContext`, `CancelToken` and `tool_specs(kind)` from `ooxml-python-session` beside the prototype's functions. That crate is this design's `ooxml-python`; the prototype's package holds the name until it is retired, when it becomes the module itself. Every method copies its Python values in, then releases the lock (`py.detach`); a panic in `call` is an `internal` reply.
- Python's side for docx-agent is `docx_agent.rust.session.RustTools`: `Tool`s from the exported specs and `RustResult`s from the replies, rendered by the unchanged adapters. Nothing uses it by default.

**As built (lane P1c: docx-agent's reads served by Rust).**
- The bridge's update (`Session::update`, `commit`) parses a touched part and diffs it with the part's tree in the version before (`ooxml_xml::diff`), applying the differences to the old tree by path copying: the new version shares every unchanged subtree, so what is kept by pointer (block ids, a part's facts, a layout's measures) is found again after a bridge update as after an edit made in Rust.
- `commit` returns the version it made, held (`Opened`; `ooxml_core.Version`); a `CallContext` names the versions a call reads (`at`), so a host that owns the document reads exactly the version its caller holds, whatever was committed since, and a `Placer` (a port of `ooxml-tools-word`) through which `word_inspect`'s `layout` asks where blocks are laid out, until the session holds the layout.
- A reply says, for the application only, what of it is not yet given as the Python tool gives it (`Done::unported`: a chart's or SmartArt diagram's summary until P3, Accept All's cell ends); the host answers such a call its old way meanwhile.
- docx-agent's reading tools (`describe`, `word_read`, `word_inspect`, `find_text`, and `read_batch`'s ops through them) are answered this way by default (`docx_agent.rust.reads`): each committed version's parts whose bytes differ handed over, the call run with the interpreter lock released, Python adding only what it holds (`describe`'s pages, tracking, problems; the placements). The Python path stays as the fallback, counted by tool and reason (`read_fallbacks{tool,reason}`), never silent.
- A story part's lookups (`ooxml_word_model::docx::facts::PartFacts`: content controls, revision records, drawing frames, hyperlink anchors) are found in one walk and kept per top-level block: a later version walks only the blocks it does not share with the one before.
- A drawing's chart and SmartArt diagram are read in Rust (lane R1): `ooxml_chart::data` is ooxml-edit's `chart_model` as a second view beside the render model, over the same format-code walk (`read::cache`); `ooxml_drawingml::diagram` walks a diagram's data model as ooxml-edit's `DiagramModel` does (iteratively, nesting bounded by `MAX_DEPTH` where Python's recursion stops); `ooxml_word_model::docx::graphics` finds the parts through the frame's part's relationships. `word_read`'s comment summaries and `word_inspect`'s `chart` and `diagram` JSON are docx-agent's; `unported` no longer names them.
- `find_text`'s `regex` is Python's (`ooxml_tools::pyre`, over `fancy-regex`): a pattern is written in its syntax with the same meaning, or refused with why where the meaning could differ.

**As built (lane P2c, part 1: the session owns Word documents).**
- `Session::open` of a Word document holds it in its editor (`ooxml-word-edit`'s `Editor` over `WordDoc`): its ids and the problems it opened with are read once; other kinds stay packages until their phases. `Opened::word` gives the version as the edit core keeps it.
- An editing tool is `EditHandler<T>` in the registry: `fn(&T, &Editing) -> ToolResult` over the document's editor, the caller, the version it was opened as (for `this_session`) and the tracking mode the session keeps per document (`word_set_tracking`'s handler hands it back, `EditHandler::tracking`); each edit is written under that mode as its `RevisionPolicy`, dated by the session's clock to the minute. `word_set_tracking`, `word_changes` and `word_comments` are registered. A changing call's reply carries the head version it left.
- History: `Session::undo`/`redo` as docx-agent's session does them (the latest calls; a caller's own latest only while it is the document's latest, else `Entangled` naming the later changes and who made them). In the store, an undo's or redo's change set is kept in a journal with the commits' (`ChangeSet`, an exact undo's inverses worked out only when asked), so a scoped undo's inverses move past the later steps (`Journal::rebase`) and commits move past undos; an undo is exact when the head holds the step's content, whatever its number. `DocModel::carry` keeps what no step takes back (the id ledger and renames: an id given is never given again), `DocModel::kept` names the keys that entangle nothing (id sequences, annotation numbering). A patch removes a node that is the same or equal (`Node::removable_as`): an inverse made again on another version builds equal nodes.
- The save gate: `Session::save_checked` refuses a Word document with problems it did not open with (`SessionError::NewProblems`); `Session::validation` is what it compares.
- The bridge: `Session::update` reads the whole document (an exclusive commit, section 13 step 5) and adds the id values the parts bring to the ledger; `Session::bridge(doc, tool, parts)` is the same commit counted in `bridge_calls{tool}` (`Session::bridge_calls`). Python: `ooxml_core.Session.undo`, `redo`, `validation`, `save(checked=)`, `bridge`, `bridge_calls`.
- Retired ids (lane P2c10): each Word document keeps, in the session, the block ids and drawing numbers its versions removed with the step that removed each (`Session::retired`: the version, its caller and its tool), noted as each version is made -- a served call's edits (`Edited::ids`, `ooxml_word_edit::ids_delta` over the subtrees the change does not share), the bridge's update, an undo or redo (`HistoryOutcome::previous`) -- not read back from the store's steps, which keep only the undo window. An undone deletion's ids are back; a redone one's are retired again by the step that first removed them. Ids a saved state retired are given by the host (`Session::seed_retired`). docx-agent's deleted-id errors and its id state read this record for a document the session owns (`Document.retired_ids`), the session's versions numbered as the tool layer numbered them. Python: `ooxml_core.Session.retired`, `seed_retired`.

**As built (lane P2c3: the switch).**
- docx-agent's Word documents in a toolbox session are owned by the process's Rust session from the moment they join it: ooxml-edit's `DocumentFormat.owner` (docx-agent's `docx_agent.rust.owner`) takes each version the tool layer's `Document` makes (`committed`, at each commit), makes each undo and redo it makes with the same caller (`followed`), and gives the saved bytes (`save`). The Python `Document` is the bridge's working copy, kept at the session's head.
- A tool with a Rust editing handler runs in the session (`docx_agent.rust.edits`); the parts it changed come back as `Opened::changed_since` (`ooxml_core.Version.changed_since`) and are written into the working copy as the call's step. A call the handler answers `Done::unported`, a refusal (its words stay today's), an op of a `batch`, a call prepared on a draft, values written by reference and an address naming a renamed or deleted id run the Python handler, counted in `edit_fallbacks{tool,reason}`.
- Every other tool runs on the working copy and its version is committed through `Session::bridge` as that tool's and caller's step: one working-copy step is one session step, so the session's own-undo rule finds the same steps. The working copy's history still words an undo's refusal; a disagreement between the two is logged, counted and brought back through the bridge.

**As built (lane P2c4: the session is the only owner).**
- The Rust session's head is the document; the tool layer keeps no working copy. An owned document's entry is an `OwnedEntry` (`ooxml_edit.tools.owned`): its version, steps, undo and redo, saved bytes and ids are the owner's (`DocumentFormat.owner`, docx-agent's `docx_agent.rust.owner.RustOwner`). `OwnedHistory` numbers the states as `History` does (undo and redo back to the number of the state they bring back) over the session's steps; an undo the session does not make as the numbers say is an error (`OwnerDisagrees`).
- A tool with a Rust editing handler runs in the session ahead of docx-agent's handler and wrapper (`docx_agent.rust.edits.SERVED`): the session's version is the call's step, its reply's application-only `touched` (made in `EditHandler::run` from the change itself, `ooxml_word_edit::touched`) the result's, and the checks read `Session::validation`. The Python `Document` is not read.
- The Python `Document` is a view made current when Python reads it (a bridged tool, a Python-only check or read): the parts changed since (`Opened::changed_since`), the ids given and renames made since (`Opened::ids_since`, from the word pool, no part read), numbered as the entry (`History.follow`). A bridged tool's change is the view's own batch, committed by `Session::bridge` as the step of that tool and caller.
- Reads of an owned document find the session's version by the entry's number (`Mirror.versions`), without the view; an address naming an id is left to the session, whose refusal goes to the Python path.
- Owned documents take no block drafts (section 5's transactions replace them).

**As built (lane P2c6: served calls side by side).**
- A call the session serves runs without the tool layer's document lock (`ooxml_edit.tools.served`, the format's `DocumentFormat.serve`: docx-agent's `rust.edits.serve_owned`). It holds the document's gate (`ooxml_edit.tools.gate.Gate`) shared, beside the other served calls, while the session prepares its edit on a snapshot and commits it (section 5). Every holder of the document's lock holds the gate alone (`GatedLock`): a bridged tool's step, an undo or redo, the application's own changes; so a bridge never commits over a concurrent Rust edit, and is never rebased.
- `Session::edit` (Python `Session.edit`) answers a call with the versions its own edits made (`CallMade`: its first's number, its last held), recorded per call by the handlers' editor (`ooxml_tools_word::CallEditor`); a changing call's reply version is its last. `OwnedHistory.served` numbers steps in the order the session made them (a call that ends first waits, under the history's own short lock, for the steps before its own), and counts each as the dispatcher counts a change under the lock: its parts, its owner, its refs, the committed version for reads.
- The checks of a served call run after it, with nothing held: the session's validation of the version the call made (`Session.validation(doc, at=version)`). A call whose checks compare layouts (Python lays them out) is served under the lock instead.
- Finds read each story part's paragraph texts kept per version (`ooxml_word_model::docx::PartTexts`, `StoryPart::texts`): a version takes the text of every paragraph it shares with the last version that read them, so `find_text`, `replace_text` and the edit core's finds walk only the paragraphs that changed.

**As built (lane P2c8: the ported tools served, group 1).**
- `word_insert_text` and `word_delete` are Rust tools (`ooxml-tools-word`'s `text`), their definitions today's byte for byte (a placing tool's items take the places its own fields do: `Places::in_items`, `#[tool(place_items = "items")]`). docx-agent's `SERVED` holds them beside `word_set_text` and `replace_text`.
- A place found as the one place a text occurs is found on the transaction's own version (`ooxml_word_edit::InsertBeside`, `DeletePlaced`, `TextPlace::Once`), read by condition as `replace_text`'s finds are; the items of one call run in turn in one atomic batch, so each finds the text the ones before left.
- Whole blocks go through `DeleteBlocks`: the outline is checked first (a heading whose content stays is refused, with its range and `force` as the ways out, `EditError::Argument`'s options); it reads the styles part, and the whole document only when a heading it deletes leaves blocks under another. The file's original text the blocks held is reported as `word_edit_table`'s deletions report it (`Watch::block`).
- `word_set_text` writes every item through `SetStyledText`: styled text, a paragraph style, a cell in lines and French spacing are the session's; only values or addresses by reference stay with the host.
- Still the host's: a find `within` a block or span, a picture (`d:`), a heading's range without its end, a ref name the tool layer would fold.

**As built (lane P2c13: relayout after served edits).**
- A served edit starts the relayout on commit (lane O) as a Python edit does: without the lock, its checks' marker (`rust.edits.Made`) carries the call's engine (`Engine.of` over the toolbox's pool), used without touching the view; under the lock, the call's engine is engaged on the view and the checks' relayout lets it go.

**As built (lane P2c13: served edits that read the pages).**
- `word_fields`, `word_headers_footers` and `word_template` are served: `Session.edit` takes the document's page engine (`layout=`, a `WordPages`, as `Session.call` does), the one docx-agent's in-process Rust layouts of the document use (`docx_agent.pages.word_pages`, by lineage and layout options); the edit core lays the versions out through it (`FieldLayout`: a field's result, a header's page number, the upgrade's reflow). Options the Rust layout does not take (custom measurement, outlined text) leave the call to the host.
- Their work ahead -- a layout in Python with the lock let go -- is not done where the session is asked first (`rust.edits.will_serve`); a call the session then leaves to the host lays out under the lock. `word_fields` and `word_template` declare work ahead, so the session serves them under the lock; `word_headers_footers` beside the others.

**As built (lane P4d7: calls that name other documents).**
- A tool's declared documents rule (`ooxml_tools::documents`, `named_at`) names the call's own document first; the session pins every other open document it names like the call's own (the version the context or batch holds, else the head; no lock: versions are immutable) and hands them over in `Call::others`, which read handlers see as `Version::others` and editing handlers as `Editing::others` (`Others::get` by id). A named document that is not open is refused at the field naming it (`reference.doc`). `word_compare_blocks` is the first tool to read one.

**As built (lane P4f1, part 1: `word_copy_from`).**
- `ooxml_word_edit::CopyBlocks` copies blocks of another open document (`CopySource::Other`, the version the call pins) or of this one as the call found it (`CopySource::Own`) as docx-agent's importer does: the source blocks selected (`_source_blocks`; another document's ids as they are, one it does not know left to the host, whose session may have renamed it), each made the source's current view, its styles brought by name under `use_destination` (one the destination lacks imported under a free id with what it is based on), placed where `crate::placing` puts new blocks (shared with insert_markdown), and stamped keeping the copy's own ids where the document never had them (`EditSink::keep_or_draw`, under the seed's key as a draw is). An edit of this document's source blocks conflicts with the copy (their keys are read).
- Still the host's (`unported`): tracked copies, `replace:`, the other style policies and style maps, and copies holding lists, relationships, notes, comments, bookmarks, drawings or controls (the next parts of the lane).

**As built (lane P5a, part a: `check` in the session).**
- `check` is a Rust tool (`ooxml-tools-word`'s `check`, its definition today's byte for byte: one schema for decks and documents, `documents("doc", "copies[].reference.doc")` -- a declared document path may step through an array, `[]`, in `ooxml_tools::documents` as in ooxml-edit's `documents_named`). It answers the facts read from the document alone: `validate` (the version's problems against those the document opened with, the first 50 listed) and `fields` (`ooxml_word_edit::field_facts`: counts per kind, empty tables of contents, references to missing bookmarks), each part's seconds in `timing`. Everything else -- `reflow`, `layout`, `pages`, `range`, `min_size`, `styles`, copies, a scale, a deck's arguments, the application's `min_text_size` -- is `unported` and the host's for now.
- A reading tool's `Version` carries `owned` (`ooxml_tools_word::Owned`: the version as the session's editor holds it, and the problems the document opened with) where the session owns the document; a tool that needs it leaves a package it only reads to the host.
- The save gate's comparison of an owned document is the session's (ooxml-edit's `DocumentFormat.validation`, docx-agent's `rust.owner.session_validation`, outside a batch): `check`'s `validation`, saving and a changing call's checks no longer make the Python view current to validate it.
- docx-agent asks the session first (`rust.reads.serve_check`), counted in `read_fallbacks{check,reason}` where it does not answer.

**Library API.**
- `docx_agent.Document`, `pptx_agent` and `xlsx_agent` become handles. Methods that take and return plain values keep their signatures.
- APIs that expose lxml (`.element`, `package.tree()`, `History`) warn for two releases and are then removed, each with a CHANGELOG note naming the replacement.
- `.pyi` stubs are generated and checked with mypy.

**Wheels (ADR-017).**
- One abi3-py310 wheel per platform: manylinux_2_28 x86_64 and aarch64, macOS x86_64 and arm64, Windows x86_64.
- Built with maturin and attached to GitHub Releases.
- The seven Python packages become pure Python and pin `ooxml-core==` exactly.
- Plus a `cp315t` wheel per platform, a build of its own because abi3 does not load on 3.15t (Q2: kept).

---

## 10. Errors, refusals and observability

**Errors.**
- Each crate's `thiserror` enum maps to the format's edit and read errors, then to `ToolError` with the 11 codes.
- Internal errors carry a trace id. Arguments are logged only when `log_arguments` is on.

**Warnings and facts** are typed values in results, never log lines. Rule 7 is served by layout facts.

**Tracing.**
- Spans per call and stage: validate, prepare, commit, wait, derive, layout, facts, serialise.
- `data.timing` is built from those spans: today's parts plus `commit_wait` and `redo`.
- A bridge feeds Python `logging` (pyo3-log).

**Counters per session** replace today's `STATS` dicts and are read with `session.stats()`: commits, conflicts, redos, exclusive fallbacks, bridge calls per tool, cache hit rates, evictions, cancellations.

---

## 11. Untrusted input

**Limits** are configuration, with defaults taken from ooxml-edit's limits:
- **Zip:** entry count, total uncompressed size, ratio per entry. Nested packages count against the same budget.
- **XML:** depth ≤ 256, plus caps on attributes, name length, text size and nodes per part.
- **DTDs:** never expanded. A document type with an external id stays opaque, and so do the entity references in text that it allows (as lxml keeps them). An internal DTD subset, and an entity reference in an attribute value, are refused: lxml writes them back in forms of libxml2's own that the core does not reproduce, and OOXML parts have none. External references are never fetched.
- **Images:** dimensions and decoded size are checked before decoding.
- **Metafiles:** record caps and bounds checks.
- **Formulas:** length and token caps.
- **Calls:** a deadline and cancellation per call.

**Fuzz targets** (cargo-fuzz) run nightly, and for 4 minutes per target on PRs that touch one. Corpora are seeded from the fixtures, and every crash becomes a test. The targets:
- the XML parser and writer (round-trip property);
- zip and OPC;
- each format's open;
- the Word address parser;
- Markdown;
- the metafile players;
- numfmt;
- the formula tokenizer;
- sfnt;
- the tool argument validator.

No parser contains unsafe code.

---

## 12. Testing strategy

1. **Property and unit tests per crate:**
   - the XML writer against lxml (spike 1's corpus test becomes a test asset);
   - store serialisability, and undo and redo as inverses, with loom on the commit;
   - for every derived fact, incremental equals from scratch;
   - relayout equals full layout.
2. **The differential oracle** (`ooxml-oracle` plus pytest). Every ported tool runs on Rust and on a frozen copy of today's Python, pinned as a `legacy` extra. The results' JSON (minus timing) and the saved part bytes must be equal. Sequences come from:
   - made-up documents (the harness generators);
   - the golden transcripts;
   - the random multi-writer test (lane J's block_stress, generalised): the final state and every version must equal a serial replay in commit order. Fixed seeds run on PRs, many seeds nightly.
   - Locally, lane E's replay (18/18) stays a gate and is never committed.
3. **Office-measured fixtures:**
   - about 76 docx2svg and about 25 pptx2svg measurement JSON files move with their crates;
   - each crate's test README states the fidelity rules: measured, not transcribed; the render's final view is Accept All; EMF follows Windows; screenshots win over PDF;
   - the table generators become xtask commands with pinned inputs.
4. **Golden replays:**
   - transcripts and VRT snapshots (docx2svg 135, pptx2svg 52, ooxml-common 164) compared byte for byte;
   - PNGs compared by SSIM where resvg versions differ.
5. **Performance budgets:**
   - instruction counts (iai-callgrind on Linux) for open, edits, check, fields, render and save at 295 and 3k pages, with ±10 % gates in a budget file;
   - wall-clock benches nightly on a fixed runner, never as PR gates;
   - memory per version and per document asserted with a counting allocator.
6. **Tool rules:** each of the 8 rules is tested per tool family, carried over from today's suites.
7. **One integration-test program per crate:**
   - integration tests go in `tests/integration/` as a module of `main.rs`, not as a new file in `tests/`: every file there is a program of its own, built and linked again after each change to the crate or one below it;
   - a crate whose tests are one file (`tests/<name>.rs`) moves it into `tests/integration/` when it gets a second;
   - helpers the modules share live once in the program (`common.rs` or the crate's support module), not as a copy per module;
   - a test that needs its own process stays a file of its own (today only ooxml-store's `tests/loom.rs`, built with `RUSTFLAGS="--cfg loom"`);
   - run one module with a filter, e.g. `cargo test -p ooxml-word-layout --test integration relayout::`.

---

## 13. Migration plan (strangler)

**Principles:**
- The product can be released after every PR.
- A document has one owner at a time; from P2 that owner is Rust.
- The bridge is metered and has an exit date.
- A format's legacy Python is deleted as soon as its last tool is ported.

**The anti-corruption bridge** (P2–P4 for Word; P5 slides; P6 sheets). For an unported Python tool on a Rust-owned document:
1. Rust writes the version's touched parts as bytes, cached per version (about 100 ms for the 3k body).
2. Python opens today's `Document` lazily from those parts and runs the old handler.
3. Python returns the parts it changed.
4. Rust parses them and diffs the trees structurally, short-circuiting on pointer equality (spike 1's `find_change`).
5. Rust commits the change as an exclusive transaction whose read set is "everything".
- History, the ledger and refs stay in Rust throughout.
- **Cost per call at 3k pages:** about 1 s. This is an estimate built from measured parts (write 0.1 s, lxml parse about 0.4 s, Rust parse 0.3 s), comparable to today's edits.
- `bridge_calls{tool}` is counted. A phase may exit only when its tools make zero bridge calls in the transcripts.

**Today's Rust:**

| Path | Decision |
|---|---|
| `crates/core/src/owner.rs` | **Replace.** Its coalescing moves into the `ooxml-session` scheduler: no thread per document, no JSON commits, no panics. |
| `units.rs`, `measure.rs` | **Keep, refactored** into `ooxml-base` and `ooxml-text`. |
| `model.rs` | **Delete**, with the exporter. |
| `xml.rs` | **Replace** with `ooxml-xml`. |
| `seqmatch.rs` | **Test oracle only, then delete.** |
| `crates/word`: `paginate.rs`, `linebreak.rs`, `table.rs`, `incr.rs`, `engine.rs`, `facts.rs` | **Keep, refactored** into `ooxml-word-layout`, with named types and the Word model as input instead of JSON. |
| `crates/word/src/parse.rs` | **Rewrite** as `ooxml-word`'s resolver, without Python callbacks; its tests stay as equality tests. |
| `snapshot.rs`, `fieldpages.rs` | **Fold into** the layout crate. |
| `crates/python`: `lib.rs`, `body.rs`, `tree.rs` | **Replace** with `ooxml-python`. |
| `bin/word-check.rs` | **Keep**, as an xtask command. |

**Phases.** A lane is one agent run of about a working day, as lanes F–K were.

| Phase | Scope | Exit criteria | Lanes |
|---|---|---|---|
| **P0 Foundation** | base, xml, zip, opc, store, derive, tools (contract and derive), session and python skeletons, xtask, CI. The layout track needs only base, xml, zip, opc and derive; the rest runs beside it | §15 | 5 |
| **PL Word layout in Rust (priority, from 2026-11-16)** | Four steps, below. Replaces the old P3a and takes the read-side model out of P1 | Below | 20 |
| &nbsp;&nbsp;L1 Text engine | `ooxml-fonts`, `ooxml-text`: sfnt reader, Office's measured tables, Word's advances, one break loop with Word's rules as data, hyphenation. Starts once `ooxml-base` is merged; runs beside L2 | Measures and line breaks equal docx2svg's on every corpus paragraph | 4 |
| &nbsp;&nbsp;L2 Word model for layout | Generated `w:` views, the styles cascade, numbering, fields threading, sections; open a version in Rust from its parts | Resolved blocks equal docx2svg's layout input on every fixture; open ≤ 1.5 s at 3k | 5 |
| &nbsp;&nbsp;L3 Word layout | `ooxml-word-layout`, seeded from `crates/word`, up to docx2svg's coverage (notes, columns, floats, sections, hyphenation, merged and nested cells, vertical alignment, keeps); layout facts for check; fields update; SVG for text, tables and pictures. Drawings, charts and SmartArt get their extents from Rust and are painted by docx2svg's Python painters until P3 (metered) | Relayout equals full; SVG byte-identical to docx2svg on the VRT and corpus; check's layout facts and fields equal | 8 |
| &nbsp;&nbsp;L4 Switch | docx-agent's layout, `describe`, check's layout facts, `render` and the live view use the Rust layout. Python still owns the document: each version's touched parts go to Rust, which reuses every unchanged block's memo. No layout objects in Python, no pickles. docx2svg's Python layout is frozen as a fallback and deleted in P4 | A made-up 360-page tracked report with 12 writers and 2 readers: memory flat across versions; one kept layout ≤ 50 MB; an edit's own work unaffected by kept layouts; relayout after a one-block edit ≤ 100 ms at 3k pages | 3 |
| **P1 Word reads** | Addresses and ledger (`docx-ids.1.`-compatible), Markdown out; `describe`, `word_read`, `word_inspect`, `find_text`, `read_batch` on the L2 model (replaces the mirror and the exporter) | Read tools equal on every fixture and transcript | 4 |
| **P2 Word common edits; Rust owns the document** | Transactions, `EditSink`, `RevisionPolicy`; set_text, insert_text, insert_markdown (Markdown in), delete, set_cells, undo, redo, batch, replace_text; validation per change; `touched`; the save gate and save; the bridge; docx-agent switched to the Rust session | Differential equal on transcripts and multi-writer seeds; 24 writers at 3k: edit p99 ≤ 250 ms, check p99 ≤ 100 ms | 10 |
| **P3 DrawingML, charts, metafiles, render** | colour, schema (`a:`, `c:`), drawingml, chart, metafile, scene, render (PNG via resvg); Word drawings, charts and SmartArt (ends L3's metered Python painting) | ooxml-common goldens equal; Word VRT complete; PNG equal or within SSIM bounds | 12 |
| **P4 Remaining Word tools** | Revision and comment tools, structure (move, copy_from, sections, headers and footers, fields, notes, links), objects (format_table, drawings, sdt elements, charts, SmartArt), style (format, lists, styles, template, design facts, compare, follow), numbers and scales facts; then delete docx-agent's and docx2svg's legacy | Zero bridge calls; docx-agent is a shim; legacy deleted | 12 |
| **P5 PowerPoint** | slides, slides-layout (one inheritance resolver), 36 tools, fit, compare; delete the legacy | pptx VRT, corpus and transcripts equal | 12 |
| **P6 Excel** | sheets (pull reader, one tokenizer), 21 tools, derive and query | Differential equal; the save-time read-back becomes a test | 10 |
| **P7 Cleanup** | Remove the bridge, the frozen legacy, the switches; final deprecations | No legacy imports left | 3 |

**Totals:** about 88 lanes. With at most 3 agents at once, and phase dependencies keeping the average at about 2.3 busy, that is about 38 waves. The Rust layout reaches production after P0 and PL, about 11 waves.

**Why this order:**
- **Word first.** It carries production's load and the measured pain, and the existing Rust and oracles are for Word.
- **Foundation first,** so Word is not built on the prototype's JSON model.
- **Layout before edits (2026-11-16).** Production's first large tracked report (about 360 pages, 12 writers)
  showed that the Python layout dominates both memory and CPU:
  - each kept whole-document layout is about 0.4 GB of Python objects, and the process grew to 5.4 GB;
  - Python's garbage collector walks those objects on every edit, so an edit's own work rose from 0.3 s to 10–14 s;
  - laying out the report costs 25–280 s of CPU under the interpreter lock per version.

  Edits in Python, by contrast, cost well under a second once layouts are out of the heap. So the layout moves first.
  - It needs only the read-side model (L2), not Rust ownership of the document (P2). Python keeps owning the
    document until P2, and hands Rust each version's touched parts.
  - A Python-side stopgap that bounds what is kept (round 58) holds production until L4.
- **P3 before PowerPoint,** because DrawingML, charts and metafiles serve both formats, and Word's drawings need them.
- **Excel last.** It shares the least, has no layout engine, and its pain is smallest; it could become a third lane from P3 (Q4).

**When each package becomes a shim:**
- docx-agent at the end of P4 (its tool calls go through Rust from P2);
- docx2svg's layout at the end of PL (L4), and its painting at the end of P3, with its API kept as `ooxml_core` render calls;
- ooxml-edit's tool layer at P2, and its editing core deleted at P7;
- ooxml-common at the end of P5;
- pptx2svg and pptx-agent at P5;
- xlsx-agent at P6.

---

## 14. Risks and open questions

**Risks and mitigations:**
- **A long tail of byte differences** (UTF-16, DOCTYPE, DTD references, stamping).
  - The corpus contract test grows with every fixture of all seven packages.
  - The byte differential runs on every ported tool.
  - Where the core cannot match, it refuses rather than approximates.
- **Effort underestimated** (layout's long tail, charts' 10k lines). Phases are gated on equality, not dates, and the bridge keeps the product whole meanwhile.
- **Two code paths during migration.** Each tool is frozen for features once its port starts, and the lane porting it also ports any Python fixes.
- **Font and metric fidelity drift.** The same generators produce the same tables, the oracle tests are unchanged, and resvg is pinned to resvg-py's version.
- **Concurrency bugs.** Read and write sets are typed, loom checks the store, the random differential runs nightly, and locks are taken in document-id order.
- **Memory with many documents.** A session budget, per-document accounting, eviction of derived data, and a tree at ≤ 2.5× its XML.
- **Library API breakage.** Handles keep signatures, and deprecations come two releases ahead.
- **Rust build times on the 16 GB machine.** Split crates, a shared target directory, sccache, opt-level 1 for dependencies in debug builds, and heavy builds under the lock.
- **Reply wording drifting** now that it lives in Rust. The differential compares wording byte for byte.

**Questions put to the user** (with the recommendation given; the answers are in [Decisions](#decisions)):
1. Keep libxml2's form for touched parts permanently? *Yes. Every golden file and saved byte stays the same, and Word accepts it. Add no "keep untouched element bytes" mode without a measured reason.*
2. Drop the free-threaded wheel? *Yes, unless production plans to run 3.15t.*
3. How much of the Python library API survives? *Keep every call that takes and returns plain values, and deprecate the lxml-exposing ones over two releases. If the team uses only the tools, go tools-only; please check with them.*
4. Run Excel as a third parallel lane from P3? *Yes, if machine capacity allows.*
5. Keep the provider adapters in Python, reading specs exported from Rust? *Yes.*
6. May `touched` come from change sets instead of difflib, provided it is equal on every transcript and stress seed? *Yes, and any difference would be shown to you before it ships.*
7. Approve new Rust dependencies: resvg, usvg, fontdb, the fontations crates, harfrust, a B-tree rope (or imbl), arc-swap, rayon, thiserror, tracing, PyO3, flate2 with zlib-rs (all MIT, Apache or Zlib licensed)? *Approve.*
8. Accept cooperative cancellation instead of killing worker processes? *Accept: every loop is bounded by input limits.*
9. Keep the seven distribution names, each pinning `ooxml-core` exactly? *Yes.*

---

## 15. Phase 0, concrete enough to brief

**Scope:**
- **`ooxml-base`:** units and exact arithmetic (from `crates/core/units.rs`), the `Context` ports, digests.
- **`ooxml-xml`:**
  - green tree with interned names and values, and a B-tree child rope;
  - parser for UTF-8 and UTF-16, with end-of-line and attribute normalisation, CDATA as text, opaque DTD references, and limits;
  - the libxml2-form writer, cursors, and patches by path and node pointer.
- **`ooxml-zip`:** read, raw copy, deflate (zlib-rs), limits.
- **`ooxml-opc`:** a persistent package with lazy part trees, relationships, content types, save.
- **`ooxml-store`:** `Txn` with typed read and write sets, commit and redo, `History` (undo, redo, scoped undo), `ArcSwap` snapshots.
- **`ooxml-derive`:** `Memo` and `MonoidTree`, with spike 3's list-label fact as the reference test.
- **`ooxml-tools`:**
  - the `ToolArgs` derive, with its schemas checked by today's `schema.py` checker;
  - the intent rules;
  - `ToolResult` and `ToolError`, with tests that refusals carry the way out.
- **`ooxml-session` and `ooxml-python`:**
  - `Session.open(bytes)`, `save`, `call(name, json)` with the lock released, `tool_specs`;
  - one real read-only tool end to end: `describe` over package parts, rendered for both providers by the Python adapters.
- **xtask:** check-layers, a codegen skeleton (relationships and content-types schemas), data-table packing, lint configuration.
- **CI:** fmt, clippy with the §2 lints, deny, doc, tests, fuzz (4 minutes per target), iai budgets, abi3 wheels.
- The prototype crates stay as they are, beside the new ones, until PL, P1 and P2 replace them.

**Acceptance tests:**
1. **Writer:** `write(parse(raw))` equals `serialize(parse_xml(raw))` for every XML part of every package in all seven packages' tests and fixtures, plus packages made by the generators. The spike's 620 parts are the floor.
2. **Save:**
   - an unchanged open-and-save gives files byte-identical to the input;
   - with every part touched, the entries (names, order, metadata, uncompressed bytes) equal Python's `to_bytes()`.
3. **Store:**
   - random transactions from N threads equal a serial replay in commit order;
   - undo and redo are inverses;
   - scoped undo refuses entangled steps;
   - loom passes.
4. **Boundary:** 24 Python threads on one document go through snapshot, prepare and commit. Rust work runs with the lock released (over 3 cores used out of 4), and each call costs ≤ 10 µs at the boundary.
5. **Fuzzing:** the XML, zip and OPC targets run for one hour without a crash.
6. **Gates:** no `unwrap`, `expect` or `panic` in library code; the layer check is green; no file is over 800 lines.

**Measurements** (best of 3, CPU and wall separately, raw JSON in a probe folder):

| Measurement | Target |
|---|---|
| Parse the 3k document.xml | ≤ 150 ms |
| Tree size | ≤ 2.5× the XML |
| Write | ≤ 100 ms |
| One version, replacing a paragraph | ≤ 20 KB |
| One version, inserting 5 blocks | ≤ 64 KB |
| 100 versions held | ≤ 3 MB |
| Save 3k with one part touched | ≤ 400 ms |
| Boundary cost and scaling | Spike 2's figures, measured again on the real crates |

**Size and order:** about 5 lanes. Wave 1 runs xml, zip and opc beside store and derive; wave 2 runs tools, session, python and CI. Two agents are enough. **Prerequisites:** the user's answers to Q1, Q2 and Q7.

---

## ADR list

Each ADR below records its alternatives and the measurement behind it; the sections above hold the context, trade-offs and consequences. A lane that changes an ADR's decision changes it here, in the same PR.

- **ADR-001: one Rust core, Python as a shim, threads with the lock released.**
  - Alternatives:
    - keep Python and port the hot paths (lanes F–K reached 0.16–0.5 s per edit at 3k, a 24-writer p99 of 25.7 s, and 5.2 GB);
    - free-threaded Python, where the edit work is still Python;
    - a Rust core run in processes.
  - Measured: 3.9 cores with the lock released against 1.23 with it held (spike 2).
- **ADR-002: a layered crate graph enforced in CI.**
  - Alternatives: one big crate; a few large crates.
  - Decision: about 25 crates in 7 layers.
- **ADR-003: a lossless persistent green tree, in its own crate.** Alternatives, the first two measured in spike 1:
  - (a) flat path copying: 705 KB and 1.3 ms per edit;
  - (b) persistent children: 9 KB and 16 µs, and 128 KB per insert with imbl;
  - (c) bytes per block, parsed again by every reader (lanes J/K);
  - (d) rowan: flat child arrays and no model for names or attributes;
  - (e) a mutable DOM with an undo log: versions are not values.
  - Decision: (b), with a B-tree rope, interned values and hash-consing. The rope is in-house: measured against imbl's Vector in P0a (section 4), and imbl is MPL-2.0.
- **ADR-004: libxml2's form for touched parts, raw bytes for untouched ones.**
  - Alternatives: keep element bytes inside touched parts; write Word's own form.
  - Measured: 620 of 620 parts equal. Decided: permanent (Q1).
- **ADR-005: typed views generated from the schemas.**
  - Alternatives: hand-written readers (today about 67 near-identical ones); a runtime schema interpreter.
- **ADR-006: commands as MVCC transactions.**
  - Alternatives: a lock per document (a 24-writer p99 of 24 times one edit); drafts as whole copies (worse under the interpreter lock, lane J).
  - Measured: a 3–5 µs commit.
- **ADR-007: one global pool, a logical owner per document, no processes, cooperative deadlines.**
  - Alternatives: an OS thread per document; worker processes.
- **ADR-008: hand-rolled incremental data (`ooxml-derive`), not salsa.** Spike 3.
- **ADR-009: one text engine.** Measured Office tables for layout, fontations for reading, harfrust and resvg for outlines.
  - Alternatives: keep three readers; take metrics from harfbuzz alone.
- **ADR-010: a Scene display list with typed writers, and PNG via resvg in process.**
  - Alternatives: string SVG (as today); Cairo.
- **ADR-011: the tool contract via derive, JSON across the boundary, adapters in Python.**
  - Alternative: typed PyO3 objects per tool (2.2 µs against 4.1 µs, but a second schema to keep in step).
- **ADR-012: typed errors, no panic across the FFI boundary, refusals that carry the way out.**
- **ADR-013: a strangler migration where Rust owns the document from P2, with a metered bridge.**
  - Alternatives: big bang; Python as owner with a Rust mirror (today's duplication).
- **ADR-014:** limits on untrusted input, and fuzzing.
- **ADR-015:** differential oracle, property invariants, instruction-count budgets.
- **ADR-016:** measured data tables generated as binary.
- **ADR-017:** abi3 wheels for every GIL build, plus a free-threaded `cp315t` wheel (Q2, decided 2026-11-16: production moves to free-threaded 3.15t).
- **ADR-018:** configuration injected through `Context` ports, never read from the environment inside the core.

---

## Spike results

Machine: Intel i7-6567U (2 cores, 4 threads), 16 GB. Load average was about 3 during spike 2, from other activity; no heavy lane was running. I took the heavy lock for the 3k Rust runs.

**Spike 1: lossless persistent tree** (`spike1/rs/src/main.rs`, about 600 lines; `spike1/summary.json`, plus `spike1/out/`).
- **Corpus:** the Rust writer equals lxml's `serialize(parse_xml())` on 620 of 620 parts from 80 packages, 386 of which lxml rewrites (`out/corpus-contract.json`, `corpus/`).
- **295 pages:**
  - the four common edits went through today's Python tool path (`out/295/`, `oracle.json`; Python wall times 0.31, 0.14, 0.17 and 0.02 s);
  - replayed as path copies, all five versions are byte-equal to Python's, undo included (`out/replay-295-wide.json`, `out/replay-295-flat.json`);
  - parse 30 ms (lxml 39 ms); write 9 ms (lxml 18 ms);
  - tree 11 MB;
  - per version: 7 KB to replace, 73 KB to insert; undo 0.3 µs.
- **3k pages** (`out/synth-3k-wide.json`, `out/synth-3k-flat.json`):
  - parse 295 ms; write 104 ms; the round trip is byte-equal to the file;
  - tree 113 MB with the naive layout;
  - a paragraph replaced in 16 µs for 9 KB; 5 blocks inserted in 0.6 ms for 128 KB; a table cell at depth 4 in 21 µs for 10 KB; undo 0.4 µs;
  - 100 versions held: 6.5 MB (flat: 70 MB).
  - Byte equality against Python at this size was not run.
- **Limits of the spike:**
  - the new nodes for each edit were taken from the oracle's output, so the edit logic itself was not tested; that is the port's job;
  - names are stored as written;
  - UTF-8 only.

**Spike 2: the boundary** (`spike2/src/lib.rs`, `bench.py`, `bench.json`; the built module is in `spike2/pylib`).
- Per call: 0.07 µs empty, 0.15 µs with the lock released, 4.1 µs with JSON, 12.1 µs including Python's `json`, 2.2 µs typed.
- 240 edits (2 ms prepare plus a 0.5 ms check each) over 1 to 24 threads:
  - lock released: 393, 784, 1,525 and 1,531 edits/s (3.9 cores);
  - commit section 2.8–4.6 µs;
  - 9 conflicts redone at 24 threads on a hot set of 200 blocks;
  - lock held: about 478 edits/s (1.23 cores);
  - check p99 at 24 threads: 200 ms with the lock held, 65 ms with it released.

**Spike 3: incremental list labels** (`spike3/src/main.rs`, `summary.json`, `run-1..3.json`; input `blocks-3k.json`, extracted from long-61.docx). The table is in §7; every variant was equal to a full recompute after every edit.

---

## Decisions

The user's answers to §14's questions (2026-11-16):

1. **libxml2's form for touched parts** stays the write contract permanently (ADR-004).
2. **Free-threaded Python** is a supported target: production runs CPython 3.15 and moves to free-threaded 3.15t, with
   several writers on one document. Keep the `cp315t` wheel beside abi3 (ADR-017) and test 3.15t in CI. The core's
   design (no global mutable state, work done with the interpreter lock released) serves both builds; nothing in the
   core may depend on the lock for safety.
3. **The Python library API:** keep what production imports beyond the tools — the tool layer (toolbox, dispatch with
   context, tool registration, the provider adapters and schema checks, limits, datasets), the agents' `Document` /
   `Workbook` handles with their reading and editing methods and `to_bytes`/`save`, address helpers, locales and the
   renderers' entry points — as handles into Rust with the same signatures; deprecate anything else over two releases.
   The exact list is kept with the migration lanes.
4. **Excel** runs as a third lane from P3.
5. **Provider adapters** stay in Python, reading specs exported from Rust.
6. **`touched`** may come from change sets once equal on every transcript and stress seed; differences are shown first.
7. **New Rust dependencies** approved: resvg, usvg, fontdb, the fontations crates, harfrust, a B-tree rope (or imbl),
   arc-swap, rayon, thiserror, tracing, PyO3, flate2 with zlib-rs.
8. **Cooperative cancellation** instead of killing worker processes.
9. **The seven distribution names** stay, each pinning `ooxml-core` exactly.
10. **The Word layout moves to Rust as the priority (2026-11-16).** After production's large tracked report, the
    layout (phase PL) comes right after the foundation and before the edit phases, with the fidelity gates unchanged
    (SVG byte-identical, relayout equals full).
11. **PNG re-parses the writer's SVG in process (2026-11-17), an exception to section 8's "no SVG
    re-parse".** usvg 0.48.1, the version resvg-py 0.5.0 ships, builds its tree only by parsing SVG:
    its node constructors are crate-private. So `ooxml-raster` hands the typed writer's SVG to usvg
    in memory (`roxmltree` with DTDs allowed, as resvg-py parses), and resvg draws it with a font
    database built once and shared. The PNGs stay pixel-identical to resvg-py's, which a scene →
    usvg tree conversion could not promise. The SVG stays the output in its own right (section 8);
    PNG is an extra step after it. *Revisit* when a usvg release with public tree constructors is
    the one resvg-py ships too: then the scene can go to usvg's tree directly.
