"""Checks read the version they are called on (lane O, docx-agent 0.1.112; round 33, R16c),
on the Rust session (P4e, lane CUT-D: from ``test_check_owner.py`` and
``test_unlocked_layouts.py``, whose layout owner and Python layouts are gone).

* A check of a version answers as a check of that version alone, in a session that opened
  its bytes: after edits, after an undo (never the layout of the version undone, though its
  number is higher), after tracking was set (no new version).
* A batch's check after its last change answers of the batch's version, as a check alone
  would; one between its changes holds the document and says so, with the way out.
* ``word_fields`` ``update`` writes what an update of the same text in another session
  writes, also with edits side by side.

Made-up documents only.
"""

from __future__ import annotations

import copy
import functools
import threading

import pytest
from ooxml_edit.tools import Toolbox

#: What the Python host said of a check between a batch's changes (before Phase 3 E2): never said
#: where the check comes after the batch's last change.
BATCH_CHECK = ("check laid the document out between the batch's changes, holding it: other calls waited. "
               "Put check after the batch's last change (or call it alone): there it lets the document go "
               "while it lays out")
#: What a check between the changes of a batch on one document says on the Rust host (user decision
#: 2026-11-25, optimistic batches; host known differences): the batch holds nothing, and runs again
#: where it conflicts.
OPTIMISTIC_CHECK = ("check laid the document out between the batch's changes, and lays it out again each time the "
                    "batch runs again. Put check after the batch's last change (or call it alone)")
from docx_agent.tools import FORMAT, GROUPS, TOOLS
from word_docs import blank, blocks, call, paragraphs, saved

pytestmark = pytest.mark.any_faces

WORDS = "lantern meadow river stone harbour quiet orchard copper valley signal".split()
INCLUDE = ["validate", "layout", "reflow", "fields"]


def _text(seed: int, words: int) -> str:
    return " ".join(WORDS[(seed + k) % len(WORDS)] for k in range(words)).capitalize() + "."


def _box(**options) -> Toolbox:
    return Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS, workers=0, **options)


@functools.lru_cache(maxsize=None)
def report(chapters: int = 3, paragraphs_per_chapter: int = 4) -> bytes:
    """A made-up report: a table of contents, chapters on new pages, a page number in the
    footer."""
    parts = ["# Report title", "An opening paragraph."]
    for c in range(1, chapters + 1):
        parts.append(f"# Chapter {c}")
        parts += [_text(c + p, 90) for p in range(paragraphs_per_chapter)]
    with _box() as box:
        session = box.session()
        doc = session.open(blank(), "report.docx")
        call(box, session, "word_insert_markdown", doc=doc, at="end", markdown="\n\n".join(parts))
        found = blocks(box, session, doc)
        heads = [block for block, text in found if text.startswith("# Chapter ")]
        call(box, session, "word_format", doc=doc, targets=heads, page_break_before=True)
        call(box, session, "word_fields", doc=doc, action="insert_toc", after=found[1][0], levels="1-1")
        call(box, session, "word_headers_footers", doc=doc, section="s:body", which="footer", action="set",
             text="Page ", page_number="after_text")
        return saved(session, doc)


@functools.lru_cache(maxsize=None)
def figures() -> bytes:
    """A made-up report of four parts, each a heading, text with a figure, a list, a table of
    amounts and a long paragraph."""
    parts = ["# Made-up report"]
    for c in range(1, 5):
        parts += [f"# Part {c}", f"{_text(c, 60)} Rated {c}.5 of 5 in part {c}.",
                  "- first point\n- second point",
                  f"| Item | Amount |\n|---|---|\n| lantern | {c}2.5 |\n| meadow | {c}7.25 |",
                  _text(c + 3, 160)]
    with _box() as box:
        session = box.session()
        doc = session.open(blank(), "report.docx")
        call(box, session, "word_insert_markdown", doc=doc, at="end", markdown="\n\n".join(parts))
        return saved(session, doc)


def long_paragraphs(session, doc) -> list[str]:
    return [block for block, text in paragraphs(session, doc) if len(text) > 200]


