"""Round 37: an id is never given to another block.

A block's id, once it named a block in the document's session, is never generated again for
another block -- in any part, after undo and redo, after a copy or a replacement, from any
thread -- and naming it fails as *deleted*, with the change that removed it, never reaching
another block.  A document saved and opened again goes on after the ids its file still has.
All through the toolbox, on the Rust session.
"""

from __future__ import annotations

import datetime as dt
import re
import struct
import threading
import zlib

import pytest

from ooxml_edit.tools import Toolbox

from docx_agent.tools import FORMAT, GROUPS, TOOLS

import word_docs

CLOCK = lambda: dt.datetime(2026, 11, 7, 9, 0, tzinfo=dt.timezone.utc)  # noqa: E731


def png(width: int = 12, height: int = 8, color=(40, 120, 200)) -> bytes:
    def chunk(kind: bytes, data: bytes) -> bytes:
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data) & 0xFFFFFFFF)

    raw = b"".join(b"\x00" + bytes(color) * width for _ in range(height))
    return (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0))
            + chunk(b"IDAT", zlib.compress(raw)) + chunk(b"IEND", b""))


@pytest.fixture
def box():
    with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS) as toolbox:
        yield toolbox


def ok(box, session, tool: str, /, **arguments):
    result = box.dispatch(session, tool, arguments)
    assert result.ok, (tool, result.to_json())
    return result


def refused(box, session, tool: str, /, **arguments):
    result = box.dispatch(session, tool, arguments)
    assert not result.ok, (tool, result.to_json())
    return result.error


#: A document of one paragraph of text: what is appended after it is new, never the blank
#: document's empty paragraph filled.
START = word_docs.with_body('<w:p w14:paraId="1A000001" w14:textId="77777777"><w:r><w:t>Start.</w:t></w:r></w:p>')


def _opened(box, data: bytes | None = None, **options):
    session = box.session(clock=CLOCK)
    return session, session.open(data if data is not None else START, "report.docx", **options)


def _append(box, session, doc, *texts: str) -> list[str]:
    made = ok(box, session, "word_insert_markdown", doc=doc, at="end", markdown="\n\n".join(texts))
    return list(made.data["blocks"])


def every_id(session, doc) -> set[str]:
    """Every block id the document part holds now: paragraphs, tables and rows (paraIds)."""
    ids = re.findall(r'<w:(p|tr)\b[^>]*w14:paraId="([0-9A-F]{8})"', word_docs.part(word_docs.saved(session, doc)))
    return {f"p:{value}" if tag == "p" else f"t:{value}" for tag, value in ids} | \
        {f"r:{value}" for tag, value in ids if tag == "tr"}


def _values(session, doc) -> set[str]:
    """Every paraId and textId the parts hold now."""
    data = word_docs.saved(session, doc)
    out: set[str] = set()
    for name in ("word/document.xml", "word/footnotes.xml", "word/endnotes.xml", "word/comments.xml"):
        try:
            text = word_docs.part(data, name)
        except KeyError:
            continue
        out |= set(re.findall(r'w14:(?:paraId|textId)="([0-9A-F]{8})"', text))
    return out - {"77777777"}


def _texts(session, doc) -> dict[str, str]:
    return {text: block for block, text in word_docs.paragraphs(session, doc)}


# -- the example --------------------------------------------------------------------------------


def test_a_deleted_paragraph_s_id_is_not_given_to_the_next_one(box):
    session, doc = _opened(box)
    _append(box, session, doc, "a", "b")
    last = _texts(session, doc)["b"]
    deleted = ok(box, session, "word_delete", doc=doc, target=last)
    (made,) = _append(box, session, doc, "c")
    assert made != last
    error = refused(box, session, "word_set_text", doc=doc, target=last, text="x")
    assert error.code == "not_found"
    assert f"{last} was deleted" in error.message
    assert f"the change that made version {deleted.version}" in error.message and "word_read" in error.message


