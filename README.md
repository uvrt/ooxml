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
  nightly run and a manual run take everything. Branch protection requires the jobs
  `required (ubuntu, build)` and `required (windows)`, which pass when every leg they
  cover passed or was left out.
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
