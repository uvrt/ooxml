"""``word_fields`` update read from the Rust owner (0.1.117, :mod:`docx_agent.rust.fields`)
says and writes what docx-agent's own update does.

The same made-up sequences -- headings and text inserted, fields updated (all of them, some
of them, in a batch with other edits), undo and redo, several writers, checks -- run with
the path on and off (``DOCX_AGENT_RUST_FIELDS``), and after every step the two are
compared: the call's result (its timing aside), the document's bytes, its version.  What
the path does not serve (a short document, a field it does not compute, a document the
owner does not model) falls back to docx-agent's own update, counted.  Made-up documents
only; without ``ooxml-core`` these tests are skipped.
"""

from __future__ import annotations

import copy
import datetime as dt
import random

import pytest

ooxml_core = pytest.importorskip("ooxml_core", reason="needs ooxml-core (the Rust path)")
if not hasattr(ooxml_core, "field_pages"):
    pytest.skip("needs ooxml-core 0.4.0 (field_pages)", allow_module_level=True)

from ooxml_edit.tools import Toolbox  # noqa: E402
from ooxml_edit.tools.limits import Limits  # noqa: E402

from docx_agent import Document  # noqa: E402
from docx_agent.rust import fields as rust_fields  # noqa: E402
from docx_agent.tools import FORMAT, GROUPS, TOOLS  # noqa: E402

INCLUDE = ["validate", "layout", "numbers", "reflow", "fields"]
CLOCK = lambda: dt.datetime(2026, 11, 8, 9, 0, tzinfo=dt.timezone.utc)  # noqa: E731
WORDS = ("lantern meadow river stone harbour quiet orchard copper valley signal morning ledger "
         "window garden timber bridge market evening canvas thread pebble summit").split()
LIMITS = Limits(render_timeout=600, layout_timeout=600, batch_timeout=900)

pytestmark = pytest.mark.any_faces


def _sentence(rng: random.Random, n: int) -> str:
    return " ".join(rng.choice(WORDS) for _ in range(n)).capitalize() + "."


def _chapter(rng: random.Random, n: int, sections: int) -> str:
    parts = [f"# Chapter {n} {_sentence(rng, 2)[:-1]}"]
    for sub in range(sections):
        parts.append(f"## Section {n}.{sub + 1} {_sentence(rng, 2)[:-1]}")
        parts += [" ".join(_sentence(rng, rng.randint(10, 22)) for _ in range(4)) for _ in range(2)]
        if sub % 3 == 1:
            parts.append(f"- {_sentence(rng, 6)}\n- {_sentence(rng, 8)}\n  - {_sentence(rng, 5)}")
        if sub % 3 == 0:
            parts.append("| Item | Value | Comment |\n|---|---|---|\n" + "\n".join(
                f"| {_sentence(rng, 2)[:-1]} | {rng.randint(1, 999)} | {_sentence(rng, 5)} |" for _ in range(6)))
    parts.append(f"End of chapter {n}.")
    return "\n\n".join(parts)


def make_report(chapters: int, *, caption: bool = False) -> bytes:
    """A contents, ``chapters`` chapters with lists and tables, a bookmark with a page
    cross-reference and a text one, and "page X of Y" in the footer; its fields updated."""
    rng = random.Random(83)
    document = Document.new()
    document.paragraphs()[0].set_text("Made-up report")
    first = document.paragraphs()[0].id
    with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS, workers=0, limits=LIMITS) as box:
        session = box.session(clock=CLOCK)
        doc = session.open(document.to_bytes(), "report.docx")

        def call(tool, **arguments):
            out = box.dispatch(session, tool, {"doc": doc, **arguments}).to_json()
            assert out.get("ok", True), out
            return out

        call("word_fields", action="insert_toc", after=first)
        for n in range(1, chapters + 1):
            call("word_insert_markdown", at="end", markdown=_chapter(rng, n, 7))
        call("word_headers_footers", section="s:body", which="footer", action="set", text="Made-up report",
             page_number="page_x_of_y")
        document = session.entry(doc).document
        heads = [p for p in document.paragraphs() if p.text.startswith("Section ")]
        call("word_links", action="add_bookmark", target=heads[-2].id, bookmark="later")
        ends = [p for p in document.paragraphs() if p.text.startswith("End of chapter ")]
        call("word_fields", action="insert_cross_reference", at=f"{ends[0].id}@{len(ends[0].text)}", bookmark="later", kind="page")
        call("word_fields", action="insert_cross_reference", at=f"{ends[-1].id}@{len(ends[-1].text)}", bookmark="later", kind="text")
        if caption:
            call("word_fields", action="insert_caption", after=heads[1].id, text="A made-up figure")
        call("word_fields", action="update")
        return session.entry(doc).document.to_bytes()


@pytest.fixture(scope="module")
def report() -> bytes:
    return make_report(6)            # (about 30 pages: a short one is updated as before)


def _normal(result: dict) -> dict:
    out = copy.deepcopy(result)
    for key in ("duration", "block"):
        out.pop(key, None)
    data = out.get("data")
    if isinstance(data, dict):
        data.pop("timing", None)
        for op in data.get("ops") or []:
            if isinstance(op, dict):
                op.pop("duration", None)
                if isinstance(op.get("data"), dict):
                    op["data"].pop("timing", None)
    return out