def test_every_form_of_a_deleted_id_says_deleted(box):
    session, doc = _opened(box)
    blocks = _append(box, session, doc, "# Heading one", "First.", "Second.", "| A | B |\n|---|---|\n| 1 | 2 |",
                     "After.")
    heading, first, second, table = blocks[:4]
    ok(box, session, "word_delete", doc=doc, target=second)
    ok(box, session, "word_delete", doc=doc, target=table)
    for address in (second, f"{second}@0:3", f"{table}/c0,0", f"{table}/r0"):
        error = refused(box, session, "word_inspect", doc=doc, target=address)
        assert "was deleted" in error.message, (address, error.message)
    assert "was deleted" in refused(box, session, "word_read", doc=doc, range=f"{first}..{second}").message
    assert "was deleted" in refused(box, session, "word_move", doc=doc, blocks=second, after=heading).message


def test_a_made_up_id_is_still_not_found_not_deleted(box):
    session, doc = _opened(box)
    (made,) = _append(box, session, doc, "a")
    ok(box, session, "word_delete", doc=doc, target=made)
    error = refused(box, session, "word_set_text", doc=doc, target="p:0BADF00D", text="x")
    assert "was deleted" not in error.message and "no paragraph or table" in error.message


def test_many_appends_after_deletions_never_take_a_deleted_id(box):
    session, doc = _opened(box)
    lines = _append(box, session, doc, *(f"Line {k}." for k in range(30)))
    gone = lines[5:25]
    for identifier in gone:
        ok(box, session, "word_delete", doc=doc, target=identifier)
    had = set(gone)
    for k in range(30):
        made = _append(box, session, doc, *(f"New {k}.{n}." for n in range(10)))
        assert not set(made) & had
        had |= set(made)
    _append(box, session, doc, "| A | B |\n|---|---|\n" + "\n".join("| x | y |" for _ in range(40)))
    assert not every_id(session, doc) & set(gone)


def test_no_value_any_state_had_is_generated_again(box):
    """Nor a textId, renewed by an edit of the text: the pool is every value, any kind."""
    session, doc = _opened(box)
    three = _append(box, session, doc, "One.", "Two.", "Three.")
    had: set[str] = set()
    for k in range(40):
        had |= _values(session, doc)
        ok(box, session, "word_set_text", doc=doc, target=three[k % 3], text=f"Text {k}.")
        (new,) = _append(box, session, doc, f"More {k}.")
        assert new.partition(":")[2] not in had
        ok(box, session, "word_delete", doc=doc, target=new)


# -- undo, redo, aliases ------------------------------------------------------------------------


def test_undo_brings_a_deleted_block_back_with_its_id_and_redo_deletes_it_again(box):
    session, doc = _opened(box)
    two = _append(box, session, doc, "One.", "Two.", "Three.")[1]
    deleted = ok(box, session, "word_delete", doc=doc, target=two)
    assert "was deleted" in refused(box, session, "word_read", doc=doc, range=two).message
    ok(box, session, "undo", doc=doc)
    assert "Two." in ok(box, session, "word_read", doc=doc, range=two).data
    ok(box, session, "undo", doc=doc, redo=True)
    error = refused(box, session, "word_read", doc=doc, range=two)
    assert f"version {deleted.version}" in error.message, error.message
    after = word_docs.saved(session, doc)
    ok(box, session, "undo", doc=doc)
    ok(box, session, "undo", doc=doc, redo=True)
    assert word_docs.saved(session, doc) == after


def test_an_undone_insertion_s_id_is_not_given_to_the_next_insertion(box):
    session, doc = _opened(box)
    (made,) = _append(box, session, doc, "First try.")
    ok(box, session, "undo", doc=doc)
    (again,) = _append(box, session, doc, "Second try.")
    assert again != made
    assert "was deleted" in refused(box, session, "word_read", doc=doc, range=made).message
    ok(box, session, "undo", doc=doc)
    ok(box, session, "undo", doc=doc, redo=True)
    assert "Second try." in ok(box, session, "word_read", doc=doc, range=again).data


