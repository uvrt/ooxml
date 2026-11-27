# docx-agent

[![CI](https://github.com/uvrt/docx-agent/actions/workflows/ci.yml/badge.svg)](https://github.com/uvrt/docx-agent/actions/workflows/ci.yml)
[![License](https://img.shields.io/badge/license-MIT-green)](LICENSE)

An AI-editable Word (`.docx`) layer: a model reads a document, changes it (optionally as
tracked changes), checks the pages and saves it, through tool calls. The counterpart of
[`pptx-agent`](https://github.com/uvrt/pptx-agent) for Word.

Since 0.2.0 every Word document is held by the Rust host (built from this monorepo's crates,
reached through ooxml-edit's `Toolbox`): it opens, edits, lays out, checks, renders and saves the
document. docx-agent is the tool layer on it (`docx_agent.tools`, on
[`ooxml-edit`](https://github.com/uvrt/ooxml-edit)'s `ooxml_edit.tools`) and the live view
(`docx_agent.liveview`). The Python `Document` API of 0.1 is gone (CHANGELOG.md, 0.2.0).

To give a model the document as tool calls (Claude, or OpenAI's Responses and Chat
Completions APIs): [using the tools with a model provider](src/docx_agent/tools/README.md).

## Install

Not on PyPI yet. Python 3.10+. The siblings install from git:

```bash
pip install "ooxml-common @ git+https://github.com/uvrt/ooxml-common@main" \
            "ooxml-edit @ git+https://github.com/uvrt/ooxml-edit@main" \
            "docx2svg[png] @ git+https://github.com/uvrt/docx2svg@main"
pip install "docx-agent @ git+https://github.com/uvrt/docx-agent@main"
```

## Fonts

Rendering and reflow feedback lay the document out with the Rust session's page engine,
which measures every glyph with the face Word uses. Install the document's faces where the layout runs:

- **Office installed:** nothing to do.
- **No Office:** install the open metric compatible substitutes (Debian/Ubuntu: `sudo
  apt-get install fonts-crosextra-carlito fonts-crosextra-caladea fonts-liberation`;
  macOS: `brew install --cask font-carlito font-caladea font-liberation`). With Carlito in place
  of Calibri, and Liberation Sans, Serif and Mono in place of Arial, Times New Roman and
  Courier New, lines and pages break where they do in Word. Symbol and Wingdings bullets are laid out from
  metrics recorded from Word's copies.
- **Without a face and its substitute** (Aptos, Cambria, Calibri Light...), the layout
  stops at the first paragraph in that face, and everything after it is unknown, never
  guessed.

The layout reports every substitution, and never uses a substitute when the real face is
installed.

`check` (with `reflow`), `render` and `save_document` return the layout's **coverage**,
so a check that passed is told apart from one that could not see everything. It holds:

- `complete`
- the pages laid out, with Word's saved page count when the layout stopped
- the blocks laid out, out of the total
- where the layout stopped, by block id, and header and footer stops
- the placeholders: what was drawn as a marked box of its height instead of laid out, by
  page, reason and block id (any makes `complete` false)
- the approximations: what was laid out by a rule not measured against Word
- the faces substituted or missing

`check` with `layout` (or `min_size`, `styles`) measures the pages from that same layout --
text below a size by kind, direct formatting over the style with the call that clears it,
styles not in an allowed list, empty paragraphs used as space, keep-with-next paragraphs at
a page's foot, rows split across pages and header rows that do not repeat, table columns
that leave room beside one that wraps (with widths that fit), pictures and tables past the
margins, and the pages a reviewer cannot judge from the render -- as facts
with block ids and pages.
A Word toolbox is sent the shared tools with a document's parameters only: `check` has
`pages`, `include` (`validate`, `reflow`, `fields`, `layout`, `app`), `min_size` and
`styles`, and `find_text`, `replace_text` and `render` have no `slides` (ooxml-edit's
`Tool.kind_fields`), and `undo` no `scope`. A call that still passes a deck's parameter gets the answer it always
got (`render`'s and `check`'s `slides` read as `pages`, with a warning).

Refs (`$name`) live in the tools' session, not the document. Outside a call the application
reads them with `session.refs("d1")` (`{name: address}`, a copy) and finds the top-level
block holding one with `docx_agent.tools.block_of(session, "d1", "$intro")` or the blocks
between two with `blocks_between(session, "d1", "$intro", "$closing")`; a handler passes
its `call` instead (`block_of(call, "$intro")`).

The user's regional settings -- the separator a decimal tab stop aligns on, a chart's
decimal and thousands separators, the language of new chart text in a document that states
none, a new document's language and number separators -- are `Toolbox(locale=...)`, an `ooxml_common.locales.Locale`; en-US unless given, never the
server's own.

## Example

```python
from ooxml_edit.tools import Toolbox
from docx_agent.tools import FORMAT, GROUPS, TOOLS

with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS) as toolbox:
    session = toolbox.session()
    d1 = session.open(docx_bytes, name="agreement.docx")
    print(toolbox.dispatch(session, "word_read", {"doc": d1}).to_json())   # Markdown, an id before each block
    toolbox.dispatch(session, "word_set_text", {"doc": d1, "target": "p:3B212964",
                                                "text": "The supplier will host the portal for 36 months."})
    toolbox.dispatch(session, "save_document", {"doc": d1})
    for output in session.take_outputs():          # what save_document wrote, as bytes
        store(output.name, output.data)
```

A table or chart placed from a source by the tools (`from`) keeps where its values came
from in the saved file, in a custom XML item Word keeps.

More: [docs/tools.md](docs/tools.md).

## What is supported

- **Agent tools** (`docx_agent.tools`, on `ooxml_edit.tools`): the document through tool
  calls for a model; it never runs Python and never sees a path
  ([docs/tools.md](docs/tools.md)). What is supported and what is not:
  [SUPPORTED.md](src/docx_agent/tools/SUPPORTED.md); guidance for the application's
  thinking layer: [GUIDANCE.md](src/docx_agent/tools/GUIDANCE.md).

## Memory on Linux

A document's undo history, read copy and layouts live in the process that opened it, and
its layouts in the toolbox's workers. `Session.close` lets go of all of it, in both, and
hands the freed pages back to the system (`malloc_trim` on glibc). Two settings matter for
an application that keeps running:

- **`MALLOC_ARENA_MAX=2`** for a process whose threads edit documents (several writers and
  readers on one document, a live view beside them). glibc gives each thread an arena of
  its own (up to eight per core) and keeps what is freed in each; with two the process
  stays smaller. Measured on a made-up 340-page report (0.1.x, CHANGELOG.md; 12 writers, 2 readers, a live view, about 160 edits, Python 3.15): the calling process
  peaked at 1.84 GB with glibc's default arenas (31 of them) and at 1.35 GB with
  `MALLOC_ARENA_MAX=2` (6). Set it in the environment before Python starts.
- **Close documents you are done with** (`session.close(doc)`). What a closed document
  leaves is the allocators' free space: Python's own (`pymalloc`) keeps a 1 MB arena while
  any object in it lives, so the process does not shrink back to where it started; it
  reuses that space for the next document.

## Status

Pre-release (0.2.0); the tools may still change. Golden transcripts replay the end-to-end
trial's Word tasks with the tools alone to byte-identical outputs. The Microsoft Word
oracle tests are local-only (macOS with Word) and skip elsewhere, including CI.

## Documentation

- [docs/tools.md](docs/tools.md) -- the tool layer for a model
- [tools/README.md](src/docx_agent/tools/README.md), [SUPPORTED.md](src/docx_agent/tools/SUPPORTED.md), [GUIDANCE.md](src/docx_agent/tools/GUIDANCE.md) -- the agent tools
- [ROADMAP.md](ROADMAP.md) -- phases, decisions and what Word was measured to write
- [CHANGELOG.md](CHANGELOG.md), [CONTRIBUTING.md](CONTRIBUTING.md) (running the tests)

## Family

- [pptx2svg](https://github.com/uvrt/pptx2svg) -- renders PowerPoint (`.pptx`) slides to SVG and PNG.
- [docx2svg](https://github.com/uvrt/docx2svg) -- renders Word (`.docx`) documents to SVG, page by page.
- [ooxml-common](https://github.com/uvrt/ooxml-common) -- the format-neutral reading, DrawingML, fonts and text metrics both renderers share.
- [ooxml-edit](https://github.com/uvrt/ooxml-edit) -- lossless, undoable editing of OOXML packages, shared by both agent layers.
- [pptx-agent](https://github.com/uvrt/pptx-agent) -- an AI-editable PowerPoint layer: inspect, edit, re-render.
- [docx-agent](https://github.com/uvrt/docx-agent) (this repo) -- an AI-editable Word layer: tool calls on the Rust session, edits optionally as tracked changes.

## License

MIT, see [LICENSE](LICENSE). `tests/corpus/commonmark/` (examples of the CommonMark Spec)
is under CC BY-SA 4.0, and the third-party fixtures keep their own terms; each directory's
`PROVENANCE.md` says which.
