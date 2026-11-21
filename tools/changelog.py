"""Changelog fragments: each change notes itself in a file of its own; CHANGELOG.md is written
at release time only.

A pull request does not edit a package's CHANGELOG.md (two pull requests that both add to
its top section always conflict).  It adds a fragment instead:

    python/<package>/changes/<topic>.<kind>.md

``topic`` is the branch or the subject (lower case, digits and ``-``: two pull requests never
pick the same one), ``kind`` one of KINDS.  The file holds the entry as CHANGELOG.md would,
Markdown paragraphs or a list.  A pull request may hold several fragments (``<topic>-2...``).

    python tools/changelog.py check [PACKAGE]...
        The build job's version check (tools/ci.py build-check), every package by default:
        pyproject.toml's version (ooxml-core: its crate's) is the version CHANGELOG.md's top
        version section names, or newer with fragments waiting to name it; ``__version__``
        agrees; every fragment's name is ``<topic>.<kind>.md`` and it is not empty.
    python tools/changelog.py pending
        The fragments waiting for a release, one per line.
    python tools/changelog.py release TAG
        Cutting a release (README.md, Development): on a branch ``release/<TAG>``, writes each
        package's fragments into its CHANGELOG.md under the package's version and deletes
        them; then the release pull request, its merge and the tag ``TAG`` on the merge.  A
        version already the top section's gets the fragments at the top of that section; a
        newer one a new section, ``## <version> -- <TAG>``.  Within it the fragments go by
        kind (KINDS' order, a ``### <Kind>`` heading each), then by name.
    python tools/changelog.py direct-edits FILE...
        The CHANGELOG.md files among these changed paths (tools/ci.py plan refuses them on a
        pull request whose branch is not a release branch, RELEASE_BRANCH).

Standard library only, Python 3.10+.
"""

from __future__ import annotations

import argparse
import re
import sys
from dataclasses import dataclass
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
PY = ROOT / "python"

#: A fragment's kinds, in the order a release section lists them.
KINDS = ("added", "changed", "fixed", "removed")
FRAGMENT = re.compile(r"^[a-z0-9][a-z0-9-]*\.(?P<kind>[a-z]+)\.md$")
#: The folder of a package's fragments.
CHANGES = "changes"
#: A pull request from a branch with this prefix may write CHANGELOG.md (it is a release).
RELEASE_BRANCH = "release/"
VERSION = re.compile(r"\d+(?:\.\d+)+")


@dataclass(frozen=True)
class Fragment:
    path: Path
    kind: str


def packages(py: Path = PY) -> list[Path]:
    """The folders of the packages that keep a CHANGELOG.md."""
    return sorted(p.parent for p in py.glob("*/CHANGELOG.md"))


def _key(version: str) -> tuple[int, ...]:
    return tuple(int(part) for part in version.split("."))


def project_version(folder: Path) -> str | None:
    """pyproject.toml's ``[project]`` version, or, for a maturin package, its crate's."""
    text = (folder / "pyproject.toml").read_text(encoding="utf-8")
    table = re.search(r"^\[project\]\s*$(.*?)(?=^\[|\Z)", text, re.M | re.S)
    found = table and re.search(r'^version\s*=\s*"([^"]+)"', table.group(1), re.M)
    if found:
        return found.group(1)
    manifest = re.search(r'^manifest-path\s*=\s*"([^"]+)"', text, re.M)
    if manifest is None:
        return None
    cargo = (folder / manifest.group(1)).read_text(encoding="utf-8")
    table = re.search(r"^\[package\]\s*$(.*?)(?=^\[|\Z)", cargo, re.M | re.S)
    found = table and re.search(r'^version\s*=\s*"([^"]+)"', table.group(1), re.M)
    return found.group(1) if found else None


def top_section(text: str) -> tuple[str, int] | None:
    """The version the first ``## <version> ...`` heading names, and its line's index (an
    ``Unreleased`` heading above it is passed over)."""
    for index, line in enumerate(text.splitlines()):
        if line.startswith("## "):
            found = VERSION.match(line[3:].strip())
            if found:
                return found.group(0), index
    return None


def package_versions(folder: Path) -> dict[str, str]:
    """``__version__`` of each module under src/, by its file."""
    versions = {}
    for init in sorted(folder.glob("src/*/__init__.py")):
        found = re.search(r'^__version__\s*=\s*"([^"]+)"', init.read_text(encoding="utf-8"), re.M)
        if found:
            versions[str(init.relative_to(folder))] = found.group(1)
    return versions


def fragments(folder: Path) -> tuple[list[Fragment], list[str]]:
    """The package's fragments, sorted by kind (KINDS) and name, and what is wrong with any."""
    found, wrong = [], []
    for path in sorted((folder / CHANGES).glob("*")):
        match = FRAGMENT.match(path.name)
        if not path.is_file() or match is None or match["kind"] not in KINDS:
            wrong.append(f"{CHANGES}/{path.name}: name it <topic>.<kind>.md, kind one of {', '.join(KINDS)}")
        elif not path.read_text(encoding="utf-8").strip():
            wrong.append(f"{CHANGES}/{path.name} is empty")
        else:
            found.append(Fragment(path, match["kind"]))
    found.sort(key=lambda f: (KINDS.index(f.kind), f.path.name))
    return found, wrong