def test_the_same_edits_write_the_same_bytes(box):
    def run() -> bytes:
        session, doc = _opened(box)
        blocks = _append(box, session, doc, "One.", "Two.", "Three.")
        ok(box, session, "word_delete", doc=doc, target=blocks[2])
        _append(box, session, doc, "Four.")
        ok(box, session, "undo", doc=doc)
        _append(box, session, doc, "Five.")
        return word_docs.saved(session, doc)

    assert run() == run()


# -- replacements and copies --------------------------------------------------------------------


def test_a_replaced_block_names_the_one_that_replaced_it(box):
    session, doc = _opened(box)
    two = _append(box, session, doc, "One.", "Two.", "Three.")[1]
    new = ok(box, session, "word_insert_markdown", doc=doc, at=f"replace:{two}", markdown="Two again.").data["blocks"][0]
    assert new != two
    assert f"{two} was replaced with {new}" in refused(box, session, "word_read", doc=doc, range=two).message


def test_replacing_two_copies_in_one_batch_gives_neither_an_id_either_had(box):
    session, doc = _opened(box)
    model = _append(box, session, doc, "Model.", "| Name | Value |\n|---|---|\n| a | 1 |\n| b | 2 |", "End.")[1]
    first = ok(box, session, "word_copy_from", doc=doc, source_doc=doc, range=model, at="end").data["blocks"][0]
    second = ok(box, session, "word_copy_from", doc=doc, source_doc=doc, range=model, at="end").data["blocks"][0]
    before = every_id(session, doc)
    result = ok(box, session, "batch", ops=[
        {"tool": "word_copy_from", "arguments": {"doc": doc, "source_doc": doc, "range": model,
                                                 "at": f"replace:{first}"}},
        {"tool": "word_copy_from", "arguments": {"doc": doc, "source_doc": doc, "range": model,
                                                 "at": f"replace:{second}"}}])
    made = set(result.created)
    assert made and not made & before
    for identifier in (first, second):
        assert "was" in refused(box, session, "word_inspect", doc=doc, target=identifier).message


def test_a_copy_from_another_document_does_not_keep_an_id_deleted_here(box):
    source, source_doc = _opened(box)
    _append(box, source, source_doc, "Kept.", "Other.")
    data = word_docs.saved(source, source_doc)
    session = box.session(clock=CLOCK)
    doc = session.open(data, "report.docx")
    other = session.open(data, "source.docx")
    kept = _texts(session, doc)["Kept."]
    ok(box, session, "word_delete", doc=doc, target=kept)
    copied = ok(box, session, "word_copy_from", doc=doc, source_doc=other, range=kept, at="end").data["blocks"][0]
    assert copied != kept
    assert "was deleted" in refused(box, session, "word_read", doc=doc, range=kept).message
    fresh = session.open(START, "fresh.docx")
    kept_where_never_used = ok(box, session, "word_copy_from", doc=fresh, source_doc=other, range=kept,
                               at="end").data["blocks"][0]
    assert kept_where_never_used == kept


# -- drawings -----------------------------------------------------------------------------------


def _picture(box, session, doc, anchor: str, alt: str) -> str:
    image = session.add_blob(png(), "picture.png", "image/png")
    made = ok(box, session, "word_drawings", doc=doc, action="insert_picture", at=f"{anchor}@0", image=image,
              alt_text=alt)
    return next(block for block in made.created if block.startswith("d:"))


def test_a_deleted_drawing_s_number_is_not_given_to_the_next(box):
    session, doc = _opened(box)
    (anchor,) = _append(box, session, doc, "Picture here.")
    first = _picture(box, session, doc, anchor, "One")
    ok(box, session, "word_delete", doc=doc, target=first)
    second = _picture(box, session, doc, anchor, "Two")
    assert second != first
    error = refused(box, session, "word_drawings", doc=doc, action="set", target=first, wrap="square")
    assert f"{first} was deleted" in error.message, error.message


# -- a document saved and opened again ----------------------------------------------------------


def _drawing_numbers(data: bytes) -> list[int]:
    """Every ``wp:docPr/@id`` of the main part, in order."""
    return [int(v) for v in re.findall(r'<wp:docPr id="(\d+)"', word_docs.part(data))]