def _sequence(session, doc, *, check_first: bool = True):
    document = session.entry(doc).document
    paragraphs = document.paragraphs()
    at = max(k for k, p in enumerate(paragraphs) if p.text.startswith("Chapter 1"))
    body = paragraphs[at:]
    chapters = [p for p in body if p.text.startswith("Chapter ")]
    heads = [p for p in body if p.text.startswith("Section ")]
    ends = [p for p in body if p.text.startswith("End of chapter ")]
    longs = [p for p in body if len(p.text) > 200]
    one = f"{chapters[0].id}..{ends[0].id}"
    rng = random.Random(9)
    text = lambda n=12: _sentence(rng, n)  # noqa: E731
    steps = [("a", "check", {"range": [one], "include": INCLUDE})] if check_first else []
    return steps + [
        ("a", "word_insert_markdown", {"at": f"before:{ends[0].id}", "markdown": f"## Added {text(2)}\n\n{text(30)}"}),
        ("a", "word_fields", {"action": "update"}),
        ("b", "check", {"range": [one], "include": INCLUDE}),
        ("b", "word_set_text", {"target": longs[1].id, "text": longs[1].text + " " + text(60)}),
        ("a", "word_fields", {"action": "update"}),
        ("a", "word_fields", {"action": "update"}),
        ("b", "word_set_text", {"target": longs[3].id, "text": text(20)}),
        ("b", "undo", {}),
        ("a", "undo", {}),
        ("a", "undo", {"redo": True}),
        ("b", "word_set_text", {"target": longs[4].id, "text": text(25)}),
        ("a", "word_fields", {"action": "update"}),
        ("c", "batch", {"ops": [
            {"tool": "word_insert_markdown", "arguments": {"doc": doc, "at": f"before:{heads[5].id}",
                                                           "markdown": f"## Inserted {text(2)}\n\n{text(40)}"}},
            {"tool": "word_fields", "arguments": {"doc": doc, "action": "update"}}]}),
        ("c", "batch", {"ops": [
            {"tool": "word_fields", "arguments": {"doc": doc, "action": "update"}},
            {"tool": "word_set_text", "arguments": {"doc": doc, "target": longs[-1].id, "text": text(30)}}]}),
        ("a", "word_fields", {"action": "update", "ids": ["fld:body/0"]}),
        ("a", "word_delete", {"target": longs[5].id}),
        ("a", "word_fields", {"action": "update"}),
        ("a", "undo", {}),
        ("b", "check", {"range": [one], "include": INCLUDE}),
        ("a", "undo", {"redo": True}),
        ("b", "word_set_text", {"target": longs[0].id, "text": text(10)}),
    ]


def _run(data: bytes, on: bool, monkeypatch, *, check_first: bool = True) -> tuple[list, dict]:
    monkeypatch.setenv("DOCX_AGENT_RUST", "1")
    monkeypatch.setenv(rust_fields.ENV, "1" if on else "0")
    rust_fields.STATS.clear()
    out = []
    with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS, workers=0, limits=LIMITS) as box:
        session = box.session(clock=CLOCK)
        doc = session.open(data, "report.docx")
        for writer, tool, arguments in _sequence(session, doc, check_first=check_first):
            view = session.view(context={"agent": writer})
            arguments = dict(arguments)
            if tool != "batch":
                arguments["doc"] = doc
            result = box.dispatch(view, tool, arguments).to_json()
            entry = session.entry(doc)
            with entry.lock:
                out.append((tool, arguments, result, entry.document.to_bytes(), entry.version))
    return out, dict(rust_fields.STATS)


def _compare(mine: list, theirs: list) -> None:
    assert len(mine) == len(theirs)
    for (tool, arguments, a, data_a, version_a), (_, _, b, data_b, version_b) in zip(mine, theirs):
        assert a.get("ok", True), (tool, arguments, a)
        assert _normal(a) == _normal(b), (tool, arguments)
        assert data_a == data_b, (tool, arguments)
        assert version_a == version_b, (tool, arguments)


def test_fields_update_read_from_the_owner_says_what_docx_agent_says(report, monkeypatch):
    mine, stats = _run(report, True, monkeypatch)
    theirs, off = _run(report, False, monkeypatch)
    assert stats.get("served", 0) >= 8, stats
    assert not off, off
    _compare(mine, theirs)


def test_an_owner_is_opened_for_the_update(report, monkeypatch):
    """No check before the update: the update opens the document's owner itself."""
    mine, stats = _run(report, True, monkeypatch, check_first=False)
    theirs, _ = _run(report, False, monkeypatch, check_first=False)
    assert stats.get("opened", 0) == 1 and stats.get("served", 0) >= 8, stats
    _compare(mine, theirs)


@pytest.mark.parametrize("name", ["short", "caption"])
def test_what_is_not_served_is_updated_docx_agent_s_way(name, monkeypatch):
    """A short document (its next change's reflow reads the layout kept) and a document
    with a caption's SEQ (the owner does not model it): docx-agent's own update, counted."""
    data = make_report(2, caption=name == "caption")
    mine, stats = _run(data, True, monkeypatch)
    theirs, _ = _run(data, False, monkeypatch)
    assert not stats.get("served"), stats
    assert stats.get("fallback: a short document" if name == "short" else "fallback: no owner"), stats
    _compare(mine, theirs)


def test_off_by_environment(report, monkeypatch):
    monkeypatch.setenv("DOCX_AGENT_RUST", "0")
    rust_fields.STATS.clear()
    with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS, workers=0, limits=LIMITS) as box:
        session = box.session(clock=CLOCK)
        doc = session.open(report, "report.docx")
        assert box.dispatch(session, "word_fields", {"doc": doc, "action": "update"}).ok
    assert not rust_fields.STATS
