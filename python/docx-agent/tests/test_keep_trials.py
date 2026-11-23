"""Round 40: what a range's keep does to the pages, laid out (``check(range=...)``'s
``if_kept`` and ``if_released``, the Rust session's check).

Each fact is compared with the document after the call it is about: the ``fix`` made, the
document laid out again -- incrementally, as the tools do, and whole, from its bytes alone
-- and the same pages and page counts read there (``word_inspect`` and ``check``).  Made-up
documents only."""

from __future__ import annotations

import threading

import pytest

pytest.importorskip("ooxml_edit.tools.shared", reason="needs ooxml-edit 0.4 (the shared tools)")

from ooxml_edit.tools import Toolbox  # noqa: E402
from test_table_keeps import grid  # noqa: E402
from test_tools_layout_facts import FILLER, call, docx, ok, p  # noqa: E402
from layout_reads import block_ids, pages_of  # noqa: E402

from docx_agent.tools import FORMAT, GROUPS, TOOLS  # noqa: E402
import word_docs  # noqa: E402

#: The keeps a check tries at most (the Rust session's check's ``TRIALS``).
TRIALS = 4

pytestmark = pytest.mark.any_faces

WORDS = " ".join(f"word{k:02d} lorem ipsum" for k in range(40))


@pytest.fixture(scope="module")
def box():
    with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS) as toolbox:
        yield toolbox


@pytest.fixture
def session(box):
    return box.session()


def _fill(count: int) -> str:
    return "".join(p(f"{FILLER} {k}") for k in range(count))


def _span_doc(fill: int = 17) -> bytes:
    """Filler, then a span -- a heading, a long paragraph, a table of five rows, a closing
    line -- that starts on page 1 and ends on page 2, and a paragraph after it."""
    return docx(_fill(fill) + p("Head") + p(WORDS) + grid(5) + p("Closing line") + p("After") + _fill(3))


def _ids(box, session, d: str) -> list[str]:
    return block_ids(box, session, d)


def _range(box, session, d: str, *ranges: str, **arguments) -> list[dict]:
    return ok(box, session, "check", doc=d, range=list(ranges), include=["reflow"], **arguments).data["range"]


def _seen(box, session, d: str, blocks: list[str], after: str | None) -> dict:
    """The facts an effect states, read from the layout of the document as it then is."""
    pages = sorted({n for b in blocks for n in pages_of(box, session, d, b)})
    count = ok(box, session, "check", doc=d, include=["reflow"]).data["reflow"]["pages"]
    out = {"starts": pages[0], "ends": pages[-1], "document_pages": count}
    if after is not None:
        out["after"] = pages_of(box, session, d, after)[0]
    return out


def _seen_both(box, session, d: str, blocks: list[str], after: str | None) -> list[dict]:
    """:func:`_seen` of the document as the session holds it, and of its bytes opened anew."""
    fresh = session.open(word_docs.saved(session, d), "fresh.docx")
    return [_seen(box, session, d, blocks, after), _seen(box, session, fresh, blocks, after)]


def _stated(effect: dict) -> dict:
    out = {"starts": effect["starts"], "ends": effect["ends"], "document_pages": effect["document_pages"][1]}
    if "after" in effect:
        out["after"] = effect["after"][1]
    return out


def test_if_kept_is_what_the_fix_then_does_to_the_pages(box, session):
    d = session.open(_span_doc(), "kept.docx")
    ids = _ids(box, session, d)
    span = f"{ids[17]}..{ids[20]}"
    [entry] = _range(box, session, d, span)
    assert entry["one_page"] is False and entry["fix"]["calls"] == [{"targets": [span], "keep_together": True}]
    effect = entry["if_kept"]
    assert effect["starts"] == 2 and effect["ends"] == 2 and effect["one_page"] is True
    assert effect["document_pages"] == [2, 2] and effect["after"] == [2, 2]
    blank = effect["blank"]
    assert blank["page"] == 1 and blank["pt"] > blank["now_pt"] and 0 < blank["share"] < 1
    assert effect["said"].startswith("keeping it whole (this range alone): starts on page 2, on one page; "
                                     f"page 1 left {round(blank['share'] * 100)}% blank")
    assert "the document stays at 2 pages" in effect["said"]

    call(box, session, d, entry["fix"])
    ids = _ids(box, session, d)
    blocks, after = ids[17:21], ids[21]
    # Laid out incrementally, as the tools do it, and whole from the bytes alone: the same.
    assert _seen_both(box, session, d, blocks, after) == [_stated(effect)] * 2


