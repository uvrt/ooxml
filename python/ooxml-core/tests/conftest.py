"""The differential tests' documents and tools.

The documents are made up here, by the generators in harness/ (seeded), and laid out by
docx2svg (harness/export.py): its results are the reference the Rust kernels and the owner
must equal.  Exports are cached per session in a work folder (OOXML_CORE_WORK, or pytest's
temporary directory).

OOXML_CORE_SECTIONS (default 6) sets the report's size: 6 sections is 5 pages, 62 about 50
pages.  The `word-check` binary is OOXML_WORD_CHECK, else the workspace's
target/release/word-check, else target/debug/word-check.
"""
from __future__ import annotations

import os
import sys
from pathlib import Path

import pytest

HERE = Path(__file__).resolve().parent
HARNESS = HERE / "harness"
ROOT = HERE.parents[2]
sys.path.insert(0, str(HARNESS))


def _need(module: str):
    return pytest.importorskip(module, reason=f"{module} is not installed (the differential tests need it)")


@pytest.fixture(scope="session")
def work(tmp_path_factory) -> Path:
    path = os.environ.get("OOXML_CORE_WORK")
    if path:
        Path(path).mkdir(parents=True, exist_ok=True)
        return Path(path)
    return tmp_path_factory.mktemp("ooxml-core")


@pytest.fixture(scope="session")
def word_check() -> Path:
    for candidate in (os.environ.get("OOXML_WORD_CHECK"), ROOT / "target/release/word-check",
                      ROOT / "target/debug/word-check"):
        if candidate and Path(candidate).exists():
            return Path(candidate)
    pytest.skip("no word-check binary: cargo build --release -p ooxml-word")


def export_json(data: bytes, out: Path, edit: str | None = None, lines_only: bool = False) -> Path:
    """docx2svg's layout of ``data`` (after ``edit``, KIND@POS) as harness/export.py writes it;
    ``lines_only``: line breaking only, every paragraph the exporter can take."""
    _need("docx2svg")
    _need("docx_agent")
    import docx2svg
    import edits
    import export
    from docx_agent import Document
    from docx_agent.layout import layout_options

    if edit:
        kind, position = edit.split("@")
        document = Document.open(data)
        edits.edit(document, kind, position)
        data = document.to_bytes()
    options = {} if lines_only else layout_options(Document.open(data), {})
    obj = export.export(data, docx2svg.ConvertOptions(**options), lines_only=lines_only)
    out.write_text(export.finish(obj))
    return out


@pytest.fixture(scope="session")
def report(work) -> Path:
    """The made-up report (contents, cross references, page X of Y)."""
    _need("docx_agent")
    import subprocess

    sections = os.environ.get("OOXML_CORE_SECTIONS", "6")
    path = work / f"report-{sections}.docx"
    if not path.exists():
        subprocess.run([sys.executable, str(HARNESS / "make_report.py"), str(path), "1", sections], check=True,
                       stdout=subprocess.DEVNULL)
    return path


@pytest.fixture(scope="session")
def report_export(work, report) -> Path:
    out = work / (report.stem + ".json")
    return out if out.exists() else export_json(report.read_bytes(), out)


@pytest.fixture(scope="session")
def edited_exports(work, report) -> dict[str, Path]:
    """docx2svg's layout of each of edits.py's 15 edits of the report."""
    import edits

    folder = work / (report.stem + "-edits")
    folder.mkdir(exist_ok=True)
    out = {}
    for kind in edits.KINDS:
        for position in edits.POSITIONS:
            name = f"{kind}@{position}"
            path = folder / f"{name}.json"
            out[name] = path if path.exists() else export_json(report.read_bytes(), path, edit=name)
    return out