def test_a_copy_into_a_reopened_document_keeps_no_drawing_number_it_retired(box):
    """P4f5: a drawing number the document retired before it was saved stays retired once
    it is opened again with its state -- a picture copied in that has that number takes
    another (the host passes the numbers it retired)."""
    session, doc = _opened(box)
    (anchor,) = _append(box, session, doc, "Pictures here.")
    _picture(box, session, doc, anchor, "Kept")
    gone = _picture(box, session, doc, anchor, "Gone")
    ok(box, session, "word_delete", doc=doc, target=gone)
    saved = session.save(doc)
    (kept,) = _drawing_numbers(saved.data)
    source, source_doc = _opened(box)
    (held,) = _append(box, source, source_doc, "A picture.")
    first = _picture(box, source, source_doc, held, "First")
    _picture(box, source, source_doc, held, "Second")
    ok(box, source, "word_delete", doc=source_doc, target=first)
    source_data = word_docs.saved(source, source_doc)
    (retired,) = _drawing_numbers(source_data)
    assert retired != kept and f"d:{retired}" == gone
    later = box.session(clock=CLOCK)
    again = later.open(saved.data, "report.docx", state=saved.state)
    other = later.open(source_data, "source.docx")
    copied = box.dispatch(later, "word_copy_from", {"doc": again, "source_doc": other, "range": held, "at": "end"})
    assert copied.ok, copied.to_json()
    numbers = _drawing_numbers(word_docs.saved(later, again))
    assert len(numbers) == 2 and retired not in numbers, numbers


def test_reopened_the_sequence_goes_on_after_the_last_id_the_file_has(box):
    session, doc = _opened(box)
    lines = _append(box, session, doc, *(f"Line {k}." for k in range(12)))
    gone = lines[3:9]
    for identifier in gone:
        ok(box, session, "word_delete", doc=doc, target=identifier)
    later, again = _opened(box, word_docs.saved(session, doc))
    made = set()
    for k in range(20):
        made |= set(_append(box, later, again, f"New {k}."))
    assert not made & set(gone)
    assert not made & (every_id(later, again) - made - {f"r:{m[2:]}" for m in made})


# -- the tools ----------------------------------------------------------------------------------


def _report(box, blocks: int = 6) -> bytes:
    session, doc = _opened(box)
    _append(box, session, doc, "# Part one", *(f"Text {k}." for k in range(blocks)), "# Part two",
            *(f"More {k}." for k in range(blocks)))
    return word_docs.saved(session, doc)


def _paragraph(session, doc, index: int) -> str:
    return [block for block, text in word_docs.paragraphs(session, doc) if text][index]


def test_a_tool_naming_a_deleted_id_says_which_call_deleted_it(box):
    session = box.session()
    doc = session.open(_report(box), "report.docx")
    one = session.view(context={"agent": "writer-1"})
    two = session.view(context={"agent": "writer-2"})
    target = _paragraph(session, doc, 3)
    deleted = box.dispatch(two, "word_delete", {"doc": doc, "target": target})
    assert deleted.ok, deleted.to_json()
    made = box.dispatch(two, "word_insert_markdown", {"doc": doc, "markdown": "Other text.", "at": "end"})
    assert target not in made.data["blocks"]
    for tool, arguments in (("word_set_text", {"doc": doc, "target": target, "text": "Mine."}),
                            ("word_read", {"doc": doc, "range": target}),
                            ("word_inspect", {"doc": doc, "target": target}),
                            ("word_format", {"doc": doc, "targets": [target], "bold": True}),
                            ("word_delete", {"doc": doc, "target": target})):
        result = box.dispatch(one, tool, arguments)
        assert not result.ok, tool
        error = result.to_json()["error"]
        assert error["code"] == "not_found", (tool, error)
        assert f"{target} was deleted by word_delete (agent=writer-2), the change that made version " \
               f"{deleted.version}" in error["message"], (tool, error)
        assert "word_read" in error["message"]


