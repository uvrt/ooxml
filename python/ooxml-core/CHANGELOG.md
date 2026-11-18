# Changelog

ooxml-core is the Rust core of the ooxml packages (`crates/`), as the Python module
`ooxml_core`. Its version is the `crates/python` crate's.

## 0.10.0 -- unreleased

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
