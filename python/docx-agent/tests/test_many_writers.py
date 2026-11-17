"""Many writers on one long document (round 59, R135): the reads a writer makes after each
change cost what they read, not the whole document.

- ``word_read`` of a span reads the span's blocks, counting only the list items before it
  (:meth:`docx_agent.markdown.read.Reader.span`), and gives what reading the whole story
  and cutting it gave.
- A read copy of a version (or a draft) parses its effective formatting with its document's
  kept blocks (:func:`docx_agent.edit.effective._kept_blocks`): the blocks a change left as
  they were are not parsed again.
- ``describe`` finds the comments' records once, not three times per comment.

Made-up documents only."""

from __future__ import annotations

import datetime
import functools

import pytest

from ooxml_edit.tools import Toolbox

from docx_agent import Document
from docx_agent.markdown import project, render
from docx_agent.markdown.read import Reader
from docx_agent.tools import FORMAT, GROUPS, TOOLS

pytestmark = pytest.mark.any_faces

WORDS = "lantern meadow river stone harbour quiet orchard copper valley signal".split()


@functools.lru_cache(maxsize=1)
def made() -> bytes:
    """Chapters with a numbered list that runs on across them, a table and a comment each."""
    doc = Document.new()
    parts = ["# Report title"]
    for c in range(1, 6):
        parts.append(f"# Chapter {c}")
        parts.append(" ".join(WORDS[(c + k) % len(WORDS)] for k in range(40)).capitalize() + ".")
        parts.append(f"1. step {c}a\n2. step {c}b")
        parts.append(f"| A{c} | B |\n|---|---|\n| **{c}** | 2 |")
        parts.append(f"End of chapter {c}.")
    doc.insert_markdown("\n\n".join(parts))
    steps = [p.id for p in doc.paragraphs() if p.text.startswith("step ")]
    one = doc.list_of(steps[0]).num_id
    for identifier in steps[2:]:
        doc.add_to_list(identifier, one)        # one list, numbered on across the chapters
    for k, paragraph in enumerate(p for p in doc.paragraphs() if p.text.startswith("End of chapter")):
        doc.add_comment(f"{paragraph.id}@0:3", f"Remark {k}.", author="Ada Morgan" if k % 2 else "Ben Ortiz")
    return doc.to_bytes()


def _chapters(document: Document) -> list[tuple[str, str]]:
    heads = [p.id for p in document.paragraphs() if p.text.startswith("Chapter ")]
    ends = [p.id for p in document.paragraphs() if p.text.startswith("End of chapter ")]
    return list(zip(heads, ends))


def _story_read(document: Document, span: str, view: str) -> str:
    """The span as the whole story's records cut (a reader that read the story already)."""
    reader = Reader(document, view)
    reader.__dict__["_stories"] = {document.package.document_part(): reader.story(document.package.document_part())}
    return render(project(document, span, view=view, reader=reader))


@pytest.mark.parametrize("view", ["final", "markup", "original"])
def test_a_span_reads_its_own_blocks_and_reads_as_the_story_cut(view, monkeypatch):
    document = Document.open(made())
    chapters = _chapters(document)
    head, end = chapters[3]
    spans = [f"{head}..{end}", f"{chapters[4][0]}..", f"..{chapters[0][1]}", f"{chapters[1][1]}..{chapters[2][0]}"]
    read = {span: _story_read(document, span, view) for span in spans}
    made_records = []
    original = Reader.paragraph
    monkeypatch.setattr(Reader, "paragraph", lambda self, *a: made_records.append(1) or original(self, *a))
    for span in spans:
        made_records.clear()
        assert document.to_markdown(span, view=view) == read[span], span
    made_records.clear()
    document.to_markdown(view=view)
    whole = len(made_records)
    made_records.clear()
    # (The test runs compare every span read alone with the story read: not for this count.)
    monkeypatch.setenv("DOCX_AGENT_FULL_VALIDATE", "0")
    document.to_markdown(f"{head}..{end}", view=view)
    # The chapter's own paragraphs (its table's cells among them), not the body's.
    assert 0 < len(made_records) < whole / 3, (len(made_records), whole)


def test_a_span_numbers_its_list_as_the_whole_body_does():
    document = Document.open(made())
    head, end = _chapters(document)[2]
    text = document.to_markdown(f"{head}..{end}")
    # The list runs on from the chapters before: its items are 5. and 6., as read whole.
    assert "5. step 3a" in text and "6. step 3b" in text
    assert "5. step 3a" in document.to_markdown()


def _clock():
    return datetime.datetime(2026, 11, 17, 9, 0, tzinfo=datetime.timezone.utc)


def test_a_read_copy_parses_with_its_documents_kept_blocks():
    with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS, workers=0) as box:
        session = box.session(clock=_clock)
        doc = session.open(made(), "report.docx")
        live = session.entry(doc).document
        head, end = _chapters(live)[1]
        first = box.dispatch(session, "word_read", {"doc": doc, "range": f"{head}..{end}"})
        assert first.ok
        target = next(p.id for p in live.paragraphs() if p.text.startswith("End of chapter 4"))
        assert box.dispatch(session, "word_set_text", {"doc": doc, "target": target, "text": "End of part 4."}).ok
        again = box.dispatch(session, "word_read", {"doc": doc, "range": f"{head}..{end}"})
        assert again.ok and again.data == first.data
        kept = live.__dict__.get("_effective_kept")
        assert kept is not None, "the read copy's parse kept nothing on its document"
        # One top-level block changed: every other one taken as it was.
        assert kept[0].hits > 0 and kept[0].built <= 2, (kept[0].hits, kept[0].built)


def test_describe_finds_the_comments_records_once(monkeypatch):
    with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS, workers=0) as box:
        session = box.session(clock=_clock)
        doc = session.open(made(), "report.docx")
        document = session.entry(doc).document
        comments = document.comments()
        expected = {"threads": len([c for c in comments if c.parent is None]),
                    "open": sum(1 for c in comments if c.parent is None and not c.done),
                    "by_author": {}}
        for c in comments:
            expected["by_author"][c.author] = expected["by_author"].get(c.author, 0) + 1
        calls = []
        original = Document._comment_records
        monkeypatch.setattr(Document, "_comment_records", lambda self: calls.append(1) or original(self))
        result = box.dispatch(session, "describe", {"doc": doc})
        assert result.ok
        assert result.data["comments"] == expected
        assert len(calls) <= 3, len(calls)          # was three per comment, and one more