def test_if_released_is_what_releasing_a_kept_range_does(box, session):
    d = session.open(_span_doc(), "released.docx")
    ids = _ids(box, session, d)
    span = f"{ids[17]}..{ids[20]}"
    ok(box, session, "word_format", doc=d, targets=[span], keep_together=True)
    ids = _ids(box, session, d)
    span = f"{ids[17]}..{ids[20]}"
    [entry] = _range(box, session, d, span)
    assert entry["one_page"] is True and entry["kept"] is True and "fix" not in entry and "if_kept" not in entry
    effect = entry["if_released"]
    assert effect["starts"] == 1 and effect["ends"] == 2 and effect["one_page"] is False
    assert effect["fix"] == {"tool": "word_format", "calls": [{"targets": [span], "keep_together": False}]}
    blank = effect["blank"]
    assert blank["page"] == 1 and blank["now_pt"] > blank["pt"]
    assert effect["said"].startswith("releasing its keep (this range alone): starts on page 1, over pages 1-2; "
                                     f"page 1 then {round(blank['share'] * 100)}% blank")

    call(box, session, d, effect["fix"])
    ids = _ids(box, session, d)
    assert _seen_both(box, session, d, ids[17:21], ids[21]) == [_stated(effect)] * 2
    # Released: the next check offers the keep again, with what it costs -- both sides said.
    [again] = _range(box, session, d, f"{ids[17]}..{ids[20]}")
    assert again["if_kept"]["blank"]["pt"] == blank["now_pt"]


def test_a_keep_that_adds_a_page_says_so_and_what_follows_moves(box, session):
    """A span parted at page 1's foot, page 2 filled after it: kept, the span moves to page
    2 whole and pushes what follows onto a third page."""
    d = session.open(docx(_fill(17) + p("Head") + p(WORDS) + grid(5) + p("Closing line") + _fill(14)
                          + p("Last")), "grow.docx")
    ids = _ids(box, session, d)
    assert ok(box, session, "check", doc=d, include=["reflow"]).data["reflow"]["pages"] == 2
    span = f"{ids[17]}..{ids[20]}"
    [entry] = _range(box, session, d, span)
    effect = entry["if_kept"]
    assert effect["document_pages"] == [2, 3] and effect["starts"] == 2
    assert "the document goes from 2 to 3 pages" in effect["said"]
    call(box, session, d, entry["fix"])
    ids = _ids(box, session, d)
    assert _seen(box, session, d, ids[17:21], ids[21]) == _stated(effect)


def test_a_range_starting_its_page_after_a_page_break_is_not_offered_a_release(box, session):
    d = session.open(docx(_fill(3) + p("Head", ppr="<w:pageBreakBefore/>") + p("Kept line") + p("End")), "pb.docx")
    ids = _ids(box, session, d)
    span = f"{ids[3]}..{ids[4]}"
    ok(box, session, "word_format", doc=d, targets=[span], keep_together=True)
    ids = _ids(box, session, d)
    [entry] = _range(box, session, d, f"{ids[3]}..{ids[4]}")
    assert entry["kept"] is True and entry["one_page"] is True
    assert "if_released" not in entry and "effect_left_out" not in entry


