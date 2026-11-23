"""Many writers on one long document (round 59, R135): what the reads a writer makes after
each change say on the Rust session.

- ``word_read`` of a span numbers a list that runs on across chapters as the whole body does.
- ``describe`` counts the comments' threads, the open ones and each author's.
- A batch's page cross-references read the pages their bookmarks are on.

Made-up documents only."""

from __future__ import annotations

import datetime
import functools

import pytest

from ooxml_edit.tools import Toolbox

from docx_agent.tools import FORMAT, GROUPS, TOOLS

import word_docs

pytestmark = pytest.mark.any_faces

WORDS = "lantern meadow river stone harbour quiet orchard copper valley signal".split()
_W = "{http://schemas.openxmlformats.org/wordprocessingml/2006/main}"


def _clock():
    return datetime.datetime(2026, 11, 17, 9, 0, tzinfo=datetime.timezone.utc)


def _box(workers: int = 0) -> Toolbox:
    return Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS, workers=workers)


@functools.lru_cache(maxsize=2)
def made(page_breaks: bool = False) -> bytes:
    """Chapters with a numbered list that runs on across them, a table and a comment each;
    ``page_breaks``: each chapter starting a page."""
    parts = ["# Report title"]
    for c in range(1, 6):
        parts.append(f"# Chapter {c}")
        parts.append(" ".join(WORDS[(c + k) % len(WORDS)] for k in range(40)).capitalize() + ".")
        parts.append(f"1. step {c}a\n2. step {c}b")
        parts.append(f"| A{c} | B |\n|---|---|\n| **{c}** | 2 |")
        parts.append(f"End of chapter {c}.")
    with _box() as box:
        session = box.session(clock=_clock)
        doc = session.open(word_docs.blank(), "report.docx")
        word_docs.call(box, session, "word_insert_markdown", doc=doc, at="end", markdown="\n\n".join(parts))
        listed = word_docs.paragraphs(session, doc)
        steps = [block for block, text in listed if text.startswith("step ")]
        # one list, numbered on across the chapters
        for first in steps[2::2]:
            word_docs.call(box, session, "word_lists", doc=doc, action="continue", targets=[first])
        ends = [block for block, text in listed if text.startswith("End of chapter")]
        for k, end in enumerate(ends):
            word_docs.call(box, session, "word_comments", doc=doc, action="add",
                           author="Ada Morgan" if k % 2 else "Ben Ortiz",
                           items=[{"target": f"{end}@0:3", "text": f"Remark {k}."}])
        if page_breaks:
            heads = [block for block, text in listed if text.startswith("Chapter ")]
            word_docs.call(box, session, "word_format", doc=doc, targets=heads, page_break_before=True)
        return word_docs.saved(session, doc)


def _chapters(session, doc) -> list[tuple[str, str]]:
    listed = word_docs.paragraphs(session, doc)
    heads = [block for block, text in listed if text.startswith("Chapter ")]
    ends = [block for block, text in listed if text.startswith("End of chapter ")]
    return list(zip(heads, ends))


def test_a_span_numbers_its_list_as_the_whole_body_does():
    with _box() as box:
        session = box.session(clock=_clock)
        doc = session.open(made(), "report.docx")
        head, end = _chapters(session, doc)[2]
        text = word_docs.read(box, session, doc, range=f"{head}..{end}")
        # The list runs on from the chapters before: its items are 5. and 6., as read whole.
        assert "5. step 3a" in text and "6. step 3b" in text
        assert "5. step 3a" in word_docs.read(box, session, doc)


def test_describe_counts_the_comments():
    with _box() as box:
        session = box.session(clock=_clock)
        doc = session.open(made(), "report.docx")
        result = box.dispatch(session, "describe", {"doc": doc})
        assert result.ok, result.to_json()
        assert result.data["comments"] == {"threads": 5, "open": 5, "by_author": {"Ben Ortiz": 3, "Ada Morgan": 2}}


def _page_references(root) -> list[tuple[str, str]]:
    """Each PAGEREF field's instruction and its result's text, in the order of the body."""
    out, instruction, result, state = [], [], [], None
    for node in root.find(_W + "body").iter(_W + "fldChar", _W + "instrText", _W + "t", _W + "fldSimple"):
        if node.tag == _W + "fldSimple" and "PAGEREF" in node.get(_W + "instr", ""):
            out.append((node.get(_W + "instr").strip(), "".join(t.text or "" for t in node.iter(_W + "t"))))
        elif node.tag == _W + "fldChar":
            kind = node.get(_W + "fldCharType")
            if kind == "begin":
                instruction, result, state = [], [], "instruction"
            elif kind == "separate":
                state = "result"
            elif kind == "end" and state is not None:
                if "PAGEREF" in "".join(instruction):
                    out.append(("".join(instruction).strip(), "".join(result)))
                state = None
        elif node.tag == _W + "instrText" and state == "instruction":
            instruction.append(node.text or "")
        elif node.tag == _W + "t" and state == "result":
            result.append(node.text or "")
    return out


def test_a_batchs_page_references_read_their_pages():
    with _box() as box:
        session = box.session(clock=_clock)
        doc = session.open(made(page_breaks=True), "report.docx")
        assert box.dispatch(session, "check", {"doc": doc, "include": ["reflow"]}).ok     # its layout held
        chapters = _chapters(session, doc)
        ops = [{"tool": "word_links", "arguments": {"doc": doc, "action": "add_bookmark", "target": head,
                                                    "bookmark": f"chapter{k + 1}"}}
               for k, (head, _) in enumerate(chapters)]
        ops += [{"tool": "word_fields", "arguments": {"doc": doc, "action": "insert_cross_reference",
                                                      "at": f"{end}@0", "kind": "page",
                                                      "bookmark": f"chapter{k + 2}"}}
                for k, (_, end) in enumerate(chapters[:-1])]
        result = box.dispatch(session, "batch", {"ops": ops})
        assert result.ok, result.to_json()
        found = _page_references(word_docs.tree(word_docs.saved(session, doc)))
        assert [page for _, page in found] == ["3", "4", "5", "6"], found


def test_continue_with_several_targets_continues_each():
    """``word_lists`` continue given several lists' first items: each list numbers on from the
    one before it."""
    with _box() as box:
        session = box.session(clock=_clock)
        doc = session.open(word_docs.blank(), "lists.docx")
        word_docs.call(box, session, "word_insert_markdown", doc=doc, at="end",
                       markdown="1. a1\n2. a2\n\nText.\n\n1. b1\n2. b2\n\nMore.\n\n1. c1\n2. c2")
        firsts = [block for block, text in word_docs.paragraphs(session, doc) if text in ("b1", "c1")]
        word_docs.call(box, session, "word_lists", doc=doc, action="continue", targets=firsts)
        text = word_docs.read(box, session, doc)
        assert "3. b1" in text and "5. c1" in text, text

