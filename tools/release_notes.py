"""Release notes for a GitHub Release of this repository: each distribution and its version.

    python tools/release_notes.py TAG [DIST_DIR]

Prints Markdown: a table of the distributions the release carries (the seven packages, the
font bundle, and ooxml-core once it is built), each with the version its pyproject.toml
names, then how to install from the release.  With DIST_DIR, every distribution named must
have a wheel and an sdist there, so the notes never list a file the release lacks.

Standard library only.
"""

from __future__ import annotations

import re
import sys
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
REPO = "uvrt/ooxml"
INDEX = "https://uvrt.github.io/ooxml/simple/"  # GitHub Pages, tools/simple_index.py

#: The distributions a release carries, in dependency order, by their pyproject.toml.
DISTRIBUTIONS = [
    "python/ooxml-common",
    "python/ooxml-edit",
    "python/docx2svg",
    "python/pptx2svg",
    "python/pptx2svg/packages/pptx2svg-fonts",
    "python/docx-agent",
    "python/pptx-agent",
    "python/xlsx-agent",
    "python/ooxml-core",
]


def versions() -> list[tuple[str, str]]:
    out = []
    for rel in DISTRIBUTIONS:
        path = ROOT / rel / "pyproject.toml"
        if not path.exists():
            continue
        data = tomllib.loads(path.read_text(encoding="utf-8"))
        project = data["project"]
        version = project.get("version")
        if version is None:  # maturin: the version of the crate its manifest-path names
            manifest = ROOT / rel / data["tool"]["maturin"]["manifest-path"]
            version = tomllib.loads(manifest.read_text(encoding="utf-8"))["package"]["version"]
        out.append((project["name"], version))
    return out


def _norm(name: str) -> str:
    return re.sub(r"[-_.]+", "_", name).lower()


def check(dist_dir: Path, items: list[tuple[str, str]]) -> list[str]:
    files = [p.name for p in dist_dir.iterdir()]
    missing = []
    for name, version in items:
        stem = f"{_norm(name)}-{version}"
        if not any(f.startswith(stem) and f.endswith(".whl") for f in files):
            missing.append(f"{name} {version}: no wheel")
        if not any(f == f"{stem}.tar.gz" for f in files):
            missing.append(f"{name} {version}: no sdist")
    return missing


def notes(tag: str, items: list[tuple[str, str]]) -> str:
    rows = "\n".join(f"| {name} | {version} |" for name, version in items)
    example = dict(items).get("docx-agent", "<version>")
    return f"""\
| Distribution | Version |
|---|---|
{rows}

Install from this release (third-party dependencies come from PyPI; the packages above
come from this page, and the ones they need from each other resolve from it too):

```
pip install --find-links https://github.com/{REPO}/releases/expanded_assets/{tag} docx-agent=={example}
```

The newest release, whichever it is, from the package index that lists every release:

```
pip install --extra-index-url {INDEX} docx-agent
```

With uv, which takes these packages only from the index (they are not on PyPI, and uv
does not let PyPI override an index given with `--index`):

```
uv pip install --index {INDEX} docx-agent
```
"""


def main(argv: list[str]) -> int:
    if not argv or len(argv) > 2:
        print(__doc__, file=sys.stderr)
        return 2
    items = versions()
    if len(argv) == 2:
        missing = check(Path(argv[1]), items)
        if missing:
            print("missing from the release:\n  " + "\n  ".join(missing), file=sys.stderr)
            return 1
    print(notes(argv[0], items), end="")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