def _heads(box, session, doc) -> list[str]:
    return [block for block, text in blocks(box, session, doc) if text.startswith("# Part ")]


def _same(result) -> dict:
    """A check's result without its timing, the version note and what the last check saw."""
    data = copy.deepcopy(result.data)
    data.pop("timing", None)
    if isinstance(data.get("reflow"), dict):
        data["reflow"].pop("since_last_check", None)
    return {"data": data, "warnings": [w for w in result.warnings if "newer than at the call" not in w]}


def _alone(data: bytes, arguments: dict) -> dict:
    """A check of ``data`` opened in a session of its own."""
    with _box() as box:
        session = box.session()
        doc = session.open(data, "report.docx")
        return _same(call(box, session, "check", doc=doc, **arguments))


# -- a check of a version is a check of that version alone ----------------------------------------


def test_a_check_reports_what_a_check_of_its_version_alone_reports():
    with _box() as box:
        session = box.session()
        doc = session.open(figures(), "report.docx")
        heads = _heads(box, session, doc)
        got = [(call(box, session, "check", doc=doc, include=INCLUDE), {"include": INCLUDE}, saved(session, doc))]
        for k in range(2):
            call(box, session, "word_insert_markdown", doc=doc, at=f"before:{heads[k + 1]}",
                 markdown=f"A new paragraph {k}: rated 4.{k}.")
            for arguments in ({"include": INCLUDE, "range": [f"{heads[k]}..{heads[k + 1]}"]},
                              {"include": ["validate", "reflow"]}):
                got.append((call(box, session, "check", doc=doc, **arguments), arguments, saved(session, doc)))
    for result, arguments, data in got:
        mine = _same(result)
        # (validate compares with the problems the document opened with: the same here.)
        assert mine == _alone(data, arguments), arguments


def test_a_check_after_an_undo_reads_the_undone_document():
    """A check called after an undo reads the document as the undo left it -- never the layout
    of the version before the undo, though that one's number is higher."""
    with _box() as box:
        session = box.session()
        doc = session.open(figures(), "report.docx")
        heads = _heads(box, session, doc)
        call(box, session, "check", doc=doc)
        inserted = call(box, session, "word_insert_markdown", doc=doc, at=f"before:{heads[1]}",
                        markdown="A paragraph the undo takes out.")
        edited = call(box, session, "check", doc=doc, include=INCLUDE)
        assert edited.version == inserted.version
        call(box, session, "undo", doc=doc)
        now = session.save(doc).version
        assert now < edited.version
        after = call(box, session, "check", doc=doc, include=INCLUDE)
        assert after.version == now, (after.version, now)
        assert not [w for w in after.warnings if "newer than at the call" in w]
        data = saved(session, doc)
    assert _same(after) == _alone(data, {"include": INCLUDE})
    assert "A paragraph the undo takes out." not in str(after.data)


def test_a_check_reads_its_version_as_committed_last_with_its_tracking():
    """0.1.113: a version committed again with tracking (no new version) is read as it was
    committed last: the check answers of that version, with no note that it is newer."""
    with _box() as box:
        session = box.session()
        doc = session.open(figures(), "report.docx")
        first = call(box, session, "check", doc=doc)
        call(box, session, "word_set_tracking", doc=doc, on=True, author="A. Writer")
        assert session.save(doc).version == first.version
        result = call(box, session, "check", doc=doc)
        assert result.version == first.version
        assert not [w for w in result.warnings if "newer than at the call" in w]


# -- a batch's check ---------------------------------------------------------------------------------


def test_a_check_after_the_batch_s_last_change_lays_out_unlocked():
    with _box() as box:
        session = box.session()
        doc = session.open(report(), "report.docx")
        target = long_paragraphs(session, doc)[2]
        result = call(box, session, "batch", ops=[
            {"tool": "word_set_text", "arguments": {"doc": doc, "target": target, "text": "Shorter now."}},
            {"tool": "check", "arguments": {"doc": doc, "include": ["reflow"]}}])
    check = result.data["ops"][1]
    assert check["tool"] == "check" and check["data"]["reflow"]["pages"] >= 4
    assert not any(BATCH_CHECK in w or OPTIMISTIC_CHECK in w for w in result.warnings)