def problems(folder: Path) -> list[str]:
    """What the build job's check finds wrong with the package in ``folder``."""
    version = project_version(folder)
    if version is None:
        return ["pyproject.toml has no [project] version (nor a maturin manifest-path)"]
    pending, wrong = fragments(folder)
    top = top_section((folder / "CHANGELOG.md").read_text(encoding="utf-8"))
    if top is None:
        if not pending:
            wrong.append(f"CHANGELOG.md has no version section and {CHANGES}/ no fragment for {version}")
    elif _key(version) < _key(top[0]):
        wrong.append(f"pyproject.toml says {version}, older than CHANGELOG.md's top section {top[0]}")
    elif version != top[0] and not pending:
        wrong.append(f"pyproject.toml raises the version to {version} (CHANGELOG.md's top section is {top[0]}): "
                     f"add a fragment, {CHANGES}/<topic>.<kind>.md, saying what changed")
    for path, value in package_versions(folder).items():
        if value != version:
            wrong.append(f"{path} sets __version__ = {value!r} but pyproject.toml says {version}")
    return wrong


def section(found: list[Fragment]) -> list[str]:
    """The lines the fragments make in a release section: ``### <Kind>`` and each entry."""
    lines: list[str] = []
    for kind in KINDS:
        entries = [f.path.read_text(encoding="utf-8").strip() for f in found if f.kind == kind]
        if entries:
            lines += ["", f"### {kind.capitalize()}"]
            for entry in entries:
                lines += ["", *entry.splitlines()]
    return lines


def assemble(folder: Path, tag: str) -> list[Path]:
    """Writes the package's fragments into its CHANGELOG.md for release ``tag``; returns them
    (the caller deletes them once every package is written)."""
    found, wrong = fragments(folder)
    if wrong:
        raise ValueError(f"{folder.name}: " + "; ".join(wrong))
    if not found:
        return []
    version = project_version(folder)
    if version is None:
        raise ValueError(f"{folder.name}: pyproject.toml has no version")
    path = folder / "CHANGELOG.md"
    lines = path.read_text(encoding="utf-8").splitlines()
    top = top_section("\n".join(lines))
    if top is not None and top[0] == version:
        # The version's section is there: the fragments go at its end, after its own text.
        at = next((i for i in range(top[1] + 1, len(lines)) if lines[i].startswith("## ")), len(lines))
        while at > top[1] + 1 and not lines[at - 1].strip():
            at -= 1
        new = section(found)
    else:
        if top is not None and _key(version) < _key(top[0]):
            raise ValueError(f"{folder.name}: version {version} is older than CHANGELOG.md's {top[0]}")
        # A new section, above every other.
        at = next((i for i, line in enumerate(lines) if line.startswith("## ")), len(lines))
        while at > 0 and not lines[at - 1].strip():
            at -= 1
        new = ["", f"## {version} -- {tag}", *section(found)]
    if at < len(lines) and lines[at].strip():
        new.append("")
    lines[at:at] = new
    path.write_text("\n".join(lines) + "\n", encoding="utf-8")
    return [f.path for f in found]


def release(tag: str, folders: list[Path]) -> list[Path]:
    """Every package's fragments into its CHANGELOG.md, then deleted; the fragments, so
    written.  Nothing is written when any package's fragments are wrong."""
    for folder in folders:
        _, wrong = fragments(folder)
        if wrong:
            raise ValueError(f"{folder.name}: " + "; ".join(wrong))
    done = [path for folder in folders for path in assemble(folder, tag)]
    for path in done:
        path.unlink()
    return done


def pending(folders: list[Path]) -> list[Path]:
    """The fragments waiting for a release."""
    return [f.path for folder in folders for f in fragments(folder)[0]]


def direct_edits(changed: list[str]) -> list[str]:
    """The packages' CHANGELOG.md files among ``changed`` (paths from the root)."""
    return [p for p in changed if re.fullmatch(r"python/[^/]+/CHANGELOG\.md", p)]


def is_release_branch(branch: str | None) -> bool:
    return bool(branch) and branch.startswith(RELEASE_BRANCH)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    sub = parser.add_subparsers(dest="command", required=True)
    c = sub.add_parser("check")
    c.add_argument("packages", nargs="*")
    sub.add_parser("pending")
    r = sub.add_parser("release")
    r.add_argument("tag")
    d = sub.add_parser("direct-edits")
    d.add_argument("files", nargs="+")
    args = parser.parse_args(argv)

    if args.command == "check":
        failed = False
        for folder in [PY / name for name in args.packages] or packages():
            for problem in problems(folder):
                print(f"changelog: {folder.name}: {problem}", file=sys.stderr)
                failed = True
        if not failed:
            print("changelog: versions, CHANGELOG.md, fragments and __version__ agree")
        return 1 if failed else 0
    if args.command == "pending":
        for path in pending(packages()):
            print(path.relative_to(ROOT))
        return 0
    if args.command == "release":
        try:
            done = release(args.tag, packages())
        except ValueError as error:
            print(f"changelog: {error}", file=sys.stderr)
            return 1
        print(f"changelog: {len(done)} fragments written for {args.tag}")
        return 0
    if args.command == "direct-edits":
        print("\n".join(direct_edits(args.files)))
        return 0
    return 2


if __name__ == "__main__":
    sys.exit(main())
