"""Lane H3: an application's own ``Limits`` (``max_result_chars``, ``max_list_items``) are
the Rust session's too -- handed over in ``CallContext`` -- so the reading tools that page
(``find_text``, ``word_read``, ``word_design_facts``) are served with them, not sent to the
Python path; the session's listings (``word_changes`` list and preview, ``word_comments``
list) page by them, as docx-agent's own handlers do; and a served ``word_comments`` list
keeps its page's cursor and total.  Made-up documents
only."""

from __future__ import annotations

import dataclasses
import json
import re

import pytest

ooxml_core = pytest.importorskip("ooxml_core")

from ooxml_edit.tools import Limits, Toolbox  # noqa: E402
from ooxml_edit.tools.owned import OwnedEntry  # noqa: E402

from docx_agent import Document  # noqa: E402
from docx_agent.rust import edits, reads  # noqa: E402
from docx_agent.rust import served as served_tools  # noqa: E402
from docx_agent.tools import FORMAT, GROUPS, TOOLS  # noqa: E402

#: Small limits: a page of two items, a reply of 3,000 characters.
LIMITS = Limits(max_result_chars=3000, max_list_items=2)

_DATE = re.compile(r"\d{4}-\d\d-\d\dT\d\d:\d\d:\d\dZ?")


@pytest.fixture(scope="module")
def toolbox():
    with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS, workers=0, limits=LIMITS) as box:
        yield box


@pytest.fixture(scope="module")
def report() -> bytes:
    doc = Document.new()
    parts = ["# A made-up report"]
    for c in range(1, 9):
        parts += [f"# Part {c}", f"Lantern meadow {c} by the harbour, " + "with words " * 40,
                  f"River stone number {c}."]
    doc.insert_markdown("\n\n".join(parts))
    return doc.to_bytes()


def _plain(result) -> dict:
    return json.loads(_DATE.sub("<date>", json.dumps(result.to_json(), sort_keys=True, ensure_ascii=False)))


def _read(toolbox, session, tool, arguments, rust) -> dict:
    before = reads.use_session(rust)
    try:
        return _plain(toolbox.dispatch(session, tool, arguments))
    finally:
        reads.use_session(before)


@pytest.mark.parametrize("tool, arguments", [
    ("find_text", {"text": "Lantern"}),
    ("find_text", {"text": "River", "cursor": "2"}),
    ("word_read", {}),
    ("word_design_facts", {}),
    ("word_design_facts", {"include": ["declared_styles"]}),
])
def test_the_reads_that_page_are_served_with_the_hosts_limits(toolbox, report, tool, arguments):
    session = toolbox.session()
    doc = session.open(report, "report.docx")
    arguments = {"doc": doc, **arguments}
    if arguments.get("cursor") is not None:
        first = _read(toolbox, session, tool, {k: v for k, v in arguments.items() if k != "cursor"}, None)
        arguments["cursor"] = first["next_cursor"]
    rust = ooxml_core.Session()
    served = reads.served().get(tool, 0)
    answer = _read(toolbox, session, tool, arguments, rust)
    assert reads.served().get(tool, 0) == served + 1, reads.fallbacks()
    assert answer == _read(toolbox, session, tool, arguments, None)
    assert answer["ok"]
    if tool == "find_text":
        assert len(answer["data"]) == 2 and answer["next_cursor"]


def _rust(session, doc: str, tool: str, arguments: dict) -> dict:
    """``tool``'s reply from the Rust session that owns ``doc``, called with the toolbox's
    limits as an application hosting the session alone calls it (docx-agent answers these
    reading actions with its own handlers), as the toolbox's result."""
    from docx_agent.rust.owner import context_of

    mirror = session.entry(doc).handle
    options = ooxml_core.CallOptions(context=context_of(None, LIMITS),
                                     documents=ooxml_core.CallDocuments(names={doc: mirror.doc}))
    reply, _images, _app = mirror.session.call(tool, json.dumps({**arguments, "doc": doc}), options)
    return json.loads(_DATE.sub("<date>", json.dumps(reads._result(json.loads(reply)).to_json(), sort_keys=True,
                                                     ensure_ascii=False)))


