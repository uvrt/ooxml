"""A check of a range served by the Rust owner says the faces of the version it checks
(lane J, 0.1.120): its ``reflow`` (``coverage.substituted_fonts``, ``layout_warnings``) was
read from the owner's mirror as it was when the check got there, so an edit committed while
the check waited for its version's layout -- a paragraph in another substituted face --
showed in a check of the version before it (the free-threaded stress).  Each state the
owner is handed keeps what its faces are; the check reads its version's.

Made-up faces no machine has, laid out with a substitute either way."""

from __future__ import annotations

import datetime as dt
import threading

import pytest

pytest.importorskip("ooxml_edit.tools.numbers", reason="needs ooxml-edit (numbers)")
pytest.importorskip("ooxml_core", reason="needs ooxml-core (the Rust path)")

from lxml import etree  # noqa: E402
from ooxml_edit.tools import Toolbox  # noqa: E402

from docx_agent import Document  # noqa: E402
from docx_agent.rust import check as rust_check  # noqa: E402
from docx_agent.rust import reads  # noqa: E402
from docx_agent.tools import FORMAT, GROUPS, TOOLS  # noqa: E402

W = "{http://schemas.openxmlformats.org/wordprocessingml/2006/main}"
CLOCK = lambda: dt.datetime(2026, 11, 9, 9, 0, tzinfo=dt.timezone.utc)  # noqa: E731


def _report() -> bytes:
    document = Document.new()
    document.paragraphs()[0].set_text("Start of the report.")
    document.insert_markdown("# Part\n\nText in Beta.\n\nThe end of the part.", at="end")
    run = next(p for p in document.paragraphs() if p.text == "Text in Beta.")._element.find(f".//{W}r")
    properties = etree.Element(W + "rPr")
    fonts = etree.SubElement(properties, W + "rFonts")
    fonts.set(W + "ascii", "Beta Madeup Face")
    fonts.set(W + "hAnsi", "Beta Madeup Face")
    run.insert(0, properties)
    document.package.mark_dirty(document.package.document_part())
    return document.to_bytes()


def _faces(result) -> list[str]:
    reflow = (result.to_json().get("data") or {}).get("reflow") or {}
    return [s for s in (reflow.get("coverage") or {}).get("substituted_fonts", []) if "Madeup" in s]


def _ids(session, doc):
    paragraphs = session.entry(doc).document.paragraphs()
    return paragraphs[1].id, paragraphs[-1].id, next(p.id for p in paragraphs if p.text == "Text in Beta.")


def test_a_check_says_the_faces_of_its_version_not_of_a_later_one(monkeypatch):
    with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS, workers=0) as box:
        session = box.session(clock=CLOCK)
        doc = session.open(_report(), "report.docx")
        first, last, beta = _ids(session, doc)
        span = [f"{first}..{last}"]
        box.dispatch(session, "check", {"doc": doc, "range": span})        # the owner opened

        edits: dict = {}

        def edit() -> None:
            made = box.dispatch(session, "word_insert_markdown",
                                {"doc": doc, "at": f"before:{beta}", "markdown": "Text in Alpha."})
            alpha = next(p.id for p in session.entry(doc).document.paragraphs() if p.text == "Text in Alpha.")
            edits["made"] = made
            edits["font"] = box.dispatch(session, "word_format", {"doc": doc, "targets": [alpha],
                                                                  "font": "Alpha Madeup Face"})

        def edit_once() -> None:
            if not edits:
                thread = threading.Thread(target=edit)
                thread.start()
                thread.join()

        real = rust_check.core

        class Gated:
            """The core, whose check -- once it has the version's layout -- lets the edit
            commit before the check goes on (a gate: the edit's thread is joined)."""

            def __init__(self, inner):
                self.inner = inner

            def __getattr__(self, name):
                return getattr(self.inner, name)

            def check(self, *args, **kwargs):
                found = self.inner.check(*args, **kwargs)
                edit_once()
                return found

        class Held:
            """The Rust session, whose check -- given the version it reads -- lets the edit
            commit first: where the session answers a check of ranges whole (lane P5k), the
            version it lays out is the one it was given; where it does not (it says so), the
            owner's check that answers then starts after the edit."""

            def __init__(self, inner):
                self.inner = inner

            def __getattr__(self, name):
                return getattr(self.inner, name)

            def call(self, tool, *args, **kwargs):
                if tool == "check":
                    edit_once()
                return self.inner.call(tool, *args, **kwargs)

        mirror = reads._owned(session.entry(doc)).handle
        monkeypatch.setattr(rust_check, "core", lambda: Gated(real()))
        monkeypatch.setattr(mirror, "session", Held(mirror.session))
        version = session.entry(doc).version
        served = rust_check.STATS["served"], reads.served().get("check", 0)
        checked = box.dispatch(session, "check", {"doc": doc, "range": span})
        # served by the owner (lane F) or by the session whole -- not by the Python path
        assert (rust_check.STATS["served"], reads.served().get("check", 0)) in (
            (served[0] + 1, served[1]), (served[0], served[1] + 1))
        monkeypatch.undo()                                        # the core and the session again
        assert edits["made"].ok and edits["font"].ok
        assert checked.version == version
        assert _faces(checked) == ["Beta Madeup Face -> " + _faces(checked)[0].split(" -> ", 1)[1]]
        assert not any("Alpha" in face for face in _faces(checked))
