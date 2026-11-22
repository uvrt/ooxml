"""Round 35, R27, on the Rust session (lane LV3): an application's live view
(:mod:`docx_agent.liveview`).

What each test holds: ``view`` lists every page with its blocks and the pages whose drawing
changed since a token; ``page_svg`` draws one page of the token's version, tagged with its
ids (the footer's from its part), pictures by name or embedded, even after the document
moved on; tokens of another session or none are stale or unknown; the refusals give the way
out; and views read while another thread edits.  Made-up documents only."""

from __future__ import annotations

import struct
import threading
import zlib

import pytest

from docx_agent import Document
from docx_agent.liveview import UnknownToken, embed_images, page_svg, view

from ooxml_edit.tools import ToolError

pytestmark = pytest.mark.any_faces

#: How long the calls of the threaded test may take in all before it calls them deadlocked.
DEADLINE = 90.0

WORDS = ("lantern meadow river stone harbour quiet orchard copper valley signal morning ledger window "
         "garden timber bridge market evening canvas thread pebble summit").split()


def _text(seed: int, sentences: int) -> str:
    out = []
    for s in range(sentences):
        out.append(" ".join(WORDS[(seed * 7 + s * 5 + k * 3) % len(WORDS)] for k in range(12)).capitalize() + ".")
    return " ".join(out)


def _png(width: int, height: int, color=(40, 120, 200)) -> bytes:
    def chunk(kind: bytes, data: bytes) -> bytes:
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data) & 0xFFFFFFFF)

    raw = b"".join(b"\x00" + bytes(color) * width for _ in range(height))
    return (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0))
            + chunk(b"IDAT", zlib.compress(raw)) + chunk(b"IEND", b""))


def report(chapters: int = 3) -> bytes:
    """A made-up report of some six pages: chapters on new pages, a table each, a picture,
    a footer with the page number."""
    doc = Document.new()
    parts = ["# Report title", _text(0, 2)]
    for c in range(1, chapters + 1):
        parts.append(f"# Chapter {c}")
        for p in range(3):
            parts.append(_text(c * 10 + p, 6))
        parts.append("| Item | Owner | Amount |\n|---|---|---|\n"
                     + "\n".join(f"| Item {c}.{k} | {WORDS[k % len(WORDS)]} | {k * 7} |" for k in range(8)))
    doc.insert_markdown("\n\n".join(parts))
    for paragraph in doc.paragraphs():
        if paragraph.text.startswith("Chapter "):
            paragraph.format(page_break_before=True)
    doc.insert_picture(f"{doc.paragraphs()[1].id}@0", _png(40, 30), width=60)
    section = doc.sections()[0]
    doc.add_footer(section.id, "default", "Page ")
    doc.insert_field(f"{section.footer().paragraphs[0].id}@5", "PAGE", result="1")
    return doc.to_bytes()


@pytest.fixture(scope="module")
def made() -> bytes:
    return report()


def _toolbox():
    from ooxml_edit.tools import Toolbox

    from docx_agent.tools import FORMAT, GROUPS, TOOLS

    return Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS, workers=0)


@pytest.fixture
def opened(made):
    """``(toolbox, session, doc)``: the report open in a session."""
    with _toolbox() as box:
        session = box.session()
        yield box, session, session.open(made, "report.docx")


def _set_text(box, session, doc, target: str, text: str) -> None:
    result = box.dispatch(session, "word_set_text", {"doc": doc, "target": target, "text": text})
    assert result.ok, result.error and result.error.to_json()


def _paragraphs(shown) -> list[str]:
    """The body paragraphs' ids, in order, as the view lists them (each once)."""
    seen: list[str] = []
    for page in shown.pages:
        seen.extend(b for b in page.blocks if b.startswith("p:") and b not in seen)
    return seen


# -- view --------------------------------------------------------------------------------------


def test_the_first_view_lists_every_page_with_its_blocks(opened):
    _, session, d1 = opened
    shown = view(session, d1)
    assert shown.page_count > 3
    assert shown.changed == list(range(1, shown.page_count + 1))
    assert [p.number for p in shown.pages] == shown.changed
    assert all(p.blocks for p in shown.pages)
    assert any(b.startswith("t:") for p in shown.pages for b in p.blocks), "the tables too"
    assert all(p.width > 500 and p.height > 700 for p in shown.pages)
    assert shown.version == session.entry(d1).version
    again = view(session, d1, since=shown.token)
    assert again.token == shown.token and again.changed == [] and not again.stale


def test_an_edit_changes_the_pages_it_draws_on(opened):
    box, session, d1 = opened
    shown = view(session, d1)
    target = _paragraphs(shown)[-1]
    _set_text(box, session, d1, target, "Changed on the last chapter's page.")
    after = view(session, d1, since=shown.token)
    on = [p.number for p in after.pages if target in p.blocks]
    assert after.token != shown.token and on and set(on) <= set(after.changed)
    assert 1 not in after.changed, "the title page draws nothing that changed"


