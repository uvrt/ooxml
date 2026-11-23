"""Release notes for a GitHub Release of this repository.

    python tools/release_notes.py TAG [DIST_DIR] [--since REF] [--preview]

Prints Markdown, for the team that calls the tools:

- **Removed, and what to call instead**: each package's ``### Removed`` entries in its
  CHANGELOG.md section for TAG (``## <version> -- TAG``), and the ``*.removed.md`` fragments
  still waiting (a preview); ``tools/changelog.py check`` holds every such fragment to an
  ``Instead:`` line.
- **Changed and fixed**: per package (TOOL_PACKAGES first), one line for each entry of
  its ``### Fixed``, ``### Added`` and ``### Changed`` sections for TAG and of its fragments
  still waiting (a preview), in that order, an entry that says "Nothing the package does
  changes" last: its bold lead, else its first sentence, without parentheses (headline).
- **Tool definitions that changed**: per package, the tools and top-level arguments its
  committed definitions (SPECS, one JSON per tool, each folder held to what the package
  sends by a test) lost or gained since REF (default: the release tag before TAG, or before
  HEAD when TAG is not tagged yet); a package with no definitions at REF gets its count.
- **Refused at this release, returning later**: the forms ``[[not_supported]]`` in KNOWN
  names that a call through docx-agent's Toolbox is refused in, each said for the team
  (release_refusals.READER, not the cause written for whoever ports it); the refusal itself
  names the call that works.
- links to the support matrices (SUPPORTED);
- a table of the distributions the release carries (the seven packages, the font bundle,
  and ooxml-core once it is built), each with the version its pyproject.toml names, then how
  to install from the release.  A preview shows the versions ``tools/changelog.py release``
  will set (from the newest release tag, or ``--since``), not today's.

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
import release_refusals

ROOT = Path(__file__).resolve().parent.parent
REPO = "uvrt/ooxml"
INDEX = "https://uvrt.github.io/ooxml/simple/"  # GitHub Pages, tools/simple_index.py
#: release.yml's tags: `v<YYYY>.<MM>.<DD>`, `.<N>` for a second release the same day.
TAG_PATTERN = re.compile(r"v\d{4}\.\d\d\.\d\d(\.\d+)?")
#: Each package's committed tool definitions, one ``<tool>.json`` each: the Word tools' Rust
#: exports (ooxml-session's tests/integration/registry.rs), the others' Claude definitions
#: (their tests/test_tool_specs.py).
SPECS = {"docx-agent": "crates/ooxml-tools-word/tests/golden",
         "pptx-agent": "python/pptx-agent/tests/specs",
         "xlsx-agent": "python/xlsx-agent/tests/specs"}
#: The forms the tools refuse as not supported yet (``[[not_supported]]``).
KNOWN = "crates/ooxml-oracle/known-differences.toml"
#: The support matrices the notes link to.
SUPPORTED = ["python/docx-agent/src/docx_agent/tools/SUPPORTED.md",
             "python/pptx-agent/src/pptx_agent/tools/SUPPORTED.md"]

#: The packages the team calls, and the Rust core their tools run on: their changes lead
#: "Changed and fixed".
TOOL_PACKAGES = ("docx-agent", "pptx-agent", "xlsx-agent", "ooxml-core")
#: The kinds "Changed and fixed" lists, most visible to a caller first (``removed`` has its
#: own section).
CHANGED_KINDS = ("fixed", "added", "changed")
#: What an entry that changes nothing a caller sees says: it is listed last.
INTERNAL = "Nothing the package does changes"

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


def versions(raised: dict[str, str] | None = None) -> list[tuple[str, str]]:
    """Each distribution's name and version: its pyproject.toml's, or ``raised``'s by name."""
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
        out.append((project["name"], (raised or {}).get(project["name"], version)))
    return out


def release_versions(since: str | None, folders: list[Path] | None = None) -> dict[str, str]:
    """The version ``tools/changelog.py release`` sets, by package name, of each package with
    fragments waiting among ``folders`` (default: every package): from its version at
    ``since`` (default: the newest release tag)."""
    folders = changelog.packages() if folders is None else folders
    since = since or changelog.last_release()
    bases = changelog.bases_at(since, folders) if since else {}
    return {changelog.project_name(folder): new
            for folder, (_, new) in changelog.release_versions(folders, bases).items()}


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


