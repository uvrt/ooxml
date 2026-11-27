"""``render`` on the Rust session: images kept by content, so a page an edit left as it was is
not drawn again (round 10, O1 and O3); refusals said before anything is drawn; a sheet's tiles
reused; and render and check in a daemonic process (a Celery prefork worker's child).  P4e,
lane CUT-D: from ``test_parallel_render.py`` and ``test_subprocess_render.py``, whose render
workers and Python renderer are gone; ``test_rust_render.py`` holds the rest of render's
replies.  Made-up documents only."""

from __future__ import annotations

import multiprocessing
import os



from ooxml_edit.tools import Limits, Toolbox  # noqa: E402

from docx_agent.tools import FORMAT, GROUPS, TOOLS  # noqa: E402
from pixels import pixels  # noqa: E402
from test_rust_render import _pages  # noqa: E402
from word_docs import blank, paragraph, with_body  # noqa: E402


def _box(**options) -> Toolbox:
    return Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS, font_dirs=[], workers=0,
                   limits=options.pop("limits", Limits(image_budget=None)), **options)


def _ok(box, session, tool, **arguments):
    result = box.dispatch(session, tool, arguments)
    assert result.ok, result.error and result.error.to_json()
    return result


def test_a_repeat_is_answered_from_the_store_and_an_edit_redraws_only_its_page():
    with _box() as box:
        session = box.session()
        d = session.open(_pages(4), "note.docx")
        first = _ok(box, session, "render", doc=d, pages=[1, 2, 3, 4], width=300)
        again = _ok(box, session, "render", doc=d, pages=[1, 2, 3, 4], width=300)
        assert again.data["cached"] == [1, 2, 3, 4]
        _ok(box, session, "replace_text", doc=d, find="Page 3 of", replace="Page three of", expect="one")
        after = _ok(box, session, "render", doc=d, pages=[1, 2, 3, 4], width=300)
    assert after.data["cached"] == [1, 2, 4]
    assert after.images[2].data != first.images[2].data
    assert [after.images[k].data for k in (0, 1, 3)] == [first.images[k].data for k in (0, 1, 3)]


def test_a_page_past_the_end_is_not_found_and_a_sheet_reuses_the_tiles():
    with _box() as box:
        session = box.session()
        d = session.open(_pages(3), "note.docx")
        missing = box.dispatch(session, "render", {"doc": d, "pages": [2, 9]})
        assert not missing.ok and missing.error.code == "not_found"
        _ok(box, session, "render", doc=d, pages=[1, 2], width=200)
        sheet = _ok(box, session, "render", doc=d, pages=[1, 2, 3], sheet={"columns": 3, "width": 200})
    assert sheet.data["cached"] is False and sheet.data["cached_tiles"] == [1, 2]


def test_refusals_cost_no_render():
    with _box(limits=Limits(image_budget=1)) as box:
        session = box.session()
        d = session.open(_pages(3), "note.docx")
        over = box.dispatch(session, "render", {"doc": d, "pages": [1, 2]})
        assert not over.ok and over.error.code == "limit"
        # Thirty named pages in one column: too tall at any page this document's sections
        # declare, even at the narrowest tile -- said before anything is laid out.
        tall = box.dispatch(session, "render", {"doc": d, "pages": list(range(1, 31)),
                                                "sheet": {"columns": 1, "width": 300}})
        assert not tall.ok and tall.error.code == "limit" and "pages" in tall.error.message
        # Nothing was drawn: the next render of page 1 draws it afresh.
        first = _ok(box, session, "render", doc=d, pages=[1], width=300)
    assert first.data["cached"] == []


def work_in_a_daemonic_process(queue, data: bytes) -> None:
    try:
        with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS, font_dirs=[]) as box:
            session = box.session()
            session.open(data, "report.docx")
            render = box.dispatch(session, "render", {"doc": "d1", "pages": [1], "width": 300})
            check = box.dispatch(session, "check", {"doc": "d1", "include": ["reflow"]})
            queue.put({"ok": render.ok and check.ok, "pid": os.getpid(),
                       "image": render.images[0].data if render.ok else None,
                       "pages": check.data["reflow"]["pages"] if check.ok else None,
                       "error": [r.error.message for r in (render, check) if r.error]})
    except BaseException as exc:  # noqa: BLE001 -- reported to the test
        queue.put({"ok": False, "error": f"{type(exc).__name__}: {exc}"})


def test_a_daemonic_process_renders_and_checks():
    """A daemonic process may not start workers (multiprocessing refuses): render and check
    are the Rust session's, in the process itself, and answer as here."""
    data = with_body(paragraph("Laid out and rendered elsewhere."), blank())
    context = multiprocessing.get_context("spawn")
    queue = context.Queue()
    process = context.Process(target=work_in_a_daemonic_process, args=(queue, data), daemon=True)
    process.start()
    try:
        got = queue.get(timeout=240)
    finally:
        process.join(30)
    assert got["ok"], got["error"]
    assert got["pid"] != os.getpid() and got["pages"] == 1
    with _box() as box:
        session = box.session()
        d = session.open(data, "report.docx")
        here = _ok(box, session, "render", doc=d, pages=[1], width=300)
    assert pixels(got["image"]) == pixels(here.images[0].data)
