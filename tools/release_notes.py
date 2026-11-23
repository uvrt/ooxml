"""Release notes for a GitHub Release of this repository.

    python tools/release_notes.py TAG [DIST_DIR] [--since REF] [--preview]

Prints Markdown, for the team that calls the tools:

- **Removed, and what to call instead**: each package's ``### Removed`` entries in its
  CHANGELOG.md section for TAG (``## <version> -- TAG``), and the ``*.removed.md`` fragments
  still waiting (a preview); ``tools/changelog.py check`` holds every such fragment to an
  ``Instead:`` line.
- **Tool definitions that changed**: the tools and top-level arguments the Word tools'
  exported definitions (SPECS, one JSON per tool, held to the registry by ooxml-session's
  tests) lost or gained since REF (default: the release tag before TAG, or before HEAD when
  TAG is not tagged yet).
- **Refused at this release, returning later**: the forms ``[[not_supported]]`` in KNOWN
  names, each with its cause; the refusal itself names the call that works.
- links to the support matrices (SUPPORTED);
- a table of the distributions the release carries (the seven packages, the font bundle,
  and ooxml-core once it is built), each with the version its pyproject.toml names, then how
  to install from the release.

With DIST_DIR, every distribution named must have a wheel and an sdist there, so the notes
never list a file the release lacks.  For a release tag (TAG_PATTERN, as release.yml's
tags), no changelog fragment may be waiting: a release is cut after
`tools/changelog.py release TAG` wrote them into the CHANGELOG.md files (README.md,
Development); ``--preview`` (the notes sent to the team the day before the tag) lets them
wait.

Standard library only.
"""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
import tomllib
from pathlib import Path

import changelog

ROOT = Path(__file__).resolve().parent.parent
REPO = "uvrt/ooxml"
INDEX = "https://uvrt.github.io/ooxml/simple/"  # GitHub Pages, tools/simple_index.py
#: release.yml's tags: `v<YYYY>.<MM>.<DD>`, `.<N>` for a second release the same day.
TAG_PATTERN = re.compile(r"v\d{4}\.\d\d\.\d\d(\.\d+)?")
#: The Word tools' exported definitions, one ``<tool>.json`` each.
SPECS = "crates/ooxml-tools-word/tests/golden"
#: The forms the tools refuse as not supported yet (``[[not_supported]]``).
KNOWN = "crates/ooxml-oracle/known-differences.toml"
#: The support matrices the notes link to.
SUPPORTED = ["python/docx-agent/src/docx_agent/tools/SUPPORTED.md",
             "python/pptx-agent/src/pptx_agent/tools/SUPPORTED.md"]

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


def _git(*args: str) -> str | None:
    run = subprocess.run(["git", *args], check=False, cwd=ROOT, capture_output=True, text=True)
    return run.stdout if run.returncode == 0 else None


def removed_entries(tag: str, folders: list[Path]) -> list[tuple[str, str]]:
    """``(package, entry)`` for each ``### Removed`` text in a package's section for ``tag``
    and each ``*.removed.md`` fragment still waiting."""
    out = []
    heading = re.compile(r"^## \S+ -- " + re.escape(tag) + r"\s*$")
    for folder in folders:
        lines = (folder / "CHANGELOG.md").read_text(encoding="utf-8").splitlines()
        start = next((i for i, line in enumerate(lines) if heading.match(line)), None)
        if start is not None:
            end = next((i for i in range(start + 1, len(lines)) if lines[i].startswith("## ")), len(lines))
            body = lines[start + 1:end]
            at = next((i for i, line in enumerate(body) if line.strip() == "### Removed"), None)
            if at is not None:
                stop = next((i for i in range(at + 1, len(body)) if body[i].startswith("### ")), len(body))
                text = "\n".join(body[at + 1:stop]).strip()
                if text:
                    out.append((folder.name, text))
        for fragment in changelog.fragments(folder)[0]:
            if fragment.kind == "removed":
                out.append((folder.name, fragment.path.read_text(encoding="utf-8").strip()))
    return out