def tag_section(folder: Path, tag: str) -> dict[str, list[str]]:
    """The lines under each ``### <Kind>`` heading of the package's CHANGELOG.md section for
    ``tag`` (``## <version> -- <tag>``), by kind in lower case; empty without that section."""
    heading = re.compile(r"^## \S+ -- " + re.escape(tag) + r"\s*$")
    lines = (folder / "CHANGELOG.md").read_text(encoding="utf-8").splitlines()
    start = next((i for i, line in enumerate(lines) if heading.match(line)), None)
    if start is None:
        return {}
    end = next((i for i in range(start + 1, len(lines)) if lines[i].startswith("## ")), len(lines))
    out: dict[str, list[str]] = {}
    kind = None
    for line in lines[start + 1:end]:
        if line.startswith("### "):
            kind = line[4:].strip().lower()
            out.setdefault(kind, [])
        elif kind is not None:
            out[kind].append(line)
    return out


def removed_entries(tag: str, folders: list[Path]) -> list[tuple[str, str]]:
    """``(package, entry)`` for each ``### Removed`` text in a package's section for ``tag``
    and each ``*.removed.md`` fragment still waiting."""
    out = []
    for folder in folders:
        text = "\n".join(tag_section(folder, tag).get("removed", [])).strip()
        if text:
            out.append((folder.name, text))
        for fragment in changelog.fragments(folder)[0]:
            if fragment.kind == "removed":
                out.append((folder.name, fragment.path.read_text(encoding="utf-8").strip()))
    return out


def entries(lines: list[str]) -> list[str]:
    """The entries of a section's or a fragment's lines, each on one line: each top-level list
    item (with the lines indented under it) and each other paragraph."""
    out: list[str] = []
    last = None  # what out[-1] is: "item", "paragraph", or None once a blank line closed it
    for line in lines:
        if not line.strip():
            last = "item" if last == "item" else None  # an indented line may still continue an item
        elif line.startswith(("- ", "* ")):
            out.append(line[2:])
            last = "item"
        elif (last == "item" and line[0].isspace()) or last == "paragraph":
            out[-1] += " " + line
        else:
            out.append(line)
            last = "paragraph"
    return [" ".join(entry.split()) for entry in out]


def _plain(text: str) -> str:
    """``text`` without its parenthesised asides (outside code spans) and the space before them."""
    out: list[str] = []
    depth, code = 0, False
    for char in text:
        if char == "`" and depth == 0:
            code = not code
        elif not code and char == "(":
            depth += 1
            if depth == 1:
                while out and out[-1] == " ":
                    out.pop()
            continue
        elif not code and char == ")" and depth:
            depth -= 1
            continue
        if depth == 0:
            out.append(char)
    return "".join(out)


def _cut(text: str, marks: str) -> int | None:
    """Where the first of ``marks`` followed by a space (or the end) stands outside code spans."""
    code = False
    for at, char in enumerate(text):
        if char == "`":
            code = not code
        elif not code and char in marks and (at + 1 == len(text) or text[at + 1] == " "):
            return at
    return None


def headline(entry: str) -> str:
    """One line for an entry: its bold lead (``**...**``), else its first sentence -- up to its
    first colon where that leaves five words or more -- capitalised, without a ``Lane X:`` prefix or
    parenthesised asides (who did it, measurements, examples)."""
    text = re.sub(r"^(Phase [\w.-]+, )?[Ll]ane [\w-]+:\s*", "", entry.strip())
    lead = re.match(r"\*\*(.+?)\*\*", text)
    text = _plain(lead.group(1) if lead else text).strip()
    if not lead:
        end = _cut(text, ".")
        text = text if end is None else text[:end]
        colon = _cut(text, ":;")
        if colon is not None and len(text[:colon].split()) >= 5:
            text = text[:colon]
    text = text.rstrip(" .:;")
    return text[:1].upper() + text[1:] + "."


def changed_entries(tag: str, folders: list[Path]) -> list[tuple[str, str]]:
    """``(package, line)`` of each fixed, added or changed entry in a package's section for
    ``tag`` and in its fragments still waiting: TOOL_PACKAGES first, then the others in
    DISTRIBUTIONS' order; in a package CHANGED_KINDS' order, an entry saying INTERNAL last."""
    order = [*TOOL_PACKAGES, *(Path(rel).name for rel in DISTRIBUTIONS)]
    out = []
    for folder in sorted(folders, key=lambda f: order.index(f.name) if f.name in order else len(order)):
        found = [(kind, entry) for kind, lines in tag_section(folder, tag).items() for entry in entries(lines)]
        found += [(f.kind, entry) for f in changelog.fragments(folder)[0]
                  for entry in entries(f.path.read_text(encoding="utf-8").splitlines())]
        found = [(kind, entry) for kind, entry in found if kind in CHANGED_KINDS]
        found.sort(key=lambda k: (INTERNAL in k[1], CHANGED_KINDS.index(k[0])))
        out += [(folder.name, headline(entry)) for _, entry in found]
    return out


