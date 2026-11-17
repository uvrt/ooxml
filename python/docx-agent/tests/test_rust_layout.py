"""Lane L4: the layout in Rust (:mod:`docx_agent.pages`) answers as docx2svg's Python layout
does -- pages, signatures, warnings, coverage, layout facts and every page's SVG -- for a
made-up report and each kind of edit, each version laid out from the one before in Rust and
compared with the version laid out alone in Python; nothing of it is pickled, and a document
it does not lay out yet falls back, counted by its reason."""

from __future__ import annotations

import pickle

import pytest

from docx_agent import Document, pages
from docx_agent.layout import convert

from test_incremental_layout import _answers, _edits, report

pytestmark = [pytest.mark.any_faces,
              pytest.mark.skipif(not pages.available(), reason="ooxml_core without the Rust layout")]


@pytest.fixture(scope="module")
def made() -> bytes:
    return report()


def _python(data: bytes, monkeypatch) -> dict:
    with monkeypatch.context() as patched:
        patched.setattr(pages, "available", lambda: False)
        return _answers(Document.open(data))


@pytest.mark.parametrize("name,edit", _edits(), ids=[name for name, _ in _edits()])
def test_each_version_laid_out_in_rust_answers_as_docx2svg(made, name, edit, monkeypatch):
    document = Document.open(made)
    document.layout()
    edit(document)
    assert pages.is_rust(convert(document).raw)
    assert _answers(document) == _python(document.to_bytes(), monkeypatch)


def test_a_run_of_edits_in_rust_answers_as_docx2svg(made, monkeypatch):
    document = Document.open(made)
    for _, edit in _edits():
        document.layout()
        edit(document)
    assert _answers(document) == _python(document.to_bytes(), monkeypatch)


def test_the_layout_is_held_in_rust_and_never_pickled(made):
    document = Document.open(made)
    raw = convert(document).raw
    assert pages.is_rust(raw) and raw.weight > 0
    with pytest.raises(TypeError):
        pickle.dumps(raw)


def _floating(data: bytes) -> bytes:
    """``data`` with its first table floating (``w:tblpPr``)."""
    import io
    import zipfile

    source, out = zipfile.ZipFile(io.BytesIO(data)), io.BytesIO()
    with zipfile.ZipFile(out, "w", zipfile.ZIP_DEFLATED) as target:
        for info in source.infolist():
            body = source.read(info)
            if info.filename == "word/document.xml":
                body = body.replace(b"<w:tblPr>", b'<w:tblPr><w:tblpPr w:tblpX="500" w:tblpY="500"/>', 1)
            target.writestr(info, body)
    return out.getvalue()


def test_a_document_the_rust_layout_does_not_lay_out_falls_back_counted(made):
    before = pages.fallbacks().get("floating table", 0)
    document = Document.open(_floating(made))
    assert document.layout().page_count >= 1
    assert not pages.is_rust(convert(document).raw)
    assert pages.fallbacks()["floating table"] == before + 1
