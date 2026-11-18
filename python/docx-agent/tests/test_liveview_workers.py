"""Round 58 (Q2): the live view lays out where the document's layouts are held -- a
toolbox's worker processes -- and this process keeps none of them.

What each test holds: through a worker, ``view`` gives the pages, blocks and changed pages,
and ``page_svg`` the SVG (text, outlines and their glyph set, pictures by name or embedded)
that the same version laid out here gives -- ``render_svg`` of it, ``blocks_on`` of its
layout, docx2svg's own ``changed_pages`` -- byte for byte; nothing is laid out in this
process for it; a closed document's layouts leave the worker at once, a collected one's
when the worker next comes back.  Made-up documents only (``test_liveview.report``)."""

from __future__ import annotations

import docx2svg
import pytest

from docx_agent import Document
from docx_agent import layout as layout_module
from docx_agent.liveview import embed_images, page_svg, view

import worker_probe
from test_liveview import _body, report

# Lane L4b: what these tests read -- a live view's versions held in a worker -- is docx2svg's Python layout's, the fallback
# where the Rust layout does not lay a version out: a version it lays out is laid out and
# read in the calling process, by the document's engine (tests/test_layout_memory_l4b.py).
pytestmark = [pytest.mark.any_faces, pytest.mark.usefixtures("python_layout")]


@pytest.fixture(scope="module")
def made() -> bytes:
    return report()


def _toolbox():
    from ooxml_edit.tools import Toolbox

    from docx_agent.tools import FORMAT, GROUPS, TOOLS

    return Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS, workers=1)


def _held(box, document) -> dict:
    """What the worker holding ``document``'s layouts holds (its one worker)."""
    lineage = layout_module.lineage(document)
    return box.pool.run(worker_probe.held_lineages, timeout=120, affinity=("docx-agent layout", lineage))


def _edits():
    def text(document):
        _body(document)[4].set_text("A shorter paragraph now.")

    def longer(document):
        paragraph = _body(document)[2]
        paragraph.set_text(paragraph.text + " " + paragraph.text)

    def row(document):
        document.insert_row(document.tables()[1].id, 2)

    def style(document):
        document.styles.modify("Heading 1", size=23)

    return [text, longer, row, style]


def test_the_view_through_a_worker_is_the_versions_own(made, monkeypatch):
    laid_here: list = []
    run = docx2svg.lay_out_docx

    def counted(*args, **kwargs):
        laid_here.append(1)
        return run(*args, **kwargs)

    with _toolbox() as box:
        session = box.session()
        d1 = session.open(made, "report.docx")
        assert box.dispatch(session, "check", {"doc": d1, "include": ["reflow"]}).ok
        document = session.entry(d1).document
        monkeypatch.setattr(docx2svg, "lay_out_docx", counted)      # this process's own layouts
        shown, before, glyphs = None, None, {}
        for edit in [None, *_edits()]:
            if edit is not None:
                with session.lock([d1]) as [entry]:
                    edit(entry.document)
            data = document.to_bytes()
            now = view(session, d1, since=shown.token if shown else None)
            # The version laid out here, in a document of its own (monkeypatch undone for it).
            monkeypatch.setattr(docx2svg, "lay_out_docx", run)
            here = Document.open(data)
            layout = here.layout()
            assert [p.blocks for p in now.pages] == [tuple(layout.blocks_on(n))
                                                     for n in range(1, layout.page_count + 1)]
            if before is None:
                assert now.changed == list(range(1, now.page_count + 1))
            else:
                options = docx2svg.ConvertOptions(**layout_module.layout_options(here, {}))
                old, _, _ = docx2svg.lay_out_docx(before, options)
                new, _, _ = docx2svg.lay_out_docx(data, options, previous=old)
                assert now.changed == new.changed_pages, edit.__name__
            for number in sorted({1, now.page_count, *now.changed}):
                expected = here.render_svg([number])[0]
                embedded = page_svg(session, d1, number, now.token, images="embedded")
                assert embedded.svg == expected
                external = page_svg(session, d1, number, now.token)
                assert embed_images(external.svg, external.images) == expected
                outlined = page_svg(session, d1, number, now.token, text="outlines", glyphs=glyphs)
                alone = here.render_svg([number], text="outlines")[0]
                assert embed_images(docx2svg.embed_glyphs(outlined.svg, glyphs), outlined.images) == alone
            monkeypatch.setattr(docx2svg, "lay_out_docx", counted)
            shown, before = now, data
        assert laid_here == []                      # every layout of the view was the worker's
        assert not document._layouts
        assert not any(held.lineage == layout_module.lineage(document)
                       for held in layout_module._HELD.values())
        # The worker keeps the versions the view was given with the document's layouts.
        assert shown.token in _held(box, document)[layout_module.lineage(document)]