def specs_at(ref: str, folder: str) -> dict[str, set[str]]:
    """Each tool's top-level arguments in its definition in ``folder`` at ``ref`` (read with
    one ``git cat-file --batch``): its ``schema`` (a Rust export) or ``input_schema`` (a
    Claude definition)."""
    listing = _git("ls-tree", "--name-only", f"{ref}:{folder}")
    names = [name for name in (listing or "").split() if name.endswith(".json")]
    if not names:
        return {}
    request = "".join(f"{ref}:{folder}/{name}\n" for name in names).encode()
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
        schema = spec.get("schema", spec.get("input_schema")) if isinstance(spec, dict) else None
        if spec.get("name") == name[:-5] and isinstance(schema, dict):
            out[name[:-5]] = set(schema.get("properties", {}))
    return out


def spec_changes(before: dict[str, set[str]], after: dict[str, set[str]]) -> list[str]:
    """The tools and arguments gone or new between two definitions' sets, as list lines; a
    count where there were no definitions before (nothing to compare with)."""
    if not before and after:
        return [f"- first snapshot of {len(after)} tools; no comparison"]
    lines = [f"- `{name}`: withdrawn" for name in sorted(set(before) - set(after))]
    for name in sorted(set(before) & set(after)):
        lines += [f"- `{name}`: argument `{arg}` removed" for arg in sorted(before[name] - after[name])]
        lines += [f"- `{name}`: argument `{arg}` added" for arg in sorted(after[name] - before[name])]
    lines += [f"- `{name}`: new tool" for name in sorted(set(after) - set(before))]
    return lines


def refused(path: Path) -> list[str]:
    """What is refused, for the team: release_refusals.READER of each ``[[not_supported]]``
    form, each said once; a form READER says ``None`` of is not listed."""
    forms = [e["form"] for e in tomllib.loads(path.read_text(encoding="utf-8")).get("not_supported", [])
             if "form" in e]
    unsaid = [form for form in forms if form not in release_refusals.READER]
    if unsaid:
        raise ValueError(f"tools/release_refusals.py does not say what these forms refuse: {unsaid}")
    said = [release_refusals.READER[form] for form in forms]
    return list(dict.fromkeys(text for text in said if text is not None))


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
    parts += ["## Changed and fixed", ""]
    changed = changed_entries(tag, changelog.packages())
    for package in dict.fromkeys(package for package, _ in changed):
        parts += [f"**{package}**", "", *(f"- {line}" for p, line in changed if p == package), ""]
    if not changed:
        parts += ["Nothing else changed.", ""]
    parts += ["## Tool definitions that changed", ""]
    if since is None:
        parts += ["(no earlier release tag to compare with)", ""]
    else:
        parts += [f"Against {since}:", ""]
        for package, folder in SPECS.items():
            diff = spec_changes(specs_at(since, folder), specs_at(head, folder))
            parts += [f"**{package}**", "", *(diff or ["- none"]), ""]
    parts += ["## Refused at this release, returning later", "",
              ("docx-agent's Word tools refuse these calls as not supported yet (code `refused`, the form in "
               "`details.not_supported`); each refusal names a call that works instead (`try_call`) or "
               "lists what is supported (`valid_options`)."), ""]
    parts += [f"- {text}." for text in refused(ROOT / KNOWN)] or ["- none"]
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
    parser.add_argument("--preview", action="store_true",
                        help="changelog fragments may still wait; the versions are those the release will set")
    args = parser.parse_args(argv)
    if TAG_PATTERN.fullmatch(args.tag) and not args.preview:
        waiting = changelog.pending(changelog.packages())
        if waiting:
            print(f"changelog fragments not yet in CHANGELOG.md (run tools/changelog.py release {args.tag} "
                  "on a release branch first):\n  " + "\n  ".join(str(p.relative_to(ROOT)) for p in waiting),
                  file=sys.stderr)
            return 1
    items = versions(release_versions(args.since) if args.preview else None)
    if args.dist is not None:
        missing = check(args.dist, items)
        if missing:
            print("missing from the release:\n  " + "\n  ".join(missing), file=sys.stderr)
            return 1
    print(changes(args.tag, args.since) + notes(args.tag, items), end="")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