def test_check_render_and_read_batch_say_deleted(box):
    session = box.session()
    doc = session.open(_report(box), "report.docx")
    target = _paragraph(session, doc, 3)
    assert box.dispatch(session, "word_delete", {"doc": doc, "target": target}).ok
    checked = box.dispatch(session, "check", {"doc": doc, "range": [target]})
    assert not checked.ok and "was deleted by word_delete" in checked.to_json()["error"]["message"]
    rendered = box.dispatch(session, "render", {"doc": doc, "range": [target], "width": 200})
    assert not rendered.ok and "was deleted by word_delete" in rendered.to_json()["error"]["message"]
    read = box.dispatch(session, "read_batch", {"ops": [
        {"tool": "word_read", "arguments": {"doc": doc, "range": target}}]})
    assert "was deleted by word_delete" in str(read.to_json())
    batch = box.dispatch(session, "batch", {"ops": [
        {"tool": "word_set_text", "arguments": {"doc": doc, "target": target, "text": "x"}}]})
    assert "was deleted by word_delete" in str(batch.to_json())


def test_writers_on_threads_deleting_and_adding_never_share_an_id(box):
    """Several callers at once, each adding blocks and deleting its own: no id is ever handed
    out twice, and each caller's deleted ids say deleted, by it."""
    session = box.session()
    doc = session.open(_report(box), "report.docx")
    writers = 4
    rounds = 12
    handed: list[list[str]] = [[] for _ in range(writers)]
    errors: list[str] = []
    start = threading.Barrier(writers)

    def run(k: int) -> None:
        view = session.view(context={"agent": f"writer-{k}"})
        start.wait()
        try:
            for n in range(rounds):
                made = box.dispatch(view, "word_insert_markdown",
                                    {"doc": doc, "markdown": f"Writer {k} block {n}.\n\nSecond {n}.", "at": "end"})
                assert made.ok, made.to_json()
                handed[k].extend(made.data["blocks"])
                gone = made.data["blocks"][0]
                deleted = box.dispatch(view, "word_delete", {"doc": doc, "target": gone})
                assert deleted.ok, deleted.to_json()
                again = box.dispatch(view, "word_set_text", {"doc": doc, "target": gone, "text": "x"})
                assert not again.ok and f"by word_delete (agent=writer-{k})" in again.to_json()["error"]["message"]
        except Exception as error:  # noqa: BLE001 -- any error is the failure
            errors.append(repr(error))

    threads = [threading.Thread(target=run, args=(k,)) for k in range(writers)]
    for thread in threads:
        thread.start()
    for thread in threads:
        thread.join()
    assert errors == []
    every = [identifier for ids in handed for identifier in ids]
    assert len(every) == len(set(every)) == writers * rounds * 2


def _toc_lines(session, doc) -> list[str]:
    out = []
    for p in word_docs.tree(word_docs.saved(session, doc)).find("{*}body").iter("{%s}p" % word_docs.W):
        style = p.find("{*}pPr/{*}pStyle")
        if style is not None and style.get("{%s}val" % word_docs.W).lower().startswith("toc"):
            out.append(f"p:{p.get(word_docs.W14_PARA)}")
    return out


def test_a_rebuilt_table_of_contents_keeps_its_lines_ids_and_retires_the_lines_it_drops(box):
    session, doc = _opened(box)
    blocks = _append(box, session, doc, "# One", "Text.", "# Two", "Text.", "# Three", "Text.")
    ok(box, session, "word_fields", doc=doc, action="insert_toc", before=blocks[0])
    ok(box, session, "word_fields", doc=doc, action="update")
    lines = _toc_lines(session, doc)
    before = word_docs.saved(session, doc)
    ok(box, session, "word_fields", doc=doc, action="update")
    assert word_docs.saved(session, doc) == before            # nothing changed: the same bytes
    ok(box, session, "word_delete", doc=doc, target=blocks[4], force=True)
    ok(box, session, "word_fields", doc=doc, action="update")
    kept = _toc_lines(session, doc)
    assert kept == lines[:len(kept)] and len(kept) == len(lines) - 1
    (added,) = _append(box, session, doc, "# Four")
    ok(box, session, "word_fields", doc=doc, action="update")
    now = _toc_lines(session, doc)
    assert now[:-1] == kept and now[-1] not in lines and added not in lines