def test_a_check_between_a_batch_s_changes_holds_the_document_and_says_so():
    with _box() as box:
        session = box.session()
        doc = session.open(report(), "report.docx")
        targets = long_paragraphs(session, doc)
        result = call(box, session, "batch", ops=[
            {"tool": "word_set_text", "arguments": {"doc": doc, "target": targets[0], "text": "One."}},
            {"tool": "check", "arguments": {"doc": doc, "include": ["reflow"]}},
            {"tool": "word_set_text", "arguments": {"doc": doc, "target": targets[1], "text": "Two."}}])
    assert f"ops[1]: {OPTIMISTIC_CHECK}" in result.warnings


@pytest.mark.parametrize("atomic", [False, True])
def test_a_check_after_an_atomic_or_partial_batch_answers_of_its_version(atomic):
    with _box() as box:
        session = box.session()
        doc = session.open(report(), "report.docx")
        target = long_paragraphs(session, doc)[0]
        before = session.save(doc).version
        result = call(box, session, "batch", atomic=atomic, ops=[
            {"tool": "word_set_text", "arguments": {"doc": doc, "target": target, "text": "Short."}},
            {"tool": "check", "arguments": {"doc": doc, "include": ["reflow", "fields"]}}])
        alone = call(box, session, "check", doc=doc, include=["reflow", "fields"])
    assert result.version == before + 1
    check = result.data["ops"][1]["data"]
    assert check["reflow"]["pages"] == alone.data["reflow"]["pages"]
    assert check["fields"] == alone.data["fields"]


# -- word_fields update -----------------------------------------------------------------------------


def _fields(box, session, doc) -> list[tuple[str, str]]:
    return [(f["instruction"], f["result"]) for f in call(box, session, "word_fields", doc=doc, action="list").data]


def _updated_alone(data: bytes) -> tuple[str, list[tuple[str, str]]]:
    """The text and fields of ``data`` updated in a session of its own."""
    with _box() as box:
        session = box.session()
        doc = session.open(data, "report.docx")
        call(box, session, "word_fields", doc=doc, action="update")
        return [text for _, text in paragraphs(session, doc)], _fields(box, session, doc)


def test_word_fields_update_writes_what_it_wrote_before():
    """The same results as an update of the same text in a session of its own."""
    with _box() as box:
        session = box.session()
        doc = session.open(report(), "report.docx")
        target = long_paragraphs(session, doc)[1]
        call(box, session, "word_set_text", doc=doc, target=target, text="A much shorter paragraph.")
        expected = _updated_alone(saved(session, doc))
        call(box, session, "word_fields", doc=doc, action="update")
        assert ([text for _, text in paragraphs(session, doc)], _fields(box, session, doc)) == expected


def test_concurrent_field_updates_and_edits_stay_consistent():
    """Free-threading safe: updates side by side with edits, each one undo step and at most
    one version, every result as a lone update of the final text gives."""
    with _box() as box:
        session = box.session()
        doc = session.open(report(), "report.docx")
        body = long_paragraphs(session, doc)
        before = session.save(doc).version
        results: list = []
        lock = threading.Lock()

        def work(k: int) -> None:
            if k % 2:
                found = box.dispatch(session, "word_fields", {"doc": doc, "action": "update"})
            else:
                found = box.dispatch(session, "word_set_text", {
                    "doc": doc, "target": body[k], "text": f"Edit {k}. " * (5 + 10 * k)})
            with lock:
                results.append(found)

        threads = [threading.Thread(target=work, args=(k,)) for k in range(6)]
        for thread in threads:
            thread.start()
        for thread in threads:
            thread.join(120)
        assert len(results) == 6 and all(r.ok for r in results), [r.to_json() for r in results if not r.ok]
        changed = sum(1 for r in results if r.tool == "word_set_text" or r.data["changed_results"])
        version = session.save(doc).version
        assert before + changed - 3 <= version <= before + 6
        call(box, session, "word_fields", doc=doc, action="update")
        assert _fields(box, session, doc) == _updated_alone(saved(session, doc))[1]