def test_a_range_on_one_page_or_taller_than_a_page_is_not_tried(box, session):
    d = session.open(docx(p("Intro") + p("Line") + _fill(3) + p("Head") + grid(70) + p("End")), "none.docx")
    ids = _ids(box, session, d)
    one, tall = _range(box, session, d, f"{ids[0]}..{ids[1]}", f"{ids[5]}..{ids[7]}")
    assert one["one_page"] is True and tall["taller_than_page"] is True
    for entry in (one, tall):
        assert not {"if_kept", "if_released", "effect_left_out"} & set(entry)


def _many(count: int) -> tuple[bytes, list[int]]:
    """``count`` times :func:`_span_doc`'s page -- filler, then a span of four blocks that a
    page break parts -- each from a new page: the spans' first blocks' places."""
    body, starts, at = "", [], 0
    for k in range(count):
        first = p(f"{FILLER} start", ppr="<w:pageBreakBefore/>" if k else "")
        body += first + _fill(16)
        at += 17
        starts.append(at)
        body += p("Head") + p(WORDS) + grid(5) + p("Closing line")
        at += 4
    return docx(body + p("End")), starts


def test_each_range_is_tried_alone_and_past_the_cap_the_rest_say_how_to_ask(box, session):
    cap = TRIALS
    data, starts = _many(cap + 1)
    d = session.open(data, "many.docx")
    ids = _ids(box, session, d)
    spans = [f"{ids[k]}..{ids[k + 3]}" for k in starts]
    entries = _range(box, session, d, *spans)
    assert all("fix" in e for e in entries)
    assert ["if_kept" in e for e in entries] == [True] * cap + [False]
    assert entries[cap]["effect_left_out"].startswith(f"not laid out: a check tries at most {cap} keeps")
    assert repr(spans[cap]) in entries[cap]["effect_left_out"]
    # Each alone: the first's effect is the one a check of it alone states.
    [alone] = _range(box, session, d, spans[0])
    assert alone["if_kept"] == entries[0]["if_kept"]
    # The way out works: the last alone is tried.
    [last] = _range(box, session, d, spans[cap])
    assert "if_kept" in last


def test_the_trials_leave_the_layout_checked_as_it_was(box, session):
    """The trial is laid out from the layout held for the document: the facts of the same
    version after it, and the next version's incremental layout, are a layout's of those
    bytes alone."""
    d = session.open(_span_doc(), "held.docx")
    ids = _ids(box, session, d)
    span = f"{ids[17]}..{ids[20]}"
    before = ok(box, session, "check", doc=d, include=["layout"], pages=[1, 2]).data["layout"]
    _range(box, session, d, span)
    after = ok(box, session, "check", doc=d, include=["layout"], pages=[1, 2]).data["layout"]
    assert after == before
    ok(box, session, "word_format", doc=d, targets=[ids[0]], bold=True)
    fresh = session.open(word_docs.saved(session, d), "fresh.docx")
    laid = [pages_of(box, session, d, block) for block in _ids(box, session, d)]
    assert laid == [pages_of(box, session, fresh, block) for block in _ids(box, session, fresh)]


def test_checks_with_trials_and_edits_at_once_all_finish(box):
    """Four threads, one document: two check a split range (a trial each), two edit
    elsewhere; every call ends, none fails (round 34's rule: no wait under the lock)."""
    session = box.session()
    d = session.open(_span_doc(), "threads.docx")
    ids = _ids(box, session, d)
    span = f"{ids[17]}..{ids[20]}"
    results: list = []

    def checker():
        for _ in range(2):
            results.append(box.dispatch(session, "check", {"doc": d, "range": [span]}))

    def editor(k: int):
        for n in range(2):
            results.append(box.dispatch(session, "word_format",
                                        {"doc": d, "targets": [ids[k]], "bold": n == 0}))

    threads = [threading.Thread(target=checker), threading.Thread(target=checker),
               threading.Thread(target=editor, args=(0,)), threading.Thread(target=editor, args=(1,))]
    for thread in threads:
        thread.start()
    for thread in threads:
        thread.join(120)
    assert not any(thread.is_alive() for thread in threads)
    assert len(results) == 8 and all(r.ok for r in results), [r.error for r in results if not r.ok]
