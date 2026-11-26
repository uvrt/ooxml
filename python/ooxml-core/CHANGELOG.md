# Changelog

ooxml-core is the Rust core of the ooxml packages (`crates/`), as the Python module
`ooxml_core`. Its version is the `crates/python` crate's.

## 0.12.1 -- v2026.11.26

### Added

A saved file's MS-FSSHTTPD 2.4.1 chunks for a co-authoring host (the design page "CyberAI as an Office Client",
request 4; lane coauth-2): `ooxml_core.chunk_file(data)` cuts and signs a file as Office does (an entry of at most
4,096 bytes one chunk signed SHA-1(header) XOR (CRC-32, sizes); a larger one a header and a data chunk; the central
directory the final chunk; a chunk over 1 MiB with random 1 MiB subchunks; a file Office does not analyse as ZIP in
1 MiB pieces) and gives `FileChunks`: its `chunks` (`FileChunk`: kind, entry name, offset, size, signature, random,
subchunks), `leaves()` as `(signature, size)`, the `root` and `delta(previous)`, a `ChunkDelta` naming the chunks a
host holding the previous version's leaves must be sent (`new`, `held`, `repeated`, `new_bytes`; an unchanged large
entry is reused through its parent leaf). The chunking is the shared Rust package layer's (`ooxml_zip::chunks`), so
docx, pptx and xlsx saves are signed in one place; written from the probe side's clean-room description and held to
its four test vectors byte for byte.

### Changed

- `Session.edit`'s fifth value is now the version the call's first edit was applied to, held (an
  `ooxml_core.Version`; its `state` the number it was), instead of that number: a host reads what
  the step changed from it, however many steps other writers commit meanwhile. The number alone
  could name a state the session's undo history had let go by then.

Word's part of the session is its own crate, `ooxml-session-word` (architecture review Phase 3, lane ph3-format, PR
A2): Word documents' opening, state, tool calls, checks, save gate, live view and page engines. `ooxml-session` is
format-generic and depends on no Word crate; a session is made with its formats (Word's, and the kinds Rust does not
own yet as packages). Tool results and the Python API unchanged.