def specs_at(ref: str) -> dict[str, set[str]]:
    """Each Word tool's top-level arguments in its exported definition at ``ref`` (read with
    one ``git cat-file --batch``)."""
    listing = _git("ls-tree", "--name-only", f"{ref}:{SPECS}")
    names = [name for name in (listing or "").split() if name.endswith(".json")]
    if not names:
        return {}
    request = "".join(f"{ref}:{SPECS}/{name}\n" for name in names).encode()
    run = subprocess.run(["git", "cat-file", "--batch"], check=False, cwd=ROOT, input=request, capture_output=True)
    data, at, out = run.stdout, 0, {}
    for name in names:
        end = data.index(b"\n", at)
        header = data[at:end].split()
        at = end + 1
        if len(header) != 3:  # "<ref:path> missing"
            continue
        size = int(header[2])
        text, at = data[at:at + size], at + size + 1
        try:
            spec = json.loads(text)
        except (json.JSONDecodeError, UnicodeDecodeError):
            continue
        if isinstance(spec, dict) and spec.get("name") == name[:-5] and isinstance(spec.get("schema"), dict):
            out[name[:-5]] = set(spec["schema"].get("properties", {}))
    return out


def spec_changes(before: dict[str, set[str]], after: dict[str, set[str]]) -> list[str]:
    """The tools and arguments gone or new between two definitions' sets, as list lines."""
    lines = [f"- `{name}`: withdrawn" for name in sorted(set(before) - set(after))]
    for name in sorted(set(before) & set(after)):
        lines += [f"- `{name}`: argument `{arg}` removed" for arg in sorted(before[name] - after[name])]
        lines += [f"- `{name}`: argument `{arg}` added" for arg in sorted(after[name] - before[name])]
    lines += [f"- `{name}`: new tool" for name in sorted(set(after) - set(before))]
    return lines


def refused(path: Path) -> list[tuple[str, str]]:
    """``(form, cause)`` of each ``[[not_supported]]`` entry, the cause without its
    parenthesised note of who ports it."""
    entries = tomllib.loads(path.read_text(encoding="utf-8")).get("not_supported", [])
    return [(e["form"], re.sub(r"\s*\([^()]*\)\s*$", "", " ".join(e.get("cause", "").split())))
            for e in entries if "form" in e]


def changes(tag: str, since: str | None) -> str:
    """The notes' sections above the versions table."""
    tagged = _git("rev-parse", "--verify", "--quiet", f"refs/tags/{tag}") is not None
    head = tag if tagged else "HEAD"
    if since is None:
        found = _git("describe", "--tags", "--abbrev=0", "--match", "v[0-9]*", f"{head}^")
        since = found.strip() if found else None
    parts = ["## Removed, and what to call instead", ""]
    removed = removed_entries(tag, changelog.packages())
    parts += [f"**{package}**: {text}\n" for package, text in removed] or ["Nothing removed.", ""]
    parts += ["## Tool definitions that changed", ""]
    if since is None:
        parts += ["(no earlier release tag to compare with)", ""]
    else:
        diff = spec_changes(specs_at(since), specs_at(head))
        parts += [f"The Word tools' definitions against {since}:", "", *(diff or ["- none"]), ""]
    parts += ["## Refused at this release, returning later", "",
              "Each is refused `not_supported`; the refusal names the call that works instead.", ""]
    parts += [f"- `{form}`: {cause}" for form, cause in refused(ROOT / KNOWN)] or ["- none"]
    links = [f"[{Path(p).parts[1]}](https://github.com/{REPO}/blob/{tag if tagged else 'main'}/{p})"
             for p in SUPPORTED if (ROOT / p).exists()]
    parts += ["", "What each tool supports: " + ", ".join(links) + ".", ""]
    return "\n".join(parts) + "\n"


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
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("tag")
    parser.add_argument("dist", nargs="?", type=Path)
    parser.add_argument("--since", help="the release to compare the tools' definitions with")
    parser.add_argument("--preview", action="store_true", help="changelog fragments may still wait")
    args = parser.parse_args(argv)
    if TAG_PATTERN.fullmatch(args.tag) and not args.preview:
        waiting = changelog.pending(changelog.packages())
        if waiting:
            print(f"changelog fragments not yet in CHANGELOG.md (run tools/changelog.py release {args.tag} "
                  "on a release branch first):\n  " + "\n  ".join(str(p.relative_to(ROOT)) for p in waiting),
                  file=sys.stderr)
            return 1
    items = versions()
    if args.dist is not None:
        missing = check(args.dist, items)
        if missing:
            print("missing from the release:\n  " + "\n  ".join(missing), file=sys.stderr)
            return 1
    print(changes(args.tag, args.since) + notes(args.tag, items), end="")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
