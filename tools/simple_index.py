"""A PEP 503 simple index of this repository's GitHub Releases, for GitHub Pages.

    python tools/simple_index.py dist DIST_DIR OUT_DIR --base-url URL
    python tools/simple_index.py releases OUT_DIR --repo OWNER/REPO [--local DIR] [--previous URL]

writes OUT_DIR/simple/index.html (one link per distribution) and
OUT_DIR/simple/<normalized-name>/index.html (one link per file, with `#sha256=` and
`data-requires-python`), plus OUT_DIR/files.json, the same entries as data.  The links
point at the files where they are (the GitHub Release assets); the index holds no copies.

`dist`: the files of one build, linked as BASE_URL + file name (the release workflow's check
on pull requests and manual runs, and tools/check_release_install.sh).

`releases`: every published release of REPO, from the GitHub API (GH_TOKEN or GITHUB_TOKEN
if set), newest first.  Every release is listed, not only the newest, so pip's resolver
sees every version and a user can pin an older one; pip picks the newest version itself.
A file name that more than one release carries (a package whose version did not change
between releases) is listed once, with the newest release's file.  The sha256 is the
asset's digest from the API.  Requires-Python comes from the file's metadata: from a file
in --local DIR with the same name and hash (the release just built), else from --previous
(the files.json of the index deployed before, keyed by hash), else from the file itself,
downloaded once.

Standard library only.
"""

from __future__ import annotations

import argparse
import email.parser
import hashlib
import html
import json
import os
import re
import sys
import tarfile
import tempfile
import urllib.error
import urllib.request
import zipfile
from dataclasses import asdict, dataclass
from pathlib import Path
from typing import Callable, Iterable

API = "https://api.github.com"

_WHEEL = re.compile(r"^(?P<name>[^-]+)-(?P<version>[^-]+)(-[^-]+)?-[^-]+-[^-]+-[^-]+\.whl$")
_SDIST = re.compile(r"^(?P<name>.+)-(?P<version>[^-]+)\.tar\.gz$")


@dataclass(frozen=True)
class File:
    """One file of the index: where it is, its hash and its Requires-Python ('' if none)."""

    filename: str
    url: str
    sha256: str
    requires_python: str


def normalize(name: str) -> str:
    """PEP 503: runs of '-', '_' and '.' become one '-', lowercase."""
    return re.sub(r"[-_.]+", "-", name).lower()


def split_filename(filename: str) -> tuple[str, str] | None:
    """(normalized project name, version) of a wheel or sdist file name, else None."""
    m = _WHEEL.match(filename) or _SDIST.match(filename)
    return (normalize(m["name"]), m["version"]) if m else None


def requires_python(path: Path) -> str:
    """Requires-Python of a wheel's METADATA or an sdist's PKG-INFO ('' if it has none)."""
    if path.name.endswith(".whl"):
        with zipfile.ZipFile(path) as z:
            names = [n for n in z.namelist() if re.fullmatch(r"[^/]+\.dist-info/METADATA", n)]
            if len(names) != 1:
                raise ValueError(f"{path.name}: no single .dist-info/METADATA")
            text = z.read(names[0]).decode("utf-8")
    else:
        with tarfile.open(path, "r:gz") as t:
            members = [m for m in t.getmembers() if re.fullmatch(r"[^/]+/PKG-INFO", m.name)]
            if len(members) != 1:
                raise ValueError(f"{path.name}: no single top-level PKG-INFO")
            fh = t.extractfile(members[0])
            assert fh is not None
            text = fh.read().decode("utf-8")
    value = email.parser.HeaderParser().parsestr(text).get("Requires-Python")
    return (value or "").strip()