def test_a_stale_token_says_so_and_lists_every_page(opened, made):
    _, session, d1 = opened
    shown = view(session, d1, since="0000-not-a-token")
    assert shown.stale and "every page" in shown.note
    assert shown.changed == list(range(1, shown.page_count + 1))
    with _toolbox() as other_box:
        other = other_box.session()
        theirs = view(other, other.open(made, "report.docx"))
    assert view(session, d1, since=theirs.token).stale            # another session's


# -- page_svg ----------------------------------------------------------------------------------


def test_a_page_is_tagged_with_its_versions_ids(opened):
    _, session, d1 = opened
    shown = view(session, d1)
    for page in shown.pages:
        drawn = page_svg(session, d1, page.number, shown.token, images="embedded")
        assert drawn.svg.startswith("<svg") and drawn.images == {}
        for block in page.blocks:
            assert f'data-docx-agent-id="{block}' in drawn.svg or block.startswith("t:")
    first = page_svg(session, d1, 1, shown.token).svg
    assert 'data-docx-agent-id="footer1/p:' in first, "the footer's paragraph tagged with its part"


def test_pictures_come_back_by_name(opened):
    _, session, d1 = opened
    shown = view(session, d1)
    page = 1                                    # the picture is in the title page's text
    drawn = page_svg(session, d1, page, shown.token)
    [(name, data)] = drawn.images.items()
    assert name.startswith("img/") and name.endswith(".png") and len(name) == len("img/") + 32 + 4
    assert data.startswith(b"\x89PNG") and "base64," not in drawn.svg
    assert embed_images(drawn.svg, drawn.images) == page_svg(session, d1, page, shown.token, images="embedded").svg
    with pytest.raises(KeyError, match="no such entry"):
        embed_images(drawn.svg, {})


def test_an_older_tokens_page_is_that_versions(opened):
    """The document moved on: the page drawn is the token's version, with its ids."""
    box, session, d1 = opened
    shown = view(session, d1)
    target = _paragraphs(shown)[1]
    page = next(p.number for p in shown.pages if target in p.blocks)
    before = page_svg(session, d1, page, shown.token, images="embedded").svg
    _set_text(box, session, d1, target, "Rewritten after the view.")
    now = view(session, d1, since=shown.token)
    assert page in now.changed
    assert page_svg(session, d1, page, shown.token, images="embedded").svg == before
    assert "Rewritten" in page_svg(session, d1, page, now.token, images="embedded").svg


def test_refusals_say_the_way_out(opened, made):
    _, session, d1 = opened
    shown = view(session, d1)
    with pytest.raises(IndexError, match=f"1 to {shown.page_count}"):
        page_svg(session, d1, shown.page_count + 1, shown.token)
    with pytest.raises(ValueError, match="text, outlines"):
        page_svg(session, d1, 1, shown.token, text="paths")
    with pytest.raises(ValueError, match="external, embedded"):
        page_svg(session, d1, 1, shown.token, images="inline")
    with pytest.raises(ToolError, match='text="text"'):
        page_svg(session, d1, 1, shown.token, text="outlines")
    with pytest.raises(UnknownToken, match="call view"):
        page_svg(session, d1, 1, "nothing")
    with pytest.raises(TypeError, match=r"\(session, doc, page, token\)"):
        page_svg(session, d1, 1)
    with pytest.raises(TypeError, match="open it in one"):
        view(made)                              # a document's bytes, outside a session


# -- views beside edits ------------------------------------------------------------------------


def test_views_are_read_while_another_thread_edits(opened):
    """An edit thread and a view thread at once: every view names a version whose pages it
    draws, and the last view shows the last edit."""
    box, session, d1 = opened
    body = _paragraphs(view(session, d1))[1:]
    errors: list = []

    def editing():
        for k in range(4):
            _set_text(box, session, d1, body[k % len(body)], f"Round {k}.")

    def viewing():
        token = None
        for _ in range(4):
            shown = view(session, d1, since=token)
            for number in shown.changed[:2]:
                assert page_svg(session, d1, number, shown.token).svg.startswith("<svg")
            token = shown.token

    def run(job):
        try:
            job()
        except BaseException as error:  # noqa: BLE001 -- reported below
            errors.append(error)

    threads = [threading.Thread(target=run, args=(job,), daemon=True) for job in (editing, viewing)]
    for thread in threads:
        thread.start()
    for thread in threads:
        thread.join(DEADLINE)
    assert not any(t.is_alive() for t in threads), "deadlock"
    assert not errors, errors
    last = view(session, d1)
    page = next(p.number for p in last.pages if body[3] in p.blocks)
    assert "Round 3." in page_svg(session, d1, page, last.token).svg