def _reviewed(toolbox, session, doc: str) -> None:
    """Tracked changes by a made-up reviewer (five) and comments (four) on ``doc``."""
    ids = {p.text: p.id for p in session.entry(doc).document.paragraphs()}
    steps = [("word_set_tracking", {"on": True, "author": "Reviewer A"})]
    steps += [("word_set_text", {"target": ids[f"River stone number {c}."], "text": f"River pebble {c}."})
              for c in range(1, 6)]
    steps.append(("word_comments", {"action": "add", "items": [
        {"target": ids[f"Part {c}"], "text": f"A made-up comment {c}."} for c in range(1, 5)]}))
    for tool, arguments in steps:
        result = toolbox.dispatch(session, tool, {"doc": doc, **arguments})
        assert result.ok, (tool, result.error)


def test_the_session_s_change_listings_page_by_the_hosts_limits(toolbox, report):
    session = toolbox.session()
    doc = session.open(report, "report.docx")
    if not isinstance(session.entry(doc), OwnedEntry):
        pytest.skip("no Rust session to own the document")
    _reviewed(toolbox, session, doc)
    listings = [("word_changes", {"action": "list"}), ("word_changes", {"action": "list", "cursor": "next"}),
                ("word_changes", {"action": "preview", "all": True})]
    python = []
    for tool, arguments in listings:
        if arguments.get("cursor") == "next":
            arguments = {**arguments, "cursor": python[-1]["next_cursor"]}
        python.append(_plain(toolbox.dispatch(session, tool, {"doc": doc, **arguments})))
        # (the document and version a reply names are the toolbox's, which the session's lacks)
        mine = {k: v for k, v in python[-1].items() if k not in ("doc", "version")}
        theirs = _rust(session, doc, tool, arguments)
        if arguments["action"] == "preview":
            # the page of changes only: what the preview says of this session's records is
            # not this lane's (the session asked outside an edit counts none of them its own)
            page = ("changes", "changes_not_shown")
            assert {k: theirs["data"][k] for k in page} == {k: mine["data"][k] for k in page}
            continue
        assert theirs == {**mine, "doc": None, "version": None}, (tool, arguments)
    first, second, preview = python
    assert len(first["data"]) == 2 and first["next_cursor"] and len(second["data"]) == 2
    assert len(preview["data"]["changes"]) == 2 and preview["data"]["changes_not_shown"]["count"] > 0


def _comment_pages(toolbox, monkeypatch, data: bytes, served: bool) -> list:
    """``word_comments`` list, page by page, by the session (``served``) or the Python
    handler."""
    with monkeypatch.context() as patched:
        if not served:
            hosting = served_tools.hosting()
            fewer = dataclasses.replace(hosting, served=hosting.served - {"word_comments"})
            patched.setattr(served_tools, "_HOSTING", [fewer])
        session = toolbox.session()
        doc = session.open(data, "report.docx")
        if served and not isinstance(session.entry(doc), OwnedEntry):
            pytest.skip("no Rust session to own the document")
        _reviewed(toolbox, session, doc)
        pages, cursor = [], None
        while True:
            before = edits.served().get("word_comments", 0)
            result = toolbox.dispatch(session, "word_comments", {"doc": doc, "action": "list", "cursor": cursor})
            assert edits.served().get("word_comments", 0) == before + served, edits.fallbacks()
            pages.append(_plain(result))
            cursor = result.next_cursor
            if cursor is None:
                return pages


def test_a_served_comment_listing_pages_by_the_hosts_limits(toolbox, monkeypatch, report):
    served = _comment_pages(toolbox, monkeypatch, report, True)
    assert served == _comment_pages(toolbox, monkeypatch, report, False)
    assert [len(page["data"]) for page in served] == [2, 2] and served[0]["total"] == 4
