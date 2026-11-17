"""Tests of tools/simple_index.py on made-up wheels and sdists.  Standard library only; ci.yml's
plan job runs them (`python3 -m unittest discover -s tools -p 'test_*.py'`)."""

from __future__ import annotations

import hashlib
import io
import json
import re
import sys
import tarfile
import tempfile
import unittest
import zipfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import simple_index as si  # noqa: E402

BASE = "https://example.invalid/releases/download"


def _metadata(name: str, version: str, requires: str | None) -> str:
    text = f"Metadata-Version: 2.4\nName: {name}\nVersion: {version}\n"
    if requires is not None:
        text += f"Requires-Python: {requires}\n"
    return text + "\nA description that says Requires-Python: >=9 in its body.\n"


def make_wheel(folder: Path, name: str, version: str, requires: str | None = ">=3.10", tag: str = "py3-none-any") -> Path:
    stem = f"{re.sub(r'[-_.]+', '_', name).lower()}-{version}"
    path = folder / f"{stem}-{tag}.whl"
    with zipfile.ZipFile(path, "w") as z:
        z.writestr(f"{stem.split('-')[0]}/__init__.py", f"# {path.name}\n")
        z.writestr(f"{stem}.dist-info/METADATA", _metadata(name, version, requires))
    return path


def make_sdist(folder: Path, name: str, version: str, requires: str | None = ">=3.10") -> Path:
    stem = f"{re.sub(r'[-_.]+', '_', name).lower()}-{version}"
    path = folder / f"{stem}.tar.gz"
    data = _metadata(name, version, requires).encode()
    with tarfile.open(path, "w:gz") as t:
        info = tarfile.TarInfo(f"{stem}/PKG-INFO")
        info.size = len(data)
        t.addfile(info, io.BytesIO(data))
    return path


