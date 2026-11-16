# ooxml

Reading, laying out, rendering and editing Office Open XML documents (Word, PowerPoint,
Excel), in one repository.

The packages keep their distribution and import names. Each lives under `python/` with
its own `pyproject.toml`, version, changelog and tests:

| Distribution | Import | What it is |
|---|---|---|
| ooxml-common | `ooxml_common` | shared OOXML reading, units, fonts and text measuring |
| ooxml-edit | `ooxml_edit` | the editing layer the agents are built on |
| docx2svg | `docx2svg` | Word layout and rendering to SVG/PNG |
| pptx2svg | `pptx2svg` | PowerPoint rendering to SVG/PNG (with `pptx2svg-fonts`, its font bundle) |
| docx-agent | `docx_agent` | tools for editing Word documents |
| pptx-agent | `pptx_agent` | tools for editing PowerPoint decks |
| xlsx-agent | `xlsx_agent` | tools for editing Excel workbooks |
| ooxml-core | `ooxml_core` | the Rust core (below), as a Python module; nothing imports it yet |

## The Rust core

A Cargo workspace at the root, its crates under `crates/`:

- `crates/core` (`ooxml-core`): format-neutral -- one owner per document fed by change sets,
  immutable snapshots, exact units, font metrics and measuring;
- `crates/word` (`ooxml-word`): Word's layout on the core -- line breaking, pagination,
  tables, field passes, incremental relayout -- equal to docx2svg's, and the `word-check`
  binary that compares the two;
- `crates/python`: the PyO3 module `ooxml_core` (`open_document`, `commit`, `call`,
  `close_document`), built by maturin from `python/ooxml-core`.

`cargo test --workspace` runs the Rust tests; `python/ooxml-core/tests` compares the Rust
layout with docx2svg's on made-up documents (`cargo build --release -p ooxml-word`, `pip
install ./python/ooxml-core`, then `pytest` there). Wheels: one abi3 wheel per platform for
CPython 3.10 and later, and one for the free-threaded 3.15t.

## Installing

Prebuilt packages are on this repository's
[GitHub Releases](https://github.com/uvrt/ooxml/releases), not on PyPI. Each release is
tagged with its date, `v<YYYY>.<MM>.<DD>` (`.<N>` for a second one that day), and carries
a wheel and an sdist of every package at the version its notes list. Point pip at the
release's page; the packages a package needs from this repository resolve from the same
page, and third-party dependencies (lxml, ...) from PyPI:

```
pip install --find-links https://github.com/uvrt/ooxml/releases/expanded_assets/<tag> docx-agent==<version>
```

for example `.../expanded_assets/v2026.11.20 docx-agent==0.1.113`. Pin the version with
`==` so pip takes the release's file. Extras work as before (`docx-agent[png]`,
`pptx-agent[png]`, `pptx2svg[fonts]`).

From a checkout, install the packages you need from `python/`, dependencies first, for
example `pip install ./python/ooxml-common ./python/ooxml-edit ./python/docx2svg -e
./python/docx-agent[dev]`.

## Development

- Tests: each package's suite runs from its own folder, `cd python/<package> && python -m
  pytest`, as it did in its own repository.
- CI (`.github/workflows/ci.yml`): on a pull request, only the packages the change touches
  and the packages that depend on them run (`tools/ci.py` holds the dependency graph and
  each package's install and test command; `python tools/ci.py affected <paths>` shows
  what a change selects). A change to `.github/` or `tools/` runs everything; main, the
  nightly run and a manual run take every package. Production runs CPython 3.15 (moving
  to the free-threaded 3.15t) and development runs on macOS, so a pull request and a push
  to main run Linux and Windows on 3.15 and no macOS; the nightly run and a manual run
  run Linux on every Python from the package's minimum (3.10 or 3.11), Windows and macOS
  on 3.15 (`tools/ci.py`, `LEGS`; its tests: `python -m unittest discover -s tools`). The
  free-threaded legs (`.github/workflows/freethreading.yml`, 3.15t) run on Linux on every
  event. A newer push to a pull request or to main cancels the older push's run.
  Branch protection requires the jobs `required (ubuntu, build)` and `required
  (windows)`, which pass when every leg they cover passed or was left out; the macOS and
  free-threaded legs are not required.
- Rust (`.github/workflows/rust.yml`): fmt, clippy, tests, the differential tests and the
  wheels, when a change touches the workspace, ooxml-core or the packages it is compared
  with; gate job `required (rust)`.
- Releases (`.github/workflows/release.yml`): pushing a date tag builds and checks every
  distribution and creates the GitHub Release; a manual run only builds and checks.

## History

Each package was brought in from its former repository with a merge commit
(`Import <package> at <sha> with its history`) that leaves the old commits as they were.
The old history is that merge's second parent, with the paths as they were then (at the
root, not under `python/<package>/`):

```
git log --oneline <import-merge>^2            # the package's history before the import
git log --oneline -- python/<package>         # its history since
```

Until the switch, fixes that still land in the old repositories come in with
`tools/sync-from-old.sh <package>`, again as a merge commit.