def test_a_page_of_a_version_the_worker_let_go_is_laid_out_again_there(made, monkeypatch):
    """The worker lost the version (its layouts dropped): it lays it out again from the
    bytes this process kept, and draws the same page."""
    with _toolbox() as box:
        session = box.session()
        d1 = session.open(made, "report.docx")
        document = session.entry(d1).document
        first = view(session, d1)
        drawn = page_svg(session, d1, 2, first.token, images="embedded").svg
        with session.lock([d1]) as [entry]:
            _body(entry.document)[0].set_text("Changed after the first view.")
        box.pool.broadcast(layout_module.drop_lineage, layout_module.lineage(document))
        assert layout_module.lineage(document) not in _held(box, document)
        assert page_svg(session, d1, 2, first.token, images="embedded").svg == drawn
        later = view(session, d1, since=first.token)
        assert later.changed and not later.stale


def test_closing_the_document_lets_the_worker_go_of_its_layouts(made):
    with _toolbox() as box:
        session = box.session()
        d1 = session.open(made, "report.docx")
        document = session.entry(d1).document
        lineage = layout_module.lineage(document)
        assert box.dispatch(session, "check", {"doc": d1, "include": ["reflow"]}).ok
        view(session, d1)
        assert lineage in _held(box, document)
        session.close(d1)
        assert lineage not in _held(box, document)        # at once: the worker was idle


def test_a_workers_drop_forgets_the_ids_read():
    """Round 59 (Q4): the memo of ids read (every document's) goes with a drop, so the
    strings it kept do not hold Python's arenas after the document is closed."""
    from docx_agent.edit.ids import valid_long_hex

    valid_long_hex("1A2B3C4D")
    assert valid_long_hex.cache_info().currsize
    layout_module.drop_lineage("no such lineage")
    assert valid_long_hex.cache_info().currsize == 0


def test_closing_does_not_wait_for_the_workers():
    """Round 59: the close tells the workers without waiting for them (ooxml-edit's
    ``broadcast(..., wait=False)``: none takes a task before it has dropped the layouts, as
    the test above holds); a collected document's note stays queued (``later``)."""
    told: list = []

    class Runner:                          # a pool of workers of its own, as seen from here
        def broadcast(self, fn, *args, later=False, wait=True):
            told.append((fn, args, later, wait))

    runner = Runner()
    document = Document.new()
    layout_module.remember_runner(document, runner)
    lineage = layout_module.lineage(document)
    layout_module.release_document(document)
    assert told == [(layout_module.drop_lineage, (lineage,), False, False)]
    layout_module._collected(lineage, document._layout_runners)
    assert told[-1] == (layout_module.drop_lineage, (lineage,), True, True)


def test_a_collected_documents_layouts_go_when_the_worker_comes_back(made):
    with _toolbox() as box:
        session = box.session()
        d1 = session.open(made, "report.docx")
        document = session.entry(d1).document
        lineage = layout_module.lineage(document)
        assert box.dispatch(session, "check", {"doc": d1, "include": ["reflow"]}).ok
        # What the document's finalizer does: no lock taken, the note queued.
        layout_module._collected(lineage, document._layout_runners)
        assert lineage in _held(box, document)              # the note runs as this task ends
        assert lineage not in _held(box, document)
