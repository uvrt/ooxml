"""The Rust parser (lane I1, :mod:`docx_agent.rust.parser`) gives the Rust owner what the
Python mirror gives it, byte for byte: the first input of a document and every change set
after an edit.

The recorded sequences of ``test_rust_check`` (Markdown inserts with lists and tables, text
set, a table's rows added and filled, the fields updated, undo, several writers, checks) run
with the Rust parser on and off (``DOCX_AGENT_RUST_PARSER``); every text handed to the owner
is compared, and every tool's result.  Made-up documents only.
"""

from __future__ import annotations

import pytest

pytest.importorskip("ooxml_edit.tools.numbers", reason="needs ooxml-edit (numbers)")
ooxml_core = pytest.importorskip("ooxml_core", reason="needs ooxml-core (the Rust path)")
if not hasattr(ooxml_core, "Parser"):
    pytest.skip("needs ooxml-core 0.5.0 (the Rust parser)", allow_module_level=True)

from ooxml_edit.tools import Toolbox  # noqa: E402
from ooxml_edit.tools.limits import Limits  # noqa: E402

from docx_agent import Document  # noqa: E402
from docx_agent.rust import check as rust_check  # noqa: E402
from docx_agent.rust import parser as rust_parser  # noqa: E402
from docx_agent.tools import FORMAT, GROUPS  # noqa: E402

from test_rust_check import CLOCK, INCLUDE, TOOLS, _normal, _sequence, report  # noqa: E402,F401

pytestmark = pytest.mark.any_faces


def _run(data: bytes, parser: bool, monkeypatch, sequence=_sequence) -> tuple[list, list, dict]:
    """Every text handed to the owner (``open_document``'s and ``commit``'s, in order), every
    result, and the parser's statistics."""
    monkeypatch.setenv("DOCX_AGENT_RUST", "1")
    monkeypatch.setenv(rust_parser.ENV, "1" if parser else "0")
    rust_check.STATS.clear()
    rust_parser.STATS.clear()
    sent: list = []
    opened, committed = ooxml_core.open_document, ooxml_core.commit

    def open_document(key, kind, text):
        sent.append(("open", text))
        return opened(key, kind, text)

    def commit(key, text):
        sent.append(("commit", text))
        return committed(key, text)

    monkeypatch.setattr(ooxml_core, "open_document", open_document)
    monkeypatch.setattr(ooxml_core, "commit", commit)
    results = []
    limits = Limits(render_timeout=600, layout_timeout=600, batch_timeout=900)
    with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS, workers=1, limits=limits) as box:
        session = box.session(clock=CLOCK)
        doc = session.open(data, "report.docx")
        assert box.dispatch(session, "ds_put", {"id": "values"}).ok
        for writer, tool, arguments in sequence(session, doc):
            view = session.view(context={"agent": writer})
            results.append((tool, box.dispatch(view, tool, {"doc": doc, **arguments}).to_json()))
    monkeypatch.setattr(ooxml_core, "open_document", opened)
    monkeypatch.setattr(ooxml_core, "commit", committed)
    return sent, results, dict(rust_parser.STATS)


def _first_text_difference(a: str, b: str) -> str:
    k = next((k for k, (x, y) in enumerate(zip(a, b)) if x != y), min(len(a), len(b)))
    return f"at {k} of {len(a)}/{len(b)}: {a[max(0, k - 120):k + 120]!r} != {b[max(0, k - 120):k + 120]!r}"


def test_the_owner_gets_what_the_mirror_gives_it(report, monkeypatch):
    mine, mine_results, stats = _run(report, True, monkeypatch)
    theirs, their_results, _ = _run(report, False, monkeypatch)
    assert stats.get("exported", 0) >= 1, stats
    assert stats.get("splices", 0) >= 5, stats
    assert not any(k.startswith(("stale", "refused")) for k in stats), stats
    assert [kind for kind, _ in mine] == [kind for kind, _ in theirs]
    for n, ((kind, a), (_, b)) in enumerate(zip(mine, theirs)):
        assert a == b, (n, kind, _first_text_difference(a, b))
    for (tool, a), (_, b) in zip(mine_results, their_results):
        assert _normal(a) == _normal(b), tool


def test_off_by_environment(report, monkeypatch):
    """``DOCX_AGENT_RUST_PARSER=0``: the mirror exports the document, as before."""
    _, _, stats = _run(report, False, monkeypatch, sequence=lambda session, doc: [
        ("a", "check", {"range": [session.entry(doc).document.paragraphs()[-1].id], "include": INCLUDE})])
    assert not stats, stats


def test_a_document_the_parser_does_not_read_is_mirrored_as_before(monkeypatch):
    """A footnote: the parser refuses the document whole (counted), the mirror refuses it
    too, and the check is docx-agent's own -- the same either way."""
    document = Document.new()
    document.insert_markdown("# Title\n\nSome text with a note.[^1]\n\n[^1]: The note.\n\nMore text.")
    data = document.to_bytes()

    def sequence(session, doc):
        first = session.entry(doc).document.paragraphs()[0].id
        return [("a", "check", {"range": [first], "include": INCLUDE})]

    mine, results, stats = _run(data, True, monkeypatch, sequence)
    theirs, their_results, _ = _run(data, False, monkeypatch, sequence)
    assert any(k.startswith("refused") for k in stats), stats
    assert mine == theirs
    assert [_normal(r) for _, r in results] == [_normal(r) for _, r in their_results]


_HERE = __import__("pathlib").Path(__file__).resolve().parent
_SAMPLES = sorted(str(p.relative_to(_HERE)) for p in _HERE.rglob("*.docx"))


def _mirror_export(document):
    from docx_agent.rust.exporter import Unsupported
    from docx_agent.rust.mirror import Mirror

    try:
        mirror = Mirror(document)
        rust_check._check_keys(mirror)
        mirror.fstack_end = mirror.thread_fields(mirror.units, [])
        with rust_check._records(document, mirror):
            return mirror.export_with(rust_check.constants(document, mirror))[0]
    except Unsupported:
        return None


@pytest.mark.parametrize("name", _SAMPLES)
def test_every_sample_is_exported_as_the_mirror_exports_it(name):
    """Every made-up document and sample of the tests: the parser's first input is the
    mirror's byte for byte, or the parser refuses the document whole (the mirror then
    exports it, as before) -- never a different one, never one the mirror refuses."""
    import warnings

    with warnings.catch_warnings():
        warnings.simplefilter("ignore")
        document = Document.open((_HERE / name).read_bytes())
    theirs = _mirror_export(document)
    mine, stats, _ = rust_parser.export(document)
    if mine is None:
        assert "refused" in stats
        return
    assert theirs is not None, "the parser exports a document the mirror refuses"
    assert mine == theirs, _first_text_difference(mine, theirs)


def test_the_report_is_exported_as_the_mirror_exports_it(report):
    document = Document.open(report)
    mine, stats, _ = rust_parser.export(document)
    assert mine is not None, stats
    assert mine == _mirror_export(document)