def sha(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


class Folder(unittest.TestCase):
    def setUp(self) -> None:
        self._tmp = tempfile.TemporaryDirectory()
        self.tmp = Path(self._tmp.name)

    def tearDown(self) -> None:
        self._tmp.cleanup()

    def sub(self, name: str) -> Path:
        p = self.tmp / name
        p.mkdir()
        return p


class NamesTest(unittest.TestCase):
    def test_normalize(self) -> None:
        for raw in ("Docx_Agent", "docx-agent", "docx.agent", "DOCX--_.agent"):
            self.assertEqual(si.normalize(raw), "docx-agent")

    def test_split_filename(self) -> None:
        self.assertEqual(si.split_filename("docx_agent-0.1.123-py3-none-any.whl"), ("docx-agent", "0.1.123"))
        self.assertEqual(si.split_filename("docx_agent-0.1.123.tar.gz"), ("docx-agent", "0.1.123"))
        self.assertEqual(
            si.split_filename("core_x-0.8.0-cp310-abi3-manylinux2014_aarch64.manylinux_2_17_aarch64.whl"),
            ("core-x", "0.8.0"),
        )
        self.assertEqual(si.split_filename("pkg-1.0-1build-py3-none-any.whl"), ("pkg", "1.0"))
        self.assertIsNone(si.split_filename("notes.md"))
        self.assertIsNone(si.split_filename("pkg-1.0.zip"))


class DistTest(Folder):
    def test_pages_hashes_and_requires_python(self) -> None:
        dist = self.sub("dist")
        w = make_wheel(dist, "Some.Pkg", "1.2.0", requires=">=3.10,<4")
        s = make_sdist(dist, "Some.Pkg", "1.2.0", requires=">=3.10,<4")
        n = make_wheel(dist, "other", "0.1", requires=None)
        (dist / "notes.md").write_text("not a package\n")
        out = self.sub("site")
        projects = si.write_index(si.files_from_dir(dist, "/dist/"), out)
        self.assertEqual(sorted(projects), ["other", "some-pkg"])

        root = (out / "simple" / "index.html").read_text()
        self.assertIn('<a href="other/">other</a>', root)
        self.assertIn('<a href="some-pkg/">some-pkg</a>', root)
        self.assertIn('<meta name="pypi:repository-version" content="1.0">', root)

        page = (out / "simple" / "some-pkg" / "index.html").read_text()
        # HTML-escaped specifier, one link per file, hash fragment of the file's bytes.
        self.assertIn(f'<a href="/dist/{w.name}#sha256={sha(w)}" data-requires-python="&gt;=3.10,&lt;4">{w.name}</a>', page)
        self.assertIn(f'<a href="/dist/{s.name}#sha256={sha(s)}" data-requires-python="&gt;=3.10,&lt;4">{s.name}</a>', page)
        other = (out / "simple" / "other" / "index.html").read_text()
        self.assertIn(f'<a href="/dist/{n.name}#sha256={sha(n)}">{n.name}</a>', other)
        self.assertNotIn("data-requires-python", other)

        data = json.loads((out / "files.json").read_text())
        self.assertEqual(len(data["files"]), 3)

    def test_escaping_of_urls_and_names(self) -> None:
        dist = self.sub("dist")
        w = make_wheel(dist, "pkg", "1.0", requires='>=3.10"<x>&')
        out = self.sub("site")
        si.write_index(si.files_from_dir(dist, "https://h.invalid/a?b=1&c=2/"), out)
        page = (out / "simple" / "pkg" / "index.html").read_text()
        self.assertIn('data-requires-python="&gt;=3.10&quot;&lt;x&gt;&amp;"', page)
        self.assertIn(f'href="https://h.invalid/a?b=1&amp;c=2/{w.name}#sha256=', page)


class ReleasesTest(Folder):
    """Three made-up releases, newest first, served by a fake fetch."""

    def setUp(self) -> None:
        super().setUp()
        self.blobs: dict[str, bytes] = {}
        self.fetched: list[str] = []
        r1, r2, r3 = self.sub("r1"), self.sub("r2"), self.sub("r3")
        # r1: a 1.0, b 1.0.  r2: a 1.1 (needs a newer Python), b 1.0 rebuilt (other bytes).
        # r3: a 1.1 rebuilt again, c 0.1.  A draft and a pre-release carry a 9.0.
        self.r1 = [make_wheel(r1, "a", "1.0", ">=3.9"), make_sdist(r1, "a", "1.0", ">=3.9"), make_wheel(r1, "b", "1.0")]
        self.r2 = [make_wheel(r2, "a", "1.1", ">=3.11"), make_sdist(r2, "a", "1.1", ">=3.11"), make_wheel(r2, "b", "1.0")]
        (r2 / "notes.md").write_text("x")
        self.r3 = [make_wheel(r3, "a", "1.1", ">=3.11", tag="cp311-abi3-linux_x86_64"), make_wheel(r3, "c", "0.1")]
        self.local = r3
        draft = self.sub("draft")
        self.draft = [make_wheel(draft, "a", "9.0")]
        self.api = [
            self._release("v3", "2026-03-01T00:00:00Z", self.r3),
            self._release("v1", "2026-01-01T00:00:00Z", self.r1),
            self._release("v2", "2026-02-01T00:00:00Z", self.r2 + [r2 / "notes.md"]),
            self._release("v9", "2026-04-01T00:00:00Z", self.draft, draft=True),
            self._release("v9rc", "2026-04-02T00:00:00Z", self.draft, prerelease=True),
        ]

    def _release(self, tag: str, when: str, paths: list[Path], **flags: bool) -> dict:
        assets = []
        for p in paths:
            url = f"{BASE}/{tag}/{p.name}"
            self.blobs[url] = p.read_bytes()
            assets.append({"name": p.name, "browser_download_url": url, "digest": f"sha256:{sha(p)}"})
        return {"tag_name": tag, "published_at": when, "assets": assets, **flags}

    def fetch(self, url: str) -> bytes:
        self.fetched.append(url)
        if url.startswith(si.API):
            page = int(url.rsplit("page=", 1)[1])
            return json.dumps(self.api if page == 1 else []).encode()
        return self.blobs[url]

    def build(self, **kw: object) -> tuple[list[si.File], dict[str, str]]:
        files = si.files_from_releases(si.list_releases("o/r", self.fetch), fetch=self.fetch, **kw)
        return files, {f.filename: f.url for f in files}

    def test_every_release_newest_file_per_name(self) -> None:
        files, urls = self.build()
        self.assertEqual(
            sorted(urls),
            sorted({"a-1.0-py3-none-any.whl", "a-1.0.tar.gz", "b-1.0-py3-none-any.whl", "a-1.1-py3-none-any.whl",
                    "a-1.1.tar.gz", "a-1.1-cp311-abi3-linux_x86_64.whl", "c-0.1-py3-none-any.whl"}),
        )
        self.assertEqual(urls["a-1.0.tar.gz"], f"{BASE}/v1/a-1.0.tar.gz")
        self.assertEqual(urls["b-1.0-py3-none-any.whl"], f"{BASE}/v2/b-1.0-py3-none-any.whl")  # the newer of two
        by = {f.filename: f for f in files}
        self.assertEqual(by["b-1.0-py3-none-any.whl"].sha256, sha(self.r2[2]))
        self.assertEqual(by["a-1.0-py3-none-any.whl"].requires_python, ">=3.9")
        self.assertEqual(by["a-1.1.tar.gz"].requires_python, ">=3.11")
        self.assertNotIn("a-9.0-py3-none-any.whl", urls)  # drafts and pre-releases left out

        out = self.sub("site")
        si.write_index(files, out)
        page = (out / "simple" / "a" / "index.html").read_text()
        self.assertEqual(page.count("<a href="), 5)
        self.assertIn('data-requires-python="&gt;=3.9"', page)
        self.assertIn('data-requires-python="&gt;=3.11"', page)

    def test_local_and_previous_save_downloads(self) -> None:
        files, _ = self.build()
        downloads = [u for u in self.fetched if not u.startswith(si.API)]
        self.assertEqual(len(downloads), len(files))  # nothing known: every file read once

        previous = {f.sha256: f.requires_python for f in files if "/v3/" not in f.url}
        self.fetched.clear()
        again, _ = self.build(local=self.local, previous=previous)
        self.assertEqual([u for u in self.fetched if not u.startswith(si.API)], [])
        self.assertEqual(again, files)

    def test_previous_from_files_json(self) -> None:
        files, _ = self.build()
        out = self.sub("site")
        si.write_index(files, out)
        blob = (out / "files.json").read_bytes()
        previous = si.load_previous("https://pages.invalid/files.json", lambda url: blob)
        self.assertEqual(previous, {f.sha256: f.requires_python for f in files})

        def missing(url: str) -> bytes:
            raise si.urllib.error.HTTPError(url, 404, "Not Found", None, None)  # type: ignore[arg-type]

        self.assertEqual(si.load_previous("https://pages.invalid/files.json", missing), {})

    def test_local_file_with_other_bytes_is_not_used(self) -> None:
        make_wheel(self.local, "c", "0.1", requires=">=3.99")  # same name, other metadata
        files, _ = self.build(local=self.local)
        self.assertEqual({f.filename: f.requires_python for f in files}["c-0.1-py3-none-any.whl"], ">=3.10")

    def test_digest_mismatch_fails(self) -> None:
        self.api[0]["assets"][1]["digest"] = "sha256:" + "0" * 64
        with self.assertRaises(ValueError):
            self.build()

    def test_pages_of_results(self) -> None:
        many = [{"tag_name": f"t{i}", "published_at": f"2026-01-01T00:00:{i % 60:02d}Z", "assets": []} for i in range(150)]

        def fetch(url: str) -> bytes:
            page = int(url.rsplit("page=", 1)[1])
            return json.dumps(many[(page - 1) * 100 : page * 100]).encode()

        self.assertEqual(len(si.list_releases("o/r", fetch)), 150)


if __name__ == "__main__":
    unittest.main()
