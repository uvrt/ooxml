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

- `crates/python`: the PyO3 module `ooxml_core` -- the Rust session's API
  (ooxml-python-session: `Session`, `CallOptions`, `tool_specs`, ...), built by maturin from
  `python/ooxml-core`.

`cargo test --workspace` runs the Rust tests; `python/ooxml-core/tests` tests the module's
session API (`pip install ./python/ooxml-core`, then `pytest` there). Wheels, for Linux x86_64 and aarch64 only (production
runs Linux): one abi3 wheel per platform for CPython 3.10 and later, and one for the
free-threaded 3.15t. On macOS and Windows, pip builds ooxml-core from its sdist, which
needs a Rust toolchain.

## Installing

Prebuilt packages are on this repository's
[GitHub Releases](https://github.com/uvrt/ooxml/releases), not on PyPI. Each release is
tagged with its date, `v<YYYY>.<MM>.<DD>` (`.<N>` for a second one that day), and carries
a wheel and an sdist of every package at the version its notes list.

The newest release: point pip at the package index, one fixed address that lists every
release's files (published to GitHub Pages by the release workflow; the links point at the
release assets). Pip takes the newest version; the packages a package needs from this
repository resolve from the index, and third-party dependencies (lxml, ...) from PyPI:

```
pip install --extra-index-url https://uvrt.github.io/ooxml/simple/ docx-agent
```

`pip install --upgrade` with the same option moves to the newest release.

**Prefer uv where you can.** These package names are not on PyPI. With `--extra-index-url`,
pip looks in both places and takes the highest version it finds. So if someone registered
one of these names on PyPI with a higher version, pip would install theirs (dependency
confusion). uv takes a package from the first index that has it, and an index given with
`--index` comes before PyPI. Our packages then always come from this index:

```
uv pip install --index https://uvrt.github.io/ooxml/simple/ docx-agent
```

A given release (to pin): point pip at that release's page and pin the version with `==`
so pip takes the release's file:

```
pip install --find-links https://github.com/uvrt/ooxml/releases/expanded_assets/<tag> docx-agent==<version>
```

for example `.../expanded_assets/v2026.11.20 docx-agent==0.1.113`. `docx-agent==<version>`
with the index works too, as it lists every release. Extras work as before
(`docx-agent[png]`, `pptx-agent[png]`, `pptx2svg[fonts]`).

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
  nightly run and a manual run take every package. Production runs CPython 3.15 on Linux
  (moving to the free-threaded 3.15t) and development runs on macOS, so a pull request and
  a push to main run Linux on 3.15, Windows on 3.15 only for the packages with code of
  their own there (ooxml-common, ooxml-edit: `windows_smoke`), and no macOS; the nightly
  run and a manual run run Linux on every Python from the package's minimum (3.10 or
  3.11), Windows and macOS on 3.15 for every package (`tools/ci.py`, `LEGS`; its tests:
  `python -m unittest discover -s tools`). The free-threaded legs
  (`.github/workflows/freethreading.yml`, 3.15t) run on Linux on every event. docx-agent's
  suite runs on Linux as four jobs, split by test file on the times recorded in
  `tools/ci_durations.json` (`python tools/ci.py shard docx-agent K/4` lists a shard's
  files; when the shards drift apart, download a run's `junit-*` artifacts and run
  `python tools/ci.py durations docx-agent junit-*/junit.xml`, with `--free-threading` for
  freethreading.yml's: those legs are split on their own times). A newer push to a pull
  request or to main cancels the older push's run.
  Branch protection requires the jobs `required (ubuntu, build)` and `required
  (windows)`, which pass when every leg they cover passed or was left out; the macOS and
  free-threaded legs are not required.
- Rust (`.github/workflows/rust.yml`): fmt, clippy, tests, the differential tests and the
  wheels, when a change touches the workspace, ooxml-core or the packages it is compared
  with; gate job `required (rust)`. The architecture's crates add jobs that run on a pull
  request only when it changes their own paths (`tools/ci.py`, `RUST_JOBS`), and on main,
  nightly and manual runs: `loom` (ooxml-store's commit path), `oracles`
  (`tools/rust_oracles.py`: the XML writer and the package save against ooxml-edit),
  `word-model` (the Word model against docx2svg, on the repository's documents and those
  docx2svg's `tools/make_*.py` make) and `cargo-deny` (`deny.toml`: the licences
  `DEPENDENCIES.md` allows), all under `required (rust)`; and `fuzz` (the three targets in
  `fuzz/`, 5 minutes each, nightly and manual runs, not on a push to main), which is not
  required. ooxml-oracle's `text-diff` (the text engine against docx2svg's records) needs
  Word's own fonts, so it is not in CI: it stays a local gate for changes to ooxml-fonts and
  ooxml-text.
- Changelogs (`tools/changelog.py`): a pull request never edits a package's `CHANGELOG.md`
  (two that do always conflict); it adds a fragment, `python/<package>/changes/<topic>.<kind>.md`,
  `topic` its branch or subject (lower case, digits, `-`), `kind` one of `added`, `changed`,
  `fixed`, `removed`, holding the entry as the changelog would. CI's plan job refuses a
  `CHANGELOG.md` change on any branch but a release branch (`release/<tag>`); the build job
  checks each version against its changelog and fragments (`python tools/changelog.py check`).
  A version is still raised in the pull request that needs it.
- Releases (`.github/workflows/release.yml`): pushing a date tag builds and checks every
  distribution, creates the GitHub Release and deploys the package index of every release
  to GitHub Pages (`tools/simple_index.py`, tests `tools/test_simple_index.py`); a manual
  run only builds and checks, the index install included. To cut release `<tag>`
  (`v<YYYY>.<MM>.<DD>`, `.<N>` for a second the same day): on a branch `release/<tag>` from
  main, `python tools/changelog.py release <tag>` writes every package's fragments into its
  `CHANGELOG.md` under its version and deletes them; merge that pull request, then tag the
  merge commit. The release workflow refuses a tag while fragments are waiting
  (`tools/release_notes.py`).

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

## Architecture

The approved design for moving the document work into one shared Rust core, with Python as a thin
layer, is in [docs/architecture/README.md](docs/architecture/README.md).
