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
        pyproject.toml's version (ooxml-core: its crate's) is not older than CHANGELOG.md's
        top version section, and newer only with fragments waiting; ``__version__`` agrees;
        every fragment's name is ``<topic>.<kind>.md`` and it is not empty.  A pull request
        does not raise a version: the release sets it (below).
    python tools/changelog.py pending
        The fragments waiting for a release, one per line.
    python tools/changelog.py versions [--since REF]
        The versions ``release`` would set, ``<package> <now> -> <new>``, one per package
        with fragments waiting.
    python tools/changelog.py release TAG [--since REF]
        Cutting a release (README.md, Development): on a branch ``release/<TAG>``, sets the
        version of each package with fragments waiting and writes the fragments into its
        CHANGELOG.md under that version, then deletes them; then the release pull request,
        its merge and the tag ``TAG`` on the merge.

        The version (next_version): from the package's version at REF (default: the newest
        release tag, ``v*``, reachable from HEAD), a ``removed`` fragment raises the minor
        version while the package is 0.x (the major one from 1.0 on), any other kind the
        patch version; a version already raised past that by hand is kept.  It is written
        to pyproject.toml (ooxml-core: its crate's Cargo.toml and the Cargo.lock files),
        and ``__version__``.  The floors (set_floors): a released package's dependencies on
        the other packages here (``name>=x``) rise to those packages' versions in this
        release, the versions CI tested it with.

        Each release section is ``## <version> -- <TAG>``, the fragments by kind (KINDS'
        order, a ``### <Kind>`` heading each), then by name; a version whose section is
        already there gets them at that section's end.
    python tools/changelog.py direct-edits FILE...
        The CHANGELOG.md files among these changed paths (tools/ci.py plan refuses them on a
        pull request whose branch is not a release branch, RELEASE_BRANCH).

Standard library only, Python 3.10+.
"""

from __future__ import annotations

import argparse
import re
import subprocess
import sys
from collections.abc import Callable
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
#: The Cargo.lock files a crate's version is written to (ooxml-core's, by set_version).
LOCKS = [ROOT / "Cargo.lock", ROOT / "fuzz" / "Cargo.lock"]
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


_PROJECT = re.compile(r"^\[project\]\s*$(.*?)(?=^\[|\Z)", re.M | re.S)
_PACKAGE = re.compile(r"^\[package\]\s*$(.*?)(?=^\[|\Z)", re.M | re.S)
_VERSION_LINE = re.compile(r'^version\s*=\s*"([^"]+)"', re.M)


def _manifest(folder: Path, pyproject: str) -> Path | None:
    """A maturin package's Cargo.toml (``manifest-path``), or None."""
    found = re.search(r'^manifest-path\s*=\s*"([^"]+)"', pyproject, re.M)
    return (folder / found.group(1)).resolve() if found else None


def _table_version(text: str, table: re.Pattern[str]) -> str | None:
    body = table.search(text)
    found = body and _VERSION_LINE.search(body.group(1))
    return found.group(1) if found else None


def project_version(folder: Path) -> str | None:
    """pyproject.toml's ``[project]`` version, or, for a maturin package, its crate's."""
    return version_in(folder, lambda path: path.read_text(encoding="utf-8"))


def version_in(folder: Path, read: Callable[[Path], str]) -> str | None:
    """project_version with the files read by ``read`` (a file's text now, or at a commit)."""
    text = read(folder / "pyproject.toml")
    version = _table_version(text, _PROJECT)
    if version is not None:
        return version
    manifest = _manifest(folder, text)
    return None if manifest is None else _table_version(read(manifest), _PACKAGE)


def project_name(folder: Path) -> str:
    """pyproject.toml's ``[project]`` name (the folder's name if it has none)."""
    body = _PROJECT.search((folder / "pyproject.toml").read_text(encoding="utf-8"))
    found = body and re.search(r'^name\s*=\s*"([^"]+)"', body.group(1), re.M)
    return found.group(1) if found else folder.name


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


def next_version(base: str, kinds: set[str]) -> str:
    """The version a release of fragments of ``kinds`` gives a package at ``base``: a
    removal raises the minor version while the package is 0.x (the major one from 1.0
    on), anything else the patch version."""
    parts = [int(part) for part in base.split(".")]
    parts += [0] * (3 - len(parts))
    if "removed" in kinds:
        at = 1 if parts[0] == 0 else 0
    else:
        at = 2
    parts[at] += 1
    parts[at + 1:] = [0] * (len(parts) - at - 1)
    return ".".join(map(str, parts))


def release_versions(folders: list[Path], bases: dict[str, str] | None = None) -> dict[Path, tuple[str, str]]:
    """Each package with fragments waiting: its version now and the one its release sets.
    ``bases`` gives a package's version at the last release, by folder name (default: its
    version now); a version raised by hand past next_version is kept."""
    out = {}
    for folder in folders:
        found, _ = fragments(folder)
        now = project_version(folder)
        if not found or now is None:
            continue
        new = next_version((bases or {}).get(folder.name, now), {f.kind for f in found})
        out[folder] = (now, max(now, new, key=_key))
    return out


def _replace_version(path: Path, table: re.Pattern[str], version: str) -> None:
    text = path.read_text(encoding="utf-8")
    body = table.search(text)
    found = body and _VERSION_LINE.search(body.group(1))
    if not body or not found:
        raise ValueError(f"{path}: no version to set")
    start = body.start(1) + found.start(1)
    path.write_text(text[:start] + version + text[start + len(found.group(1)):], encoding="utf-8")


def set_version(folder: Path, version: str, locks: list[Path]) -> None:
    """Writes ``version`` to the package's pyproject.toml (a maturin package: to its crate's
    Cargo.toml and that crate's entry in each of ``locks``) and to its ``__version__``."""
    pyproject = folder / "pyproject.toml"
    text = pyproject.read_text(encoding="utf-8")
    if _table_version(text, _PROJECT) is not None:
        _replace_version(pyproject, _PROJECT, version)
    else:
        manifest = _manifest(folder, text)
        if manifest is None:
            raise ValueError(f"{folder.name}: pyproject.toml has no version")
        body = _PACKAGE.search(manifest.read_text(encoding="utf-8"))
        named = body and re.search(r'^name\s*=\s*"([^"]+)"', body.group(1), re.M)
        if not named:
            raise ValueError(f"{manifest}: no [package] name")
        crate = named.group(1)
        _replace_version(manifest, _PACKAGE, version)
        entry = re.compile(r'(^\[\[package\]\]\nname = "' + re.escape(crate) + r'"\nversion = ")[^"]+"', re.M)
        for lock in locks:
            if lock.exists():
                lock.write_text(entry.sub(lambda m: m.group(1) + version + '"', lock.read_text(encoding="utf-8")),
                                encoding="utf-8")
    for init in folder.glob("src/*/__init__.py"):
        text = init.read_text(encoding="utf-8")
        init.write_text(re.sub(r'^(__version__\s*=\s*")[^"]+"', lambda m: m.group(1) + version + '"', text,
                               flags=re.M), encoding="utf-8")


def set_floors(folder: Path, versions: dict[str, str]) -> None:
    """Raises the package's floors on the packages ``versions`` names (``name>=x`` in
    pyproject.toml, extras kept) to their versions there; never lowers one."""
    path = folder / "pyproject.toml"

    def floor(match: re.Match[str]) -> str:
        name, extras, old = match.group(1), match.group(2) or "", match.group(3)
        new = versions.get(name)
        return f'"{name}{extras}>={new if new and _key(new) > _key(old) else old}'

    names = "|".join(re.escape(name) for name in sorted(versions, key=len, reverse=True))
    if names:
        text = path.read_text(encoding="utf-8")
        path.write_text(re.sub(r'"(' + names + r')(\[[^\]]*\])?>=(' + VERSION.pattern + r')(?=[",;\s])', floor, text),
                        encoding="utf-8")


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


def release(tag: str, folders: list[Path], bases: dict[str, str] | None = None,
            locks: list[Path] | None = None) -> list[Path]:
    """Sets the version of every package with fragments (release_versions, from ``bases``)
    and its floors on the others (set_floors), writes its fragments into its CHANGELOG.md
    and deletes them; returns the fragments so written.  Nothing is written when any
    package's fragments are wrong.  ``locks``: the Cargo.lock files a crate's version is
    in (default: the repository's)."""
    for folder in folders:
        _, wrong = fragments(folder)
        if wrong:
            raise ValueError(f"{folder.name}: " + "; ".join(wrong))
    raised = release_versions(folders, bases)
    for folder, (now, new) in raised.items():
        if new != now:
            set_version(folder, new, LOCKS if locks is None else locks)
    current = {project_name(folder): project_version(folder) for folder in folders}
    for folder in raised:
        set_floors(folder, {name: v for name, v in current.items() if v and name != project_name(folder)})
    done = [path for folder in folders for path in assemble(folder, tag)]
    for path in done:
        path.unlink()
    return done


def _git(*args: str) -> str | None:
    run = subprocess.run(["git", *args], check=False, cwd=ROOT, capture_output=True, text=True)
    return run.stdout if run.returncode == 0 else None


def last_release() -> str | None:
    """The newest release tag reachable from HEAD, or None."""
    found = _git("describe", "--tags", "--abbrev=0", "--match", "v[0-9]*")
    return found.strip() if found else None


def bases_at(ref: str, folders: list[Path]) -> dict[str, str]:
    """Each package's version at ``ref`` (one that did not exist there is left out)."""
    def read(path: Path) -> str:
        text = _git("show", f"{ref}:{path.resolve().relative_to(ROOT).as_posix()}")
        if text is None:
            raise FileNotFoundError(path)
        return text

    out = {}
    for folder in folders:
        try:
            version = version_in(folder, read)
        except FileNotFoundError:
            continue
        if version is not None:
            out[folder.name] = version
    return out


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
    v = sub.add_parser("versions")
    r = sub.add_parser("release")
    r.add_argument("tag")
    for s in (v, r):
        s.add_argument("--since", help="the last release's tag or commit (default: the newest v* tag before HEAD)")
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
    if args.command in ("versions", "release"):
        folders = packages()
        since = args.since or last_release()
        bases = bases_at(since, folders) if since else {}
        if since is None:
            print("changelog: no release tag before HEAD; versions rise from today's", file=sys.stderr)
        for folder, (now, new) in release_versions(folders, bases).items():
            print(f"{folder.name} {now} -> {new}" + (f" (at {since}: {bases[folder.name]})"
                                                     if folder.name in bases and bases[folder.name] != now else ""))
        if args.command == "versions":
            return 0
        try:
            done = release(args.tag, folders, bases)
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
