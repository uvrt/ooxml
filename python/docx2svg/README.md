# docx2svg

[![CI](https://github.com/uvrt/docx2svg/actions/workflows/ci.yml/badge.svg)](https://github.com/uvrt/docx2svg/actions/workflows/ci.yml)
[![License](https://img.shields.io/badge/license-MIT-green)](LICENSE)

Render Word (`.docx`) documents to **SVG** and **PNG**, page by page. A thin Python layer
over the Rust core ([`ooxml-core`](../ooxml-core)): the document is laid out and drawn by
the same engine docx-agent's `render` tool draws with, laid out as Word lays it out.

## Install

Python 3.10+, with [`ooxml-common`](../ooxml-common) and `ooxml-core` (the Rust core: Linux
wheels from the release; elsewhere built from source with a Rust toolchain).

```bash
pip install ooxml-common ooxml-core docx2svg      # from the release's files
```

## Example

```python
from docx2svg import convert_docx_to_svg, convert_docx_to_png, ConvertOptions

options = ConvertOptions()
svgs = convert_docx_to_svg("report.docx", options)   # one SVG string per page
pngs = convert_docx_to_png("report.docx")            # 300 dpi unless a size is given
for warning in options.warnings:                     # everything not drawn faithfully
    print(warning)
```

```bash
docx2svg report.docx -o out/ -f both --strict
```

Every drawn element carries `data-docx-path` back to the XML it came from
(`docx2svg.resolve_path` finds it), and `convert_docx(...).layout` is the laid-out
document itself (`ooxml_core.LaidPages`): [docs/api.md](docs/api.md). A document the
layout does not lay out yet raises `LayoutError` (a `NotImplementedError`), saying what
and what to do instead.

What Word takes from its machine rather than the file -- the character a decimal tab stop
aligns on, a chart's decimal and thousands separators -- comes from
`ConvertOptions(locale=...)` (an `ooxml_common.locales.Locale`), en-US unless given and
never read from the server ([docs/api.md](docs/api.md#locale)).

## What it draws

What docx-agent's `render` draws: see docx-agent's
[SUPPORTED.md](../docx-agent/src/docx_agent/tools/SUPPORTED.md) and
[docs/supported.md](docs/supported.md).

## Fonts

The layout measures every glyph with the face Word lays it out with, so it needs the
faces themselves: installed, or embedded in the document. Where a face is missing it is
laid out with a substitute, and said so: the measured ones below where they are installed,
else an approximate one (below), so a missing face never stops the layout.

**Open substitutes.** When an Office face is neither installed nor embedded, docx2svg
uses an open face whose advances and line metrics were measured equal to it, if that
face is installed:

| Office face | Substitute | Measured against Word's copy |
|---|---|---|
| Calibri | Carlito | the same advance for every character both have except 3 (2094 compared), the same line metrics |
| Arial | Liberation Sans | every Latin, Greek and Cyrillic advance the same, the same line metrics and kerning |
| Times New Roman | Liberation Serif | the same, likewise |
| Courier New | Liberation Mono | the same, likewise |
| Symbol, Wingdings | recorded metrics | the integers of Word's own copies, recorded as facts; bullets are drawn as their Unicode equivalents (•, ▪, ➢, ❖, ✓...) |

Every substitution is reported: a `font-substituted` warning, and an entry in the layout's
`coverage.substituted_fonts`. docx2svg never substitutes a face that is installed, because
fidelity is measured against Word's own faces. `ConvertOptions(substitute_fonts=False)`
(or `--no-substitute-fonts`) turns substitution off.

**Everything else has no measured substitute.** That includes Aptos (Word's default since
2023), Aptos Display, Calibri Light and Cambria: Aptos has no open clone, and the closest
open faces for the other three were measured and don't match. Gelasio is said to match
Georgia, but it hasn't been measured here, so it isn't used. Such a face is laid out with
the face PowerPoint rendering uses for it (pptx2svg's rule, shared: Carlito for Calibri
Light and Aptos, Caladea for Cambria), or that face's Office original (Calibri for Carlito),
else with the first installed face of its class (sans-serif, serif or monospace, from the
document's font table or the face's name) -- reported as *approximate*, naming both faces.
A character a face lacks is measured in another face (`font-glyph-fallback`). You can name
a substitute yourself, e.g.
`ConvertOptions(font_substitutes={"Cambria": "Caladea"})`. It is used and reported as
*approximate*, but expect different line and page breaks. Caladea's advances are 1.8 to
5.7% narrower than Cambria's and its line is 1.150 em against 1.172. With Caladea for
Cambria, `samplelib/sample-long.docx` lays out to 19 pages where Word draws 36. The
alternative is to install the real face: Aptos is a free download from Microsoft, and
Office installs the rest.

**Installing the substitutes** (they are not bundled):

```bash
# Debian / Ubuntu
sudo apt-get install fonts-crosextra-carlito fonts-crosextra-caladea fonts-liberation
# macOS (Homebrew)
brew install --cask font-carlito font-caladea font-liberation
# Windows: Windows has Calibri, Arial, Times New Roman and Courier New already.
#   Otherwise install Carlito (https://fonts.google.com/specimen/Carlito) and
#   Liberation (https://github.com/liberationfonts/liberation-fonts/releases) per user.
# Other Linux distributions package them as Carlito, Caladea and Liberation.
```

Faces in another folder -- an application's licensed fonts, say -- can be added with
`ConvertOptions(font_dirs=[...])` or `--font-dir`, or with the environment variable
**`OOXML_FONT_DIRS`** (folders separated by `os.pathsep`: `:` on macOS and Linux, `;` on
Windows), which pptx2svg reads too. The explicit argument wins over the variable; an
empty list means none. Either is added to the folders Word uses (searched after them),
never in their place, for layout and drawing alike, and the folders under it are read
too.

**Coverage.** Every conversion sets `ConvertOptions.coverage`, a summary of how much was
laid out: the pages and blocks laid out, any placeholders (what was drawn as a marked box
of its height instead of laid out, with the reason, page and element path), the faces
substituted or missing, and any approximations (places laid out by a rule not yet measured
against Word). `coverage.complete` is true only when nothing was drawn as a placeholder and
no face was missing; `coverage.status` is `complete`, `approximate` or `partial`. The CLI
prints `coverage.summary()` with the warnings.

## Status

Pre-alpha. The layout and drawing are the Rust core's (docx2svg's own Python layout was
withdrawn in lane svg-thin; its measurements against Word are kept under
`crates/ooxml-oracle/word/observations`).

## Documentation

- [docs/api.md](docs/api.md) -- the functions, options, element paths and the layout handle
- [docs/supported.md](docs/supported.md) -- what is drawn
- [docs/development.md](docs/development.md) -- running the tests
- [CHANGELOG.md](CHANGELOG.md), [CONTRIBUTING.md](CONTRIBUTING.md)

## Family

- [pptx2svg](https://github.com/uvrt/pptx2svg) -- renders PowerPoint (`.pptx`) slides to SVG and PNG.
- [docx2svg](https://github.com/uvrt/docx2svg) (this repo) -- renders Word (`.docx`) documents to SVG, page by page.
- [ooxml-common](https://github.com/uvrt/ooxml-common) -- the format-neutral reading, DrawingML, fonts and text metrics both renderers share.
- [ooxml-edit](https://github.com/uvrt/ooxml-edit) -- lossless, undoable editing of OOXML packages, shared by both agent layers.
- [pptx-agent](https://github.com/uvrt/pptx-agent) -- an AI-editable PowerPoint layer: inspect, edit, re-render.
- [docx-agent](https://github.com/uvrt/docx-agent) -- an AI-editable Word layer: inspect, edit (optionally as tracked changes), re-render.

## License

MIT, see [LICENSE](LICENSE). The third-party fixtures under `tests/fixtures/samplelib/`
and `tests/fixtures/wordto/` keep their own (permissive) terms, recorded in their
`PROVENANCE.md`. No font file is included.