- The Rust host's core (phase 3, D1a; crate `ooxml-host`): one call's pipeline from a tool's name to its
  reply -- a namespace stripped and another spelling of one tool read as it (with a note), a withdrawn tool
  refused with what to call instead, an unknown name answered with the tools and actions it most likely
  means; the roles a tool, an action or a call needs, refused in today's words; one acceptance (the tool's
  own, rule 1); `$name` refs on the declared paths, a changing call's ref names folded and owned first; a
  retry key's repeat; the handler run by the document's format on the version the call reads, what it
  leaves (`OpEffects`: refs, datasets, files, images) applied only once its change is published; the
  format's checks of a changing call; each call's record (its arguments' digest and shape). Problems in one
  shape (`code`, `part`, `element`, `message`, `severity`). Blobs are a format-neutral value the session's
  store stands behind. Nothing dispatches through the host yet: tool output is unchanged.

- Block identity in the Word layout (phase 3, LK1): every paragraph and table the layout reads from the body,
  the headers and footers and the notes now carries its block key beside its path, and a version holds its
  story parts' names. Painted lines, spans, paragraph rules, inline pictures and drawings, and the layout's
  line facts carry the key (a span its run's place), and the page SVG names them through the version's names.
  A note's blocks are named from their note by id, as the layout writes them. Output is unchanged.

- Block identity in the Word layout (phase 3, LK1b): table borders and cell shading are anchored on their table's
  cell at its place as written (kept beside the final view's rows), line numbers on their line's paragraph, column
  separators on the block holding their section's `w:sectPr`, and a text box's items are named below their
  drawing's name. A page's facts list the blocks it draws something of by anchor beside their paths. Every line
  and rule but a separator the layout makes itself is now named through the version's names. Output is unchanged.

- Block identity at the Word layout's output edges (phase 3, LK1c): the python layout rows' line ids and page
  blocks, a range's counted lines, the placeholders the coverage and the check list, and the live view's
  paragraph ids are now named through the version's names from block keys (each asserted against its old path
  in debug builds). The facts read from the page tier stay under paths until LK2. Output is unchanged.

- Refs and text facts as Rust session services (phase 3, C1): a document's refs by state number with their
  owners (undo, redo and scoped undo bring back each state's refs; refs outlive a close), their resolution on a
  tool's declared paths (`$name/rest`, a span's ends, the camelCase fold and its note, the misspelt suggestion,
  "was to be made by ops[N], which failed", the refusal to point another caller's ref elsewhere), the ref-name
  rule in words, and `{{d:...}}`/`{{t:...}}` references parsed and written in the locale (at most 50 per
  text, braces that look like a reference refused). A source's values (`Cell`, `SourceRows`, a cell's label, a
  date pattern) move from the Word tools to the shared tool contract. Nothing dispatches through them yet: tool
  output is unchanged.

- Datasets, sources and the session's stores as Rust session services (phase 3, C2): a dataset as a typed
  value in the shared tool contract (columns, rows of values, each value's sources, keys, type labels, the
  formula caches it was read from, number formats, Markdown worked out when first asked for, `from_records`, a
  run of its rows `d:x/#9:#16`); the session's dataset store in the order defined, within its limits of 50
  datasets and 50,000 cells, with staleness against each source's version now (undo makes a dataset current
  again, a scoped undo leaves it stale, a close keeps it as it was, a join's other sources count too) and the
  stale warning on every use; a dataset kept only when its call succeeds. A source qualified with its document
  (`t:x` as `d2/t:x`, with today's three refusals), read through the document's format (`FormatDoc::source`;
  docx takes none), and refused with only the sources that resolve. Blobs (checked by their bytes, images by
  their header, packages by their central directory), outputs and the image budgets. Nothing dispatches
  through them yet: tool output is unchanged.

### Fixed

- A font whose `cmap` or `kern` tables lay the same ranges over each other no longer stalls the Word layout
  or render (a document's embedded face or a font folder; found by the fuzzer, 246 s before, 1.9 ms now).
  A `cmap` subtable that claims more than its codespace (0x110000 for formats 12 and 13, 0x10000 for format 4)
  is refused whole, its characters unmapped; a `kern` table reads no more pairs than its bytes hold and stops
  at an Apple subtable shorter than its header. Fonts within the spec read as before (1,412 macOS and Office
  faces compared).

## 0.12.0 -- v2026.11.25.1

### Changed

The session is format-generic underneath (architecture review Phase 3, lane ph3-format, PR A1): each open document
is a format's `FormatDoc` over one generic history (`Stored`: undo and redo, a caller's own undo, step owners, state
numbers, step groups), and a version is held erased (`AnyVersion`). The tools register through
`ooxml_tools::Registrar` of their format's types and run as `ooxml_tools::Tool` in a `CallEnv`. Tool results
unchanged.

- Block identity, typed (phase 3, ID1): a block id is read into its story, kind (paragraph, table, row) and
  name (a durable paraId, a repeat `#k`, an invalid paraId, an ordinal `@n`, a table's row `/rK`), and written
  back exactly as before. Inside a document a block now also has a compact key (12 bytes) that the part's
  index makes and resolves in O(log n), and a version names a key's `data-docx-path` and id from the index.
  Nothing uses them yet: tool output is unchanged.

### Removed

`ooxml_core.tool_hosting` is gone (lane ph3-format, PR A1), with the Rust tools' `Hosting` flags (`blobs`,
`layout`, `keeps_written`): the Rust host hands every call what it reads, so no tool declares it. docx-agent lists
the few tools that read blobs, pages or values written by reference itself until its Python host goes.
Instead: nothing replaces it; every Word tool is the session's (read `served` as true for every tool
`ooxml_core.tool_specs("docx")` lists).

## 0.11.0 -- v2026.11.25

### Added

docx2svg's thin layer (lane svg-thin): `LaidPages.svg(page, width=None, height=None)` sizes a page;
`LaidPages.pngs(pages, raster, width=None, height=None, system=None)` draws pages as PNG with the faces
they draw, as the `render` tool draws them; `LaidPages.raster_files(pages)` gives the files a rasteriser is
handed for them; `LaidPages.coverage_summary()` gives the version's coverage. `WordFonts.face(family)`
says which installed file a face is laid out with, and `WordFonts.default_substitutes()` gives Word's
measured substitutes. `drawing_plan(svg, font_files, font_dirs)` gives the passes a text is drawn in where
two faces answer to one name, and `element_path(xml, path)` gives where a layout path's element is in a
part. A `PageRaster`'s own files are now handed over, after a document's faces, to the raster made for
its pages.

### Changed

- **A part saved with the bytes it was read with keeps its entry** (co-authoring request 1).
  The Rust package's save copied untouched entries raw already; a part touched but written
  again unchanged (a tree that writes the bytes read, bytes put back) was written with a
  new local header and central record ("version needed" 2.0, "version made by" 2.0, the
  external attributes `rw-------` where none), so its MS-FSSHTTPD chunk changed. Now its
  entry is copied as read wherever the header holds its sizes (no data descriptor, no
  needless ZIP64: `ooxml_zip::Archive::header_copyable`), as ooxml-edit's save does.

- A document's commit lock is now held only to publish (architecture review Phase 2, lane ph2-commit, PR 1). A
  commit checks what was written since its snapshot, and rebases and replays its changes onto the head, outside the
  lock. A command that keeps conflicting (a field update, a TOC, a list or comment edit that reads the whole
  document) now prepares behind a per-document gate instead of under the lock: other writers prepare meanwhile and
  publish after it, and reads, undo, redo and the history never wait for it. Results unchanged. The Rust session's `lock_times`
  reports how long the lock and the gate were held and waited for.

- A Word document's batch is now one transaction (architecture review Phase 2, lane ph2-commit, PR 3). Its ops run in
  it, each at a savepoint, and it commits once when `StepGroup.close` runs; until then only the thread that opened
  the group reads the ops' versions, and every other caller reads the head. The batch holds the document's gate from
  `Session.open_group` to its close, so other writers publish after it (as before). A second group opened by the same
  thread is refused; one from another thread waits. A `StepGroup` dropped unclosed takes back what it did and lets the
  document go. Handlers read the version their edits run on. Tool results unchanged.

- Step groups are gone from the store (architecture review Phase 2, lane ph2-commit, PR 4): every document's batch,
  of any kind, is one transaction its thread holds open (`Session.open_group`), as a Word document's has been since
  PR 3. Writers that waited for a batch's gate now publish before the next batch (or a command behind the gate) closes
  it again, so a host's batches sent back to back no longer delay the other writers past more than one batch. Tool
  results unchanged.

### Fixed

- Every version a writing tool makes has a valid, unique `w14:paraId` and textId on every paragraph and
  table row of every story, and its ids on every drawing, however the tool wrote it: stamped at commit
  after the command, from the id pool (never a retired id). Before, tools that do not stamp first
  (`word_fields` update among them) could leave a fixture's unstamped row, and Word then dropped every
  paraId of the document on saving it (paraid-durability.json, factors "row"; the new "tools" group
  measures the tools' outputs kept whole).

Architecture review phase 1 (lane ph1-seams, R2 #11): a document's page engine that a panic
left mid-layout is no longer reused. The next call that takes the engine finds it made
again from its seed (the same fonts, options and budgets; nothing laid out, so the next
version is laid out whole), instead of an engine whose memos may be half made.

Architecture review phase 1 (lane ph1-seams, R2 #6): an edit reads the document's tracking
mode for its whole call, so a `word_set_tracking` that commits beside it waits for it: an
edit is never written under one mode and committed after the mode changed. `describe`
answers `tracking` (`{"author": ...}`, or `null` when untracked) from the session's mode.
Undo leaves the mode as it is (as before; Word's Undo does not switch Track Changes).

- **A pending tracked row deletion no longer blocks the reads (round 60, R137).** Deleting a
  table row with tracking on, where the row held text the file had when opened, left its
  cells' paragraph marks deleted, and `word_inspect` of any target and `word_read`'s final
  view were refused ("Accept All's cell ends") from then on, after a save and reopen too and
  for files arriving with a reviewer's pending row deletion; `word_edit_table` refused the
  table's next edit the same way. Word 16.106's Accept All removes such a row whole (the
  last row, every row: then the table goes, the paragraphs around it stay), so its cells'
  ends take nothing in: the reads answer, and the final view is Accept All's. Where the row
  holding a vertical merge's first cell goes, the merge starts at its first kept cell (empty:
  the text went with its row), or that cell stands alone where nothing continues it; a row
  going inside a merge whose cell there is no part of it (as a tracked merge across a
  pending row deletion writes it) leaves the merge whole -- in
  `word_read`'s final view and the render model alike (`ooxml_word_model::docx::tables::
  merge_heads`). Measured: `crates/ooxml-session/tests/integration/row-deletions-word.json`.

### Removed

- `ooxml_core.LayoutFallback` is gone: nothing lays a version out another way. `WordPages.lay_out` and
  `update` raise `NotImplementedError` (naming why and where) for a version the layout does not lay out yet.

  Instead: catch `NotImplementedError`; the tools refuse such a version as not supported yet, with a way out.

Architecture review phase 1 (lane ph1-seams): the host seams the port left are gone.
`Session.set_last_check` (a check the host made itself: every check is the session's),
`Version.ids_since` and `Version.changed_since` (what a Python view of the document followed) and the `updates` count in
`Session.stats()` (commits of parts through the bridge). `Session.last_check` and
`Session.last_laid` give the version they saw as a number, never `None`. `tool_hosting(kind)`
no longer says `served` for each tool: every Word tool is the session's; it says what a tool's
calls read from the host (`blobs`, `layout`, `keeps_written`).
Instead: nothing replaces `set_last_check`, `ids_since` and `changed_since` (`parts_changed_since` names the parts) (no host check or view is left to
use them); read `served` as true for every tool `tool_specs` lists.

## 0.10.0 -- v2026.11.23

- **A call names documents as its host does (lane P2c17).** `Session.call` and
  `Session.edit` take `names` (`{the host's name: the session's id}`; `CallContext.names`,
  `ooxml_session::DocNames`): the arguments name documents by these, the session finds them
  by them (only by them, where a host gives any), and the reply names them so -- its `doc`,
  its undo and way-out calls, its summary, a not-open refusal's way out. One session serves
  every host session of a process and numbers documents across them, so a host's `d1` was
  the session's `d3` from the second host session on, and a served reply named the
  session's id.

- **Charts and SmartArt in Word's reads (lane R1).** `word_read` writes a chart's or
  SmartArt diagram's one-line summary in its drawing's comment (` [chart: column; title:
  ...; series: ...; categories: ...]`, ` [smartart: Goals [Faster edits], Risks]`, a group's
  each ` [d:G/m chart: ...]`) and `word_inspect` gives a drawing's or group member's
  `chart` JSON (types, titles, legend, format, categories, series, dates kept as serials) or
  `diagram` JSON (layout, nodes with level and text) as docx-agent does, from Rust:
  `ooxml_chart::data` reads a chart part as ooxml-edit's `chart_model` does (integers and
  floats as written, `c:ptCount` counts, a series' name its reference's first point),
  beside the render model and sharing its format-code walk; `ooxml_drawingml::diagram`
  walks a diagram's data model as ooxml-edit's `DiagramModel` does;
  `ooxml_word_model::docx::graphics` finds a drawing's parts. These reads no longer fall
  back to Python (`unported` no longer names graphics; only Accept All's cell ends).

- **Compact lengths in the layout's structures (phase PL, lane L4f).** One compact form
  for every exact length kept in quantity: `ooxml_base::Compact<L>` (16 bytes, numerator
  and denominator inline, the exact value boxed where they do not fit; `ExactLength` for
  a length type). `ooxml_text::CompactUnits` is `Compact<Units>`, and
  `ooxml_word_layout::CompactPx` is `Compact<Px>`; arithmetic stays exact on `Units` and
  `Px`. A line's height (`LineHeight`, 192 -> 96 bytes) and a cell's stacked line
  (`StackedLine`, 176 -> 96 bytes, its stack with no spare room) keep their lengths so and
  read them through methods; a cell block's pitches too. Equal geometries are kept once
  (`Measurer::share`, swept with the measures): a document breaks its paragraphs at a
  handful. A measured cell shares the model's resolved cell (`ResolvedRow.cells` holds
  `Arc<ResolvedCell>`) instead of copying 880 bytes. A painted span's pens
  (`Span.xs`) are `CompactPx` too, read through `x()` and `pens()`, with no spare room.
  Answers are unchanged (the text and page differentials).
  Measures kept by a layout, counting allocator: 311 -> 192 MB on a 2,988-page document,
  149 -> 121 MB on a made-up 350-page report; the first layout's live bytes 471 -> 341 MB
  and 178 -> 149 MB; its time is unchanged (8.4 -> 7.6 s, 1.60 -> 1.54 s best of 3). All
  350 pages painted: 109 -> 77 MB.

- **Smaller text pieces (phase PL, lane L4e).** The text engine keeps a piece per
  character in every measure; a piece is 104 bytes, was 208. Its lengths (advance, kern
  pair, hyphen) are kept as `ooxml_text::CompactUnits` -- numerator and denominator inline
  where they fit an `i64` and a `u32`, the exact value boxed otherwise, so arithmetic stays
  exact -- its format is shared by the pieces of a run, and what a pair kerns under is
  derived from its run and format when kerning instead of kept. A measure keeps no spare
  room. Measures and lines are unchanged (the text differential).
  Measures kept by a layout, counting allocator: 353 -> 149 MB on a made-up 350-page
  report, 483 -> 311 MB on a 2,988-page document (where line heights, geometries and
  tables' cell flows are most of the rest); the first layout's time is unchanged
  (1.56 -> 1.50 s, 8.2 -> 7.7 s).

- **A chart's axis intervals no longer overflow on a vast plot band** (the `chart_layout`
  fuzz target's find). How many rungs a value axis has room for is floored to a whole count
  held to what an axis can use (none to `AXIS_MAX_INTERVALS` plus the end labels' lines)
  where it is made, for the side, radial and bottom axes alike, so a finite count past `i64`
  no longer saturates and overflows taking the end labels' lines; a decade is asked only of a
  finite positive value. Every in-range count is the same as before.

- **Layout answers by id, made per page (lane L4b, after review).** A version knows its
  body blocks' ids as docx-agent names them in its bytes (`LaidPages.ids_of`, and each body
  line's id in `body_lines`); its layout summary is made and compared in Rust
  (`LaidPages.summary()`, `LayoutSummary.compare(before, aliases)`), each page's taken from
  the version before where the relayout kept that page; its warnings likewise (a page the
  relayout kept is said again, not walked again: 16 s a version at 3,000 pages before, now
  only the pages an edit reached), its page counts once per section, a field's page found
  through an index; `LaidPages.line_texts(page)` gives a page's lines' text facts (inked
  characters, text sizes, ink, widest word) without reading a span into Python, and
  `page(page, lines=False)` its rules, pictures and drawings alone.

- **The layout's memory (phase PL, lane L4b).** A version kept no longer holds the
  document's parsed text parts (its pictures keep the package without the main part and
  its headers, footers and notes: those are the version's model); a version laid out from
  the one before shares every measure, page and block it did not change, and its `weight`
  is what it alone holds now. The engine's memos are swept once a version is laid out
  (what no version in use holds goes), so they stay at what the newest version needs
  however many versions follow; the machine's faces are read once per process and shared
  by every document. `MemoryBudget(limit)` (with `BudgetAccount`): what a process's
  engines keep that can be made again -- their pages' paint, now kept in Rust, least
  recently used let go first -- and its host's kept layouts, within one limit, each
  document counted apart, and beside it the memos the versions in use hold (`held`);
  `WordPages(..., budget=)`. `WordPages.stats()` adds `swept`, `paint_hits`, `memo_bytes`
  and `paint_bytes`.

- **Glyph outlines and pair kerning bounded on untrusted fonts** (the `glyph_ink` fuzz target's
  timeout and slow unit). `GPOS` coverage and class ranges are kept as ranges instead of spread
  out glyph by glyph (a malformed table's ranges claimed every glyph thousands of times); a
  feature table is read once however many records point at it, and a table stating more lookup
  references than it has pairs of bytes is refused. A TrueType glyph is read within its own
  bytes, contours must end in increasing order, and a glyph's outline -- composites and Type 2
  subroutines followed -- may follow at most 65,535 points, components or operators (the
  format's own 16-bit counts); past that it is refused, never cut short. Real fonts' boxes and
  kerning are unchanged.

- **docx-agent's reads served by Rust (phase P1, lane P1c).** `Session.commit(doc, parts)` hands
  a version's touched parts over as `update` does and returns the version it made, held
  (`ooxml_core.Version`: `doc`, `number`); `Session.head(doc)` holds the head. A call given
  `at=[version]` reads that version whatever was committed since, and `placer` answers
  `word_inspect`'s `layout` (called with the blocks asked, it answers where each is placed).
  `Session.call` now returns a third item: what only the application reads, as JSON text
  (`unported`: what of the answer is not yet given as the Python tool gives it).
- A part handed over is diffed with its tree in the version before and shares every subtree
  that did not change; a story part's lookups (controls, revision records, drawings, link
  anchors) are found in one walk and kept per block, so a new version reads only what changed.
- `find_text`'s `regex` takes Python's regular expressions (`fancy-regex`): matched as
  `re.finditer` matches them where the meaning is certainly the same, refused with why
  (and the call that finds the text as written) where it could differ or the pattern can
  match nothing.

- **Text boxes, text beside drawings, drawings in table cells, floating tables, frames and
  text effects laid out and drawn in Rust (lane L3g).** A text box's text is laid out as a
  story of its own in its text area and drawn in its drawing; a cell is laid out around the
  floating drawings anchored in it (beside or below them, held inside the cell, its row
  reaching down to them, aligned with its lines); a run's `w14:textFill` and
  `w14:textOutline` are painted. The fallbacks `floating table`, `drawing in a table cell`
  and `drawing text wraps around in a table cell` are gone; `text beside a drawing` and
  `frame` remain only in headers, footers and notes, `text box` only for a text box holding
  a drawing text wraps around or an inline drawing that is not a picture.

- **SmartArt, VML shapes and text paths painted in Rust; charts in any locale (lane L4d).**
  `ooxml_word_pages::drawings` draws a SmartArt diagram's cached drawing (`dsp:drawing`,
  found from its data model as ooxml-common's `diagram_drawing_part` finds it, its shapes
  resolved through the document's theme, colour map and font scheme as docx2svg's
  `diagram.py` resolves them), a VML rectangle, rounded rectangle, oval or text box as the
  DrawingML preset it is, and a VML text path (a watermark) stretched to its box by its
  face's outlines at its kerned pens, as docx2svg's `drawing.text_path_markup`; the
  fallbacks `SmartArt`, `VML shape` and `chart in another locale` are gone. `WordPages`
  takes the number locale a chart's numbers and dates are written in (`thousands`,
  `date_order`, `list_separator` beside `decimal` and `locale`). A page view's drawing
  slots say the faces their text is drawn in (`faces`). `ooxml-fonts` reads glyph outlines'
  exact boxes (TrueType `glyf`, CFF Type 2) and `GPOS` pair kerning (`Face::ink`,
  `Face::pair_kern`), as docx2svg's `outlines.py`.

- **Drawings painted in Rust (phase PL/P3, lane L4c).** `ooxml_word_pages::drawings` paints a
  page's pictures, shapes, groups and charts as docx2svg does (through `ooxml-render` and
  `ooxml-chart`, with Word's colours, theme style references and chart rules) and says what
  docx2svg says of them; the `drawing` fallback is gone. `LaidPages.svg(page)` takes no
  painter: nothing calls back into Python to draw.

- **The Word layout in Rust (phase PL, lane L4).** `WordFonts` (a machine's fonts, scanned
  once), `WordPages` (one document's page engine: each version laid out from the one
  before, given as its package's bytes -- only the parts whose stored bytes changed read
  again -- or as its touched parts, `update`) and `LaidPages` (one version: its pages'
  frames, its body's lines and blocks, keeps, page-number fields, coverage, each page's
  view and SVG, and digests of what every page paints), over the new crate
  `ooxml-word-pages`; `LayoutFallback` for a version it does not lay out as docx2svg does
  yet, with its reason. The interpreter lock is let go for every layout and paint.

- **Word reads in Rust (phase P1, lane P1a).** The session's `describe` gives every field of
  docx-agent's but `pages`, `problems` and `revisions.changes` (headings, sections, styles in
  use, revisions, fields, tables, drawings and named content controls added), and it serves
  `word_read`: the document as Markdown with ids, in the final, markup and original views,
  by range and with every story, its definition docx-agent's byte for byte. Both are read
  from the Word model (`ooxml-word-model`'s `docx` and `markdown` modules), whose `ids`
  module holds the block ids, the id ledger and the `docx-ids.1.` state as docx-agent does.
- **Word reads in Rust, part 2 (lane P1b).** The session serves `word_inspect` (without
  `layout`), `find_text` (a regular expression only where it is plain text) and `read_batch`
  (each op answered as alone, the answers within the batch's budget), and `describe` gives
  `revisions.changes`; every address form docx-agent's tools take is read by one resolver
  (`ooxml-word-model`'s `address`: bookmarks, content controls and text ranges added). A
  document's readings are kept for its latest versions: a read of a version read before
  reads nothing again, a block or span is read alone, and a new version reads only the story
  parts that changed; formatting is resolved once per style and property set.

### Added

`check` in Rust (lane CL1): a body paragraph holding a drawing the layout does not model (a
VML shape it cannot read, an embedded object) is laid out as a placeholder of its estimated
lines, as docx2svg does with `CARRY_ON`, and the layout carries on after it: `reflow`'s
coverage is `partial`, lists the placeholder (page, reason, `at`, message) and leaves its
block out of `blocks_laid_out`, `layout_warnings` say `layout-placeholder:drawing`, and
`layout`'s `uncertain` names the page. Before, the whole document fell back to the host
(`check: the layout falls back`). The page is not painted in Rust (docx2svg's marked box is
not what Word draws): its SVG says so. `pages` holding none of a range's blocks read no
page's facts and say so in a warning, as docx-agent does (before: `check: pages outside the
ranges`, answered by the host). `LaidPages` gives `placeholders()` and `blocks_laid_out`.

**New ids drawn where the change lands (lane D1, part a).** Writers that each make new
paragraphs or rows from one version (`word_insert_markdown`, new paragraphs, rows,
sections, notes) no longer conflict over the ids they draw: an edit's new ids are
stand-ins drawn again where its commit lands (`Change::Draw`, `ooxml_word_edit::Drawn`),
so each gets the ids the commit order gives, byte for byte as docx-agent in that order.
The store keeps keys created under stand-ins apart and resolves them at commit
(`DocModel::created`, `resolve`, `begin`); replies name the ids as drawn
(`ooxml_tools_word::resolve_drawn`). A stand-in written where no draw reaches is pinned
(its sequence read: a conflict, as before).

Lane E1b: the Rust `word_edit_table` runs `sync` (a table kept in step with a source's rows by
key, the host handing the rows over as for `set_cells` `from`); `PlaceRows` adds rows beside any
row, deletes rows and keeps or drops the table's record in the same step.

Lane E1: `CallDocuments(sources=...)`: the rows of the sources a call names, as the host
resolved them (JSON text: ooxml-edit's `Rows` per value as the call gives it). The Rust
`word_edit_table` writes `set_cells` from them: the block placed from `index`, rows grown
after the last with `grow`, the table's record of the source kept, the refusals of a block
that does not fit as docx-agent words them.

Lane E1c: `KeepingRecord` runs an `EditTable` or `SetCells` keeping the table's record of its source
true in the same step; `tracked_row_fates` says how a tracked deletion would treat each row.

Lane E1d: `word_edit_table` keeps values written by reference (`Hosting::keeps_written`); a
source the host refused may name the argument it refused (`field`, with its `code`), for a cell's
one value.

Lane E2: `keep_together` is the Rust session's in `word_format` and `word_format_table`. A span,
table or section `word_format` keeps on one page that the version before the change is drawn
taller than a page in -- where the document's last check laid that very version out -- is said
so ("cannot be kept on one page"); nothing is laid out for it. `word_format_table` takes the
document's page engine (its hosting declares the layout) and lays the version it made out for
the "taller than a page" warning; where no layout is handed, or laying out stops or is not
done, the keep is made and the warning says its pages are not known, as docx-agent says it.
`Session.block_pages(doc, blocks, options)`: the pages the head version draws each body block on,
numbered from 1 as the session's other page reads number them, laid out by the options' page
engine within the call's limit on laying out (`("pages", numbers, None)`, `("outside", ...)`,
`("unknown", None, why)`, or no blocks and why nothing was laid out). Inside a step group
`word_format_table` leaves a table kept together unmeasured, for the host to ask after its batch.

Lane G8: `ooxml_core.tool_hosting(kind)`: how a host hands each tool its calls, as JSON
(`{name: {"served", "blobs", "layout", "keeps_written"}}`), each Rust tool's
`ooxml_tools_word::Hosting`. The tools register themselves where they are defined
(`ooxml_tools_word::Registrar`, `register`); `Registry::standard` no longer lists them, and
`Call` and the handlers are in modules of their own (`call.rs`, `handlers.rs`). The
definitions (`tool_specs`) are unchanged, byte for byte.

Lane H1: step groups. `Session.open_group(doc)` gives a `StepGroup`. The commits made until
`StepGroup.close(context=...)` are one undo step of the context's caller. `mark()` and
`revert(mark)` take back what was committed after a mark (by default since the group
opened), as a new version that no redo brings back, with the tracking mode as it was then.
Plain redo is held while the group is open and given back when it commits nothing. Undo
and redo are refused while a group is open. This is built on `ooxml_store::Store`'s
`open_group`, `group_mark`, `revert_group` and `close_group`.

Lane H3: `ooxml_core.CallContext(max_result_chars=, max_list_items=)`, the host's limits on a
call's reply (24,000 characters and 50 items unless given; 0 is refused), carried in the
session's `CallContext` as `ooxml_tools::Limits`: reading tools are handed them with
`Version::with_limits`, editing tools with `Editing::with_limits`. `find_text`, `word_read`,
`word_design_facts`, `word_changes` (list, preview), `word_comments` (list) and `read_batch`
page by them where they paged by the defaults. `ooxml_tools_word::Version` is made with
`Version::new` and `with_*` setters (`non_exhaustive`).

Lane H7a: `Version.problems()` gives the version's validation problems as `(code, part, detail)`.
`Version.summary()` gives what `open_document` says of a Word document (JSON). Both read the
version in Rust. `Version.id_state(number, last, gone=, drawings_gone=, earlier=, said=)` gives
the version's id state from the ids the session holds, with the host's numbers. Its record is
docx-agent's `Document.id_state` record; the deflate bytes may differ, and either side opens the
other's state.

The session keeps each Word document's page engine itself (lane H7b). `ooxml_core.PageSetup(fonts,
*, locale, decimal, glyph_size, numbers, budget)` is what it lays documents out with (a
`WordFonts`, the engines' options as `WordPages` takes them, a `MemoryBudget`);
`Session(pages=...)` sets it for every document, `Session.set_pages(doc, pages)` for one (other
font folders or another locale), and `Session.pages(doc)` gives the document's engine as a
`WordPages`, made on first use. A call whose host gives no layout is laid out by that engine where
the session has one (the reflow after an edit is still measured only with the host's layout), and
`word_inspect`'s `layout: true` places the blocks from the session's own layout of the version
where the host gives no placer: each block's pages, tops, bottoms, lines, story and column, as
docx-agent read them from a Rust layout.

Lane H7c: `Session.open_state(data, state, *, name)` opens a Word document's bytes with the id state saved beside them. The ids are read against the parts as they are parsed, once. No retired id is given again, and the ids the state retired are the session's record (`retired`). It returns the document's id and the state's `version` and `last`. `Version.file_kind()` gives the package's Word kind (`docx`, `docm`, `dotx`, `dotm`). Two new exceptions: `StateRefused`, raised for a state that is not these bytes' (docx-agent's words, with the way out), and `NotWordPackage`, raised for a package without a Word main part. Both are `ValueError`s.

Lane H7d: `Version.block_kind(id)` says what kind of block an id names in a Word version:
`"paragraph"`, `"table"` or `"content control"` (a block-level one). A renamed id is followed.
It gives `None` when the id names none of the blocks. `Version.has_drawing(id)` says whether the
version has the drawing `d:N`.

`Version.number_texts(doc, request, pages=None)` and `Version.scale_values(doc, request,
pages=None)` (lane H7e): what a check of numbers and a scale read in a Word version -- the
body's and the cells' texts with their spans written by reference, a scale's targets' values
with the step each is styled as now -- within a check's range and on its pages, placed by a
`WordPages` engine where one is given (JSON in, JSON out; a refusal as `{"error": ...}`).

Lane L1: `set_properties` is a Rust tool (`tool_specs("docx")` lists it, its definition
docx-agent's byte for byte; `tool_hosting` serves it).

Lane LC1: `ooxml_core.new_document(page=(size, landscape), language=, separators=(decimal,
list), properties=(title, author, created))` makes a new blank document's package in Rust,
entry for entry and byte for byte the package docx-agent's `edit/blank.py` writes (Word
16.106's new blank document: its styles, settings, font table and theme, generated from
docx-agent's measured values by `crates/ooxml-tools-word/data/word_new.py`).

Lane LC1: `save_document` is a Rust tool (`tool_specs("docx")` lists it, its definition
docx-agent's byte for byte). It writes the version as a .docx or .dotx (a .docm or .dotm kept
when the document is one and the name says so) -- refused when it holds macros the kind cannot
carry, the main part retyped, `app.xml`'s statistics refreshed when the main part changed
since the document was opened -- cleaned when asked (orphaned parts, rsids, web divisions, an
external template link), or as Markdown. `Session.call` returns the files a call gives the
application in its second slot (bytes), each one's `name` and `format` in the application's
JSON (`files`); `CallContext(writing=CallWriting(layout_counts=(pages, lines)))` hands a call the
host's layout statistics.

- **The Word live view on the session (lane LV2).** `Session.view(doc, since=None, *, layout,
  context=None)` gives the head version's pages (each `(number, blocks, width, height)`: the
  ids of the body's top-level blocks it draws, in document order, its size in points), the
  pages whose drawing changed since an earlier view's token (every page without one; those
  the earlier version did not have too), and a token, `<session nonce>-<doc>-<state
  number>` (the nonce hashed with the session's own random keys: two sessions made at the
  same moment tell their tokens apart); `stale` with a `note` where `since` names no version
  kept (another session's, older than the last four, or one whose layout the memory budget
  had no room for). `Session.page_svg(doc, page, token, *, layout, text="text",
  context=None)` draws one page of the version a token names as SVG, its paragraph groups
  tagged with that version's ids (`data-docx-agent-id`, a header's, footer's or note's
  paragraph's from its part), even after the document moved on: a version the engine no
  longer paints is laid out again on a spare engine once, that layout kept with it for its
  other pages, never becoming the engine's last.
  Both lay out with `layout`'s page engine (`CallLayout(pages=)`) within the context's
  limit on laying out, with the interpreter lock released; a version the engine does not
  lay out yet, and `text="outlines"`, raise `ooxml_core.ViewRefused` (as `render` refuses);
  an unknown token `ooxml_core.UnknownToken`. In Rust: `ooxml_session::Session::view` and
  `page_svg` (`live_view`, the kept versions counted in the session's memory budget by
  their layouts' weight: a layout that does not fit lets the least recently used go, and
  where it still does not fit is not kept, its pages laid out on demand), `ooxml_tools_word::laid_to_draw` (render's laying out within the
  call's limit, shared), `ooxml_word_pages::LaidVersion::top_blocks`.

- **A refusal of a removed id in docx-agent's words (lane P2e1).** A call refused `not_found`
  that names an id a version removed says which change removed it, from the document's
  retired-id record (a block one block replaced: "replaced with <id>", "or use <id>");
  `Session.number_version(doc, version, number)` takes the host's number
  of a version.
- **A call's date from the host (lane P2e1).** `CallContext(writing=CallWriting(date=...))`: the moment the call's
  writes are dated by, UTC to the second; without one, the session's clock, now to the second
  too (was: to the minute).
- **A call's typography from the host (lane P2e7).** `CallWriting(typography=False)`: the text
  the call writes is written as given, none of the spacing its language wants (docx-agent's
  `docx_format(typography=False)`); by default it is spaced, and the reply says how many spaces
  it wrote as no-break spaces. A served `insert_markdown` names a `table_cell` style whose
  indents do not fit its cells (round 21, T4).
- **`ooxml_core.CallWriting` (lane P2e8).** How a call writes, its date, typography and the host's
  layout statistics a save writes (`layout_counts`), grouped in one `CallContext` setting
  beside main's reply limits.
- **Other documents' tables in a refusal (lane P2e8).** `CallDocuments(tables=callable)`: the
  tables of the open documents the host holds itself (`[(name, doc, kind)]`, a workbook's
  `t:stock`), asked only where `word_edit_table` refuses a table not in its document, which
  it then names as one of them with the `sync` call that keeps a table in step with its rows.

**The pages a check of the layout alone laid out are kept (lane P5r).** The session keeps,
beside what its last check with `reflow` saw (`since_last_check`, as before), what its last
check that laid the document out saw, with `reflow` or not (`LastCheck::laid`; a check of
`layout` or of ranges sets only that one). `Session.last_laid(doc)` reads it: `(version,
summary)` or `None`.

Lane P5m: `Session.edit` measures the reflow a changing call's checks report, after the call's
edits, where the options' page engine laid out the version its first edit was applied to (or the
document's last check saw it): docx-agent's `reflow_json` form, `"stale: call check"` past 20
pages, or a `timeout: …` where laying the new version out passed the call's `layout_timeout`. It
is the fifth item of what the call made (JSON text, `None` where none was measured).
Lane P5o: the measure never waits for the page engine and is never dropped for it: where another
call holds the engine, the version before and the version after are laid out on a spare engine of
its kind (`WordPages`' seed: the same fonts, options and budgets, nothing laid out) and compared
there; what the engine laid out last is read from that seed without its lock.

**A call's locale (lane P5q).** `CallWriting(locale=...)` (a `CallContext`'s `writing`) and `Session(locale=...)` take
ooxml-common's `Locale` (or a BCP 47 tag, read as `Locale(language=tag)`): a table of
contents' `\t` styles are split on its list separator and found in its language, in
docx-agent's order -- the call's locale, then the session's where the host set one, then
the document's own language (`w:themeFontLang`, then the default run language).
`Session()` without a locale is en-US as before and sets none for its calls. One locale type
across the crates: `ooxml_numfmt::Locale`, from which a context's tag
(`ooxml_base::Locale`) is derived.

The Rust page engine lays out and paints a text box holding an inline drawing that is not
a picture (a chart, a shape, SmartArt), as docx2svg does: the drawing is painted inside the
text box's drawing, after the box's pictures and before its text, and what its painter says
is said with the box's warnings. Such a document fell back to the Python layout before
(`support::TEXT_BOX`). A text box holding a drawing with text of its own, or a floating
drawing text wraps around, still falls back.

Lane RD1: `ooxml_core.CallLayout(pages, *, raster=None, image_budgets=None, placer=None)` is
what a call reads of the document's layout. It goes to `CallOptions(layout=...)` in place of
the `placer=` and `layout=` keywords. `render` is a Rust tool (`tool_specs("docx")` lists it,
and its definition is docx-agent's byte for byte). It draws pages and sheets of pages from
the host's `WordPages`, with a document raster made from `raster` (the system's fonts and the
font folders) for the faces the pages draw. Images are kept per document within the session's
memory budget. `image_budgets` are the host's session and round budgets of
images as they stand (`[("session", limit, used), ("round", limit, used)]`); `render` refuses
images past them, worded as docx-agent words it. `tool_hosting("docx")` says `render` is
served with the host's page engine: a reading tool now declares its hosting too.

Lane U1: the store numbers a document's states as the tool layer numbers them
(`ooxml_store::Version::number`): the state opened 0, a step one more than the highest number
given, a plain undo or redo the number of the state it brings back; given in publish order in the
store's commit section. `Store::numbered(n)` finds the head-most kept state with a number. The
session's `Opened::number` reads it.
A step group is one step and one number (lane U2): its commits carry one more than the highest
number given when it opened, given for good only when it closes having made a step; a revert of
all its commits brings back the number it opened on.
Lane U3: `Version.state` (the state number; `number` stays the session's count of versions);
`Session.numbered(doc, n)`, `Session.highest(doc)`, `Session.numbers(doc)` (the highest number,
the steps undo can reach and redo can bring back, each `(before, after, seq, caller)`) and
`Session.continue_numbers(doc, state, highest)` for a document opened again. `Session.edit`'s
made tuple ends with the state number the call's step went from. A changing call's
`result.version` and `Session.undo`/`redo`'s version are state numbers (`Store::listed`,
`Store::highest`, `Store::continue_numbers`; `StepInfo::number`/`from_number`).

Lane U4: `undo` is a tool of the session's own (beside `read_batch`): the session's undo or redo
of a document's latest steps, or with a named caller that caller's own, in today's words -- the
later step and who made it when a caller's latest change is not the latest (`entangled`), and a
caller's own redo refused as `entangled` while another caller's undo is the next to redo. The
session keeps who made each step (tool and caller, by state number) as a call, a bridged step or
a step group's close is numbered; `CallContext(context=...)` gives the caller's context in the
order the host gave it (the replies name it so), `StepGroup.close(tool=...)` the batch's tool.
A caller's own undo or redo checks and moves the history in one step of the store's
(`Store::undo_newest`, `Store::redo_next`), so no commit lands between them.
Lane U5: "nothing (of yours) to undo/redo" is refused with its way out (tool rule 2), a deliberate
change from docx-agent's reply: `valid_options` lists the document's steps, newest first, each with
who made it (`version`, `tool`, `context`; `yours`, `undone`), then the oldest state kept
(`{"version": n, "oldest": true}`), at most 10 on each side of the head (`details.left_out` counts
the rest); `try_call` is the undo or redo the other way where that one would work. The way-out-less
`ToolError::nothing_to_offer` is gone.

### Changed

Lane CUT1: a form a Word tool does not support yet is one refusal, said one way
(`ooxml_tools::ToolError::not_supported`): code `refused`, "<form> is not supported yet", the
form's stable id in `details.not_supported` and a way out (a call of a supported form). The
reply's application-only `unported` reasons are gone (`Done::unported`,
`ToolResult::unported`, `Session.call`/`edit`'s `app["unported"]`): no call is handed back to
the host any more. `ToolResult::not_supported_form` names a refusal's form.

- **`ooxml_core.CallOptions` (lane P2d3).** `Session.call(name, arguments, options)` and
  `Session.edit(name, arguments, options)` take what one call runs with as one
  `CallOptions(context=, limit=, documents=, placer=, layout=, blobs=, written=)` in place of
  their growing keyword arguments (the `too_many_arguments` allowances gone); `edit`
  refuses options with a `limit`. What a call needs about the other open documents it names
  is one `CallDocuments(at=, names=)` (the versions it reads, the host's names for them; lane
  P2c17's `names=` keyword moved here). docx-agent's callers moved to it.

Lane P5e: a served `check`'s `reflow` keeps the version it saw, not its page summary. A
first check paints no page for a summary (it has nothing to compare with); the next check
compares the two versions page by page in the document's page engine
(`PageEngine::compare`): a page both versions lay out the same, in the same page context,
is the same page and is not painted, every other page is digested in both, and each page's
digests are kept across versions (bounded per document, dropped with the engine's
generation). A page's context is its own (its number, its section's and the document's
page counts, its section's properties, its header's `STYLEREF` texts, its line numbers):
one page more no longer makes every page digested again. A version a later check may still
compare with is digested whole before a change of styles, numbering, settings, theme or
fonts makes it unpaintable. `Session.last_check()`'s `LayoutSummary` is that version,
summarised when Python reads its pages or compares it. Answers are unchanged.

Lanes TB and RD2: a call's reply limits are one `ooxml_core.Limits(result_chars=, list_items=,
images_per_call=, image_edge=)` (the session's `Limits`; 24,000 characters, 50 items, 4 images
of at most 2,576 pixels unless given, 0 refused; `render` draws within the last two), passed as
`CallContext(limits=...)` in place of `CallContext(max_result_chars=, max_list_items=)`.

**A panic is the session's own reply (lane Z1).** `ooxml_session::Session::call` and
`Session::edit` (and each of a batch's ops, and `undo` and `read_batch`) catch a panic in a
tool themselves, so every host gets it the same way -- the oracles' binaries as well as this
module: an `internal` failure, "the call failed inside the library; nothing changed" (or that
what it changed before it failed stays, as its step), the same call as its `try_call` (retry,
else report), and `details.trace` = `panic:<tool>:<n>: <what it said>`. The panic's message
is the application part's `panic` (`Session.call`'s and `Session.edit`'s app JSON; `null` for
every other call), and the calls that panicked are counted by tool: `SessionStats::panics`,
`Session.panics()` and `Session.stats()`'s `panics`. The module no longer catches panics
itself.

### Fixed

Lane CUT1: `check` of a document given a deck's arguments answers as docx-agent did instead of
refusing them as not supported yet: `slides` read as the document's pages (warned "slides read as
pages: a document has pages", or ignored beside `pages`; a slide id refused naming `pages`), and
`boxes`, `spacing`, `contrast` and a deck's facts in `include` refused as `invalid_arguments`
naming what a document takes.

Lane D3: setting a long text to a wholly different one no longer spins for minutes. The text
diff behind set_text, tracked replacements and paragraph pairing finds each longest match with
a suffix automaton (the same opcodes as Python's `difflib`, autojunk off), and a diff that
would examine more than four million elements keeps only the common start and end, with one
change between.

Lane LT1: `LaidPages.page` gives each drawing its text boxes' `lines` (rows as the page's
`lines`, in page coordinates; none with `lines=False`), from the page's paint
(`PageView.text_box_lines` in ooxml-word-pages).

A document owner logs each pass before readers see the snapshot it made: a reader woken by a
new snapshot, or one reading the latest at once, finds that pass in the owner's statistics
(`stats`). Before, the pass was logged after the snapshot was published, and a reader could
read the log one pass behind (the flaky test `owner::commits_in_version_order_and_coalesced`).

- `Session.bridge(..., ids=HostIds(drawn={seed: index}, aliases={old: new}))`: what the bridged tool's id
  ledger holds beside the parts. An id it drew and did not write is not drawn again (the session's
  next edit draws the id docx-agent's own next draw gives), and a block it stamped is found by its
  old id in the session's reads and edits (lane P2e6).

Lane P2e3: where the Rust core reads a character as Python does, it reads Python's own
predicate (`ooxml_base::pystr`, tables generated from CPython 3.12 by
`tools/pystr_tables.py`): automatic hyphenation's letters (`str.isalpha`, not Unicode's
Alphabetic: combining vowel signs and letter numbers are no letters) and the digits beside a
word (`str.isdigit`); a number format's `[ColorN]` (`isdigit`: a vulgar fraction is no
colour number); the bookmark-name check (`re`'s `\w` and `\d`); and which paragraphs' records
the Rust export makes (a paragraph with a non-ASCII digit stays Python's, one with another
numeric character no longer does).

Lane P4f8/P4f9: `copy_from` reads what its destination's paragraphs use (`Key::StyleUse`: each
paragraph's style, list and whether it is blank) for the body style `unmapped: body` maps to
and the list `lists: continue` joins. A concurrent `set_text` that gave a blank paragraph text,
or a list edit that took the items before the place out of their list, wrote block keys only,
so a copy prepared before it could commit a body style or list a serial run would not give; it
is now prepared again.
The style uses are a derived key (`DocModel::derived`, new in the store): a copy that read them
is prepared again (or a step that read them keeps an earlier one from undoing), but two edits that
both alter them (two insertions from one version, two paragraphs given text) commit without a
conflict and undo past each other.

Lane P5l: an edit that lays the document out (a field update, a page cross-reference, a table's
keep_together pages, word_template's reflow) keeps the call's `layout_timeout`: past it, the call is a `timeout` that
changes nothing ("laying the document out did not finish within N s"), as docx-agent says it.

A page engine given each version as its package (`PageEngine::lay_out`, a session's
path) now finds the parts that version changed from the one given before, by identity,
else by bytes (`ooxml_opc::Package::changed_parts`). Before, it kept the list from the last version given
as bytes or touched parts: after a session's edit of a part beside the document part (a
diagram's, a chart's, the relationships), the pages the edit did not reach were said again
with the warnings of the version before, and a version laid out whole for its styles did
not name the styles part as its cause.

`check`'s `reflow.coverage.approximations` from a Rust layout now lists each entry as
docx-agent does: a glyph a face lacks with its text as the `reason` and an empty `message`
(docx2svg's `coverage_of` records it so), any other by the part of its code after the colon
(the whole code where it has none). Before, a glyph's entry said `reason: "glyph"` and put its
text in `message`.

`check`'s keep trials measure the blank at a page's foot as docx-agent does: up to the
page's footnotes (their separator's line) and below the lines its body's text boxes draw,
which the layout's facts leave out. The Rust check said a page with footnotes was blank
down to its text area's bottom.

A `PAGE`, `NUMPAGES` or `SECTIONPAGES` field (and a `PAGEREF` to a bookmark) in a paragraph of
a body text box gets its result from the page the box draws that paragraph on, as docx-agent
and Word do; the Rust update left it empty and warned that the layout cannot say, the body's
lines not drawing the box's paragraphs.

Lane PG1: a version laid out before a style edit (its styles, numbering, settings or theme)
is painted and measured in its own styles, by the page engine itself: its pages' SVG, lines,
headers, footers, notes and text boxes were drawn with the edit's styles (a pinned render
of an older version, a reflow measured from the version before a style edit). A comparison
across a style edit reports its change without the version before being digested first,
and a trial from a version before one is not laid out (nothing of it read in the current
version).

Lane RD3: `render` refuses in docx-agent's order. A sheet of named pages too tall even at the
narrowest tile, at the shortest page the sections declare (`w:pgSz`), is refused as `limit`
before whether the document has those pages (`not_found`), and before anything is laid out;
so are the image limits, and a `range` that does not resolve. The version is laid out only
when a call needs its pages.

Lane U2: a call is one commit, so one undo step and one state number. `word_drawings
insert_picture` (its own paragraph, the alignment, the picture, the float) and the calls that keep
values written by reference (`word_set_text`, `word_insert_text`, `word_replace_text`,
`word_insert_markdown`) now run their commands in one transaction. Before this they committed
several times, and a part that refused late left the earlier parts in the document. Debug builds
check that a call commits at most once.
`word_edit_table`'s rows written (`set_cells`, `insert_row`, `copy_row`): the text written into
an empty cell given its column's formatting now runs in the transaction of the call's command, so
the call is one commit, one undo step and one state number; the rows are kept in the session's
record only once the call is committed, by the ids it drew.

### Removed

P4e (lane CUT-B, part 2): the bridge's bindings are gone with the Python view they served --
`Session.update`, `commit`, `bridge`, `bridge_calls`, `seed_retired`, `rows_written`,
`note_rows_written` and `HostIds`. A host changes a Word document through the session's tools
(`Session.edit`). New: `Session.new_from_template` (a new document's bytes from a template's),
`StepGroup.reflow` (a step group's reflow, measured once before it closes; its ops measure none
of their own) and numeric chart categories.

Lane CUT2: the module is the Rust session's API alone. The prototype's functions and classes
are gone with the crates behind them (`crates/core`, `crates/word`): `Snap`, `Owner`,
`SnapshotE`, `Parser`, `open_document`, `commit`, `call`, `close_document`, `check`,
`field_pages`, `documents`, `spin`, `spin_held`, `noop`, `opcodes`, the tree and body digests
(`tree_digests`, `unit_starts`, `index_walk`, `index_walks`, `body_others`, `BodySpans`,
`body_blocks`, `common_ends`, `common_prefix_of`, `common_suffix_of`, `Starts`, `write_plan`,
`write_assemble`). `Session.number_version` is gone too: the session numbers each step it
ends (a call, a step group, an undo or redo, a revert) in its record of retired ids itself.

## 0.9.0 -- released in v2026.11.17.1

- **`ooxml_core.Session`**: the architecture's Rust session (lane P0d), beside the
  prototype's functions in the same module. `open(data, kind=, name=)` gives a document id
  (`d1`); `call(name, arguments_json, context=, limit=)` runs a tool on the document's head
  snapshot and returns `(reply_json, images)` -- refusals are replies, with their way out;
  `update(doc, {part: bytes or None}, context=)` commits a version's touched parts as one
  step, `(version, changed, attempts)`; `save`, `close`, `version`, `documents`,
  `tool_specs(kind)`, `stats`, `store_stats`, `shutdown`. Every method releases the
  interpreter lock for its Rust work; any number of threads may share a session, on a GIL
  or a free-threaded build. The one tool so far is `describe` over package parts (its
  name, compatibility mode, core properties, comments, notes and content controls).
- **`ooxml_core.CallContext(caller=, cancel=, deadline=)`** and **`ooxml_core.CancelToken`**:
  who calls, and how a call is stopped: at its next safe point, with `timeout`, changing
  nothing.
- **`ooxml_core.tool_specs(kind)`**: the tools' definitions as JSON, for the Python
  adapters.

## 0.8.1 -- unreleased

- The Word parse says a table cell's conditional formats as docx2svg 0.13.0 does (round 58,
  measured in Word): row band before column band, the bands as positions (`hBand:<n>`,
  `vBand:<n>`) named against the style's band sizes by the resolver, every leading header
  row (`w:tblHeader`) the header row, no `wholeTable`.

## 0.8.0 -- unreleased

- **`ooxml_core.BodySpans`**: a Word body's top-level blocks (`w:p`, `w:tbl`, `w:sdt`,
  `w:customXml`) found in its part's bytes by their tags (`scan`), and the next version's
  found from them (`advance`: the bytes alike at the start and the end, the blocks kept on
  either side, only the stretch between read again) -- docx-agent's `_scan`, `_blocks`,
  `_rescan` and `_kept_counts`, step for step. A step is kept by the version it starts
  from (weakly: a version's reading keeps no later version alive), so every pass of the
  next version takes the same one. Each block's paragraph,
  table and row counts (`counts`, `count_sum`, `stretch_counts`), and whether two
  versions' bytes outside the blocks are the same (`same_gaps`). The work is done with the
  interpreter lock let go; no Python object is made per block.
- **`common_ends`, `common_prefix_of`, `common_suffix_of`**: how many bytes two byte
  strings share at the start and the end, the lock let go.
- **`write_plan`, `write_assemble`, `Starts`**: a part written from its last bytes -- the
  body's children whose digests are not among the last write's named, and the new bytes
  and the children's places put together from the last bytes and the children written
  again (filled with the lock let go).
- **`index_walks(root)`**: `index_walk`'s digest and each top-level block's own walk
  digest and place among the body's children, and whether anything outside the blocks has
  a paragraph, row or table. **`body_others(body)`**: the places of a body's children that
  are not blocks.
- The tree walks (`tree_digests`, `index_walk`) read a namespace's prefix and URI once per
  namespace and walk, not once per element.
- The Word parser's change set (`Parser.splice`) counts list labels from the change on
  (the counters before each unit kept, the count stopping where they are as they were),
  keeps its units' keys and `REF` targets counted per change, and takes a unit kept within
  the change whole where the fields open before it are those it was read with. With
  `DOCX_AGENT_FULL_VALIDATE=1` each is compared with the whole reading. Its statistics give
  each step's time (`phases`).

## 0.7.0 -- unreleased

- **`ooxml_core.tree_digests(root, body)`**: digests of an lxml tree read from libxml2's
  nodes (lxml's public `struct LxmlElement`, libxml2's `xmlNode`), of everything libxml2's
  serializer writes: one per child of `body` as lxml counts children (the child, its subtree
  and its tail), one for the rest of the part; 128 bits each (two foldhash hashes seeded at
  random per process), the children read side by side on two threads of the core's own
  (made again in a forked process), the interpreter lock let go.
  `None` where a node is of a kind it does not read. docx-agent 0.1.120 writes again only
  the blocks whose digests changed.
- **`ooxml_core.unit_starts(data, start, end)`**: where each child of a body begins in the
  bytes libxml2 wrote (its tail runs to the next).
- **`ooxml_core.index_walk(root)`**: a digest of what docx-agent's index of a story reads --
  every paragraph, row and table not under `mc:Fallback`, by node, kind, `w14:paraId` and
  a row's table -- for telling the index unchanged without a walk in Python.
- **`Parser.faces_first(names)`**: of `names`, the faces the units' text is measured in, in
  the order the units first name them -- what a check says of the substituted faces, kept
  per state by docx-agent 0.1.120 without making the exporter's tables whole; and
  **`Parser.glyph_reads(pairs)`**: per `(face, character)`, whether the body holds the
  character and the face draws it (the exporter's `chars` and `drawn`).

## 0.6.0 -- unreleased

- **`ooxml_core.Parser`: a Word document's body read in Rust** (`ooxml_word::parse`, the
  XML reader in `ooxml_core::xml`). `Parser.open(document_xml, resolver)` reads the body --
  paragraphs, tables, runs, the complex fields threaded through the body, bookmarks, `REF`
  fields replaced by their bookmarks' text, list labels counted -- and makes the owner's
  first input as docx-agent's mirror exports it, byte for byte; `splice(lo, hi, elements,
  version, more, resolver)` makes each change set from the elements an edit changed (the
  fields threaded through them again, `REF` fields computed again where a bookmark they
  show changed, the list labels counted again, the neighbours made again), as the mirror's
  `splice` does. What a block's formatting resolves to (the style cascade, a run's
  character formats, geometry, borders, a table's grid) and what the parser does not read
  itself are asked of `resolver(kind, argument)` once per distinct input. A cell
  paragraph's autofit extent is measured with the core's own kernel. Anything the parser
  does not read refuses the document whole (`None`, and why).

## 0.5.0 -- unreleased

- **`ooxml_core.opcodes(a, b, width)`**: Python's
  `difflib.SequenceMatcher(None, a, b, autojunk=False).get_opcodes()` over two sequences
  of `width`-byte keys given joined, ported step for step (`ooxml_core::seqmatch`: the same
  longest matches in the same order, joined the same way) -- the same opcodes, in
  milliseconds for 30,000 keys where difflib takes a tenth of a second or more; the
  interpreter lock let go. docx-agent 0.1.118 compares a version's blocks with the
  version before here (the touched heading sections of a call's result). Tested against
  difflib on random sequences with many repeated keys.

## 0.4.0 -- unreleased

- **`ooxml_core.field_pages(doc, version, keys, info=False)`**: what a fields update reads
  of one laid-out state of a document opened with `open_document`, at exactly that version
  (a commit's sequence number; the interpreter lock let go throughout): the first page
  (1-based, blank pages counted) each paragraph or table key is drawn on, `null` for one
  not drawn, the page count, and with `info` every page's `[number, section, story kind
  (0 default, 1 first, 2 even), blank]` (`ooxml_word::fieldpages`). `None` where that
  version's snapshot is no longer kept. docx-agent 0.1.117's `word_fields` update reads
  its pages here.

## 0.3.0 -- unreleased

- **A snapshot no longer the latest lets go of what a check made of it.** The owner tells
  the format when it publishes a newer snapshot (`Format::retire`); a Word snapshot then
  drops its blocks' and paragraphs' key maps (made again if a check of that version comes),
  so the 64 kept snapshots no longer each hold their own (several MB each at ~3,000 pages).

## 0.2.0 -- unreleased

- **`ooxml_core.check(doc, version, request, seen=None)`**: docx-agent's check of some
  ranges, answered on the snapshot of exactly that version of a document opened with
  `open_document` (the interpreter lock let go throughout): the ranges' pages, extent,
  breaks and keeps, `since_last_check` against the snapshot `seen`, the layout facts
  (`ooxml_word::facts`: docx-agent's `layout_facts` rules, ported one for one, reading the
  range's pages and the records docx-agent made of each block), the fields facts, and the
  pages of some blocks. Returns the answer (JSON) and the snapshot, or `(None, None)` where
  that version's snapshot is no longer kept. What it does not port answers
  `{"fallback": why}` and docx-agent checks its own way.
- **Each version is laid out and published on its own** (a Word document's owner): a
  check reads exactly the version it asks for; the latest 64 snapshots are kept.
- The export and change sets carry each block's record (`rec`), what a paragraph, a run and
  a table draw (`draw`, for `since_last_check`) and the document's constants; a change set
  may carry new constants and sections.
- `open_document` reports a document it cannot read or lay out as an error rather than
  stopping the process.
- JSON keeps the order docx-agent writes its results in (serde_json's `preserve_order`).

## 0.1.0

- The first version: Word's layout (measure, paginate, the page-number fields) behind a
  document's owner, as moved in from the performance lanes' prototype.