def sha256_of(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def files_from_dir(dist: Path, base_url: str) -> list[File]:
    """Every wheel and sdist in DIST, linked as BASE_URL + its name."""
    out = []
    for path in sorted(dist.iterdir()):
        if split_filename(path.name) is None:
            continue
        out.append(File(path.name, base_url + path.name, sha256_of(path), requires_python(path)))
    return out


# -- GitHub Releases ---------------------------------------------------------------------

Fetch = Callable[[str], bytes]


def _http_fetch(url: str) -> bytes:
    headers = {"User-Agent": "simple-index"}
    token = os.environ.get("GH_TOKEN") or os.environ.get("GITHUB_TOKEN")
    if token and url.startswith(API):
        headers["Authorization"] = f"Bearer {token}"
        headers["Accept"] = "application/vnd.github+json"
    with urllib.request.urlopen(urllib.request.Request(url, headers=headers), timeout=120) as r:
        return r.read()


def list_releases(repo: str, fetch: Fetch) -> list[dict]:
    """Published releases (no drafts or pre-releases), newest first."""
    releases: list[dict] = []
    page = 1
    while True:
        batch = json.loads(fetch(f"{API}/repos/{repo}/releases?per_page=100&page={page}"))
        releases += batch
        if len(batch) < 100:
            break
        page += 1
    releases = [r for r in releases if not r.get("draft") and not r.get("prerelease")]
    releases.sort(key=lambda r: r.get("published_at") or "", reverse=True)
    return releases


def files_from_releases(
    releases: list[dict],
    *,
    fetch: Fetch,
    local: Path | None = None,
    previous: dict[str, str] | None = None,
) -> list[File]:
    """The files of RELEASES (newest first); a file name repeated in an older release is
    left out.  PREVIOUS maps sha256 to Requires-Python (a deployed index's files.json)."""
    previous = previous or {}
    local_files = {p.name: p for p in local.iterdir()} if local else {}
    seen: set[str] = set()
    out: list[File] = []
    with tempfile.TemporaryDirectory() as tmp:
        for release in releases:
            for asset in release.get("assets", []):
                name = asset["name"]
                if name in seen or split_filename(name) is None:
                    continue
                seen.add(name)
                url = asset["browser_download_url"]
                digest = asset.get("digest") or ""
                sha = digest.removeprefix("sha256:") if digest.startswith("sha256:") else ""
                path = local_files.get(name)
                if path is not None and sha and sha256_of(path) != sha:
                    path = None  # same name, other bytes: not this file
                if sha and path is None and sha in previous:
                    out.append(File(name, url, sha, previous[sha]))
                    continue
                if path is None:  # no digest, or metadata unknown: read the file itself
                    path = Path(tmp) / name
                    path.write_bytes(fetch(url))
                got = sha256_of(path)
                if sha and got != sha:
                    raise ValueError(f"{name}: sha256 {got} is not the asset's digest {sha}")
                out.append(File(name, url, got, requires_python(path)))
                if path.parent == Path(tmp):
                    path.unlink()
    return out


def load_previous(url: str, fetch: Fetch) -> dict[str, str]:
    """sha256 -> Requires-Python from a deployed files.json; empty if there is none yet."""
    try:
        data = json.loads(fetch(url))
    except (urllib.error.URLError, OSError, ValueError) as e:
        print(f"no previous index at {url} ({e}); reading every file's metadata", file=sys.stderr)
        return {}
    return {f["sha256"]: f["requires_python"] for f in data.get("files", [])}


# -- Writing ------------------------------------------------------------------------------

_HEAD = '<!DOCTYPE html>\n<html>\n<head>\n<meta name="pypi:repository-version" content="1.0">\n'


def _project_page(project: str, files: Iterable[File]) -> str:
    rows = []
    for f in sorted(files, key=lambda f: f.filename):
        href = html.escape(f"{f.url}#sha256={f.sha256}", quote=True)
        rp = f' data-requires-python="{html.escape(f.requires_python, quote=True)}"' if f.requires_python else ""
        rows.append(f'<a href="{href}"{rp}>{html.escape(f.filename)}</a><br>')
    title = f"Links for {html.escape(project)}"
    return f"{_HEAD}<title>{title}</title>\n</head>\n<body>\n<h1>{title}</h1>\n" + "\n".join(rows) + "\n</body>\n</html>\n"


def write_index(files: list[File], out: Path) -> dict[str, list[File]]:
    """OUT/simple/index.html, OUT/simple/<project>/index.html and OUT/files.json."""
    projects: dict[str, list[File]] = {}
    for f in files:
        parts = split_filename(f.filename)
        if parts is None:
            raise ValueError(f"not a wheel or sdist: {f.filename}")
        projects.setdefault(parts[0], []).append(f)
    simple = out / "simple"
    simple.mkdir(parents=True, exist_ok=True)
    links = "\n".join(f'<a href="{p}/">{p}</a><br>' for p in sorted(projects))
    (simple / "index.html").write_text(
        f"{_HEAD}<title>Simple index</title>\n</head>\n<body>\n{links}\n</body>\n</html>\n", encoding="utf-8"
    )
    for project, pfiles in projects.items():
        (simple / project).mkdir(exist_ok=True)
        (simple / project / "index.html").write_text(_project_page(project, pfiles), encoding="utf-8")
    data = {"files": [asdict(f) for f in sorted(files, key=lambda f: f.filename)]}
    (out / "files.json").write_text(json.dumps(data, indent=1) + "\n", encoding="utf-8")
    return projects


def main(argv: list[str]) -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    d = sub.add_parser("dist", help="the files of one build")
    d.add_argument("dist_dir", type=Path)
    d.add_argument("out_dir", type=Path)
    d.add_argument("--base-url", required=True)
    r = sub.add_parser("releases", help="every published release of a repository")
    r.add_argument("out_dir", type=Path)
    r.add_argument("--repo", required=True)
    r.add_argument("--local", type=Path)
    r.add_argument("--previous")
    args = ap.parse_args(argv)
    if args.cmd == "dist":
        files = files_from_dir(args.dist_dir, args.base_url)
    else:
        previous = load_previous(args.previous, _http_fetch) if args.previous else {}
        releases = list_releases(args.repo, _http_fetch)
        files = files_from_releases(releases, fetch=_http_fetch, local=args.local, previous=previous)
    projects = write_index(files, args.out_dir)
    for project in sorted(projects):
        print(f"{project}: {len(projects[project])} files")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
