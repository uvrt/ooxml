"""A document saved and opened again with its id state (round 38): what the file cannot say --
ids deleted after the last surviving one of their sequence, who deleted them -- comes back
from a short string kept beside the bytes (``Session.save``'s ``state``, ``Session.open``'s
``state``), and Word's bytes are not touched."""

from __future__ import annotations

import re
import threading

import pytest

from ooxml_edit.tools import StateRefused, Toolbox

from docx_agent.tools import FORMAT, GROUPS, TOOLS

import word_docs
from test_retired_ids import png


@pytest.fixture
def box():
    with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS) as toolbox:
        yield toolbox


def ok(box, session, tool: str, /, **arguments):
    result = box.dispatch(session, tool, arguments)
    assert result.ok, (tool, result.to_json())
    return result


def _opened(box, data: bytes, **options):
    session = box.session()
    return session, session.open(data, "report.docx", **options)


def _lines(box, session, doc, prefix: str = "Line") -> list[str]:
    return [block for block, text in word_docs.paragraphs(session, doc) if text.startswith(prefix)]


def _deleted_at_the_end(box):
    """Blocks added, the latest ones deleted: no surviving id of the sequence comes after
    them, so the file alone cannot tell they were ever there."""
    session, doc = _opened(box, word_docs.blank())
    ok(box, session, "word_insert_markdown", doc=doc, at="end",
       markdown="\n\n".join(f"Line {k}." for k in range(8)))
    gone = _lines(box, session, doc)[-4:]
    for identifier in gone:
        ok(box, session, "word_delete", doc=doc, target=identifier)
    return session, doc, gone


def _append(box, session, doc, texts: list[str]) -> list[str]:
    made = ok(box, session, "word_insert_markdown", doc=doc, at="end", markdown="\n\n".join(texts))
    return [block for block in made.data["blocks"] if block.startswith("p:")]


def _all_ids(data: bytes) -> set[str]:
    """Every paraId and textId the document part holds."""
    return set(re.findall(r'w14:(?:paraId|textId)="([0-9A-F]{8})"', word_docs.part(data))) - {"77777777"}


# -- the limit the file alone leaves ------------------------------------------------------------


def test_without_the_state_the_latest_deleted_ids_come_back_as_before(box):
    session, doc, gone = _deleted_at_the_end(box)
    later, again = _opened(box, session.save(doc).data)
    assert later.save(again).version == 0
    made = _append(box, later, again, [f"New {k}." for k in range(6)])
    assert set(made) & set(gone)            # the stated limit, unchanged without a state


def test_with_the_state_no_deleted_id_is_given_again_and_naming_one_says_deleted(box):
    session, doc, gone = _deleted_at_the_end(box)
    saved = session.save(doc)
    version = session.save(doc).version
    later, again = _opened(box, saved.data, state=saved.state)
    made = _append(box, later, again, [f"New {k}." for k in range(30)])
    assert not set(made) & set(gone)
    assert len(set(made)) == len(made)
    for identifier in gone:
        refused = box.dispatch(later, "word_set_text", {"doc": again, "target": identifier, "text": "x"})
        assert not refused.ok and refused.error.code == "not_found"
        found = re.search(rf"{identifier} was deleted .*the change that made version (\d+)", refused.error.message)
        assert found and int(found.group(1)) <= version, refused.error.message
        assert "before the document was saved and opened again" in refused.error.message


def test_the_bytes_are_the_same_with_or_without_the_state(box):
    session, doc, _ = _deleted_at_the_end(box)
    saved = session.save(doc)
    assert session.save(doc).data == saved.data
    with_state, d1 = _opened(box, saved.data, state=saved.state)
    without, d2 = _opened(box, saved.data)
    assert with_state.save(d1).data == without.save(d2).data


def test_no_value_the_document_ever_had_is_generated_after_the_reopen(box):
    session, doc = _opened(box, word_docs.blank())
    ok(box, session, "word_insert_markdown", doc=doc, at="end",
       markdown="\n\n".join(f"Line {k}." for k in range(10)))
    pool = _all_ids(session.save(doc).data)
    for identifier, text in word_docs.paragraphs(session, doc)[:5]:
        ok(box, session, "word_set_text", doc=doc, target=identifier, text=text + " again")   # textIds renewed
        pool |= _all_ids(session.save(doc).data)
    for identifier in _lines(box, session, doc)[-3:]:
        ok(box, session, "word_delete", doc=doc, target=identifier)
    saved = session.save(doc)
    later, again = _opened(box, saved.data, state=saved.state)
    before = _all_ids(saved.data)
    _append(box, later, again, [f"More {k}." for k in range(40)])
    new = _all_ids(later.save(again).data) - before
    assert new and not new & pool


def test_versions_go_on_from_the_saved_one(box):
    session, doc, _ = _deleted_at_the_end(box)
    last = session.save(doc).version            # the highest version given: the head's
    ok(box, session, "undo", doc=doc)
    ok(box, session, "undo", doc=doc, redo=True)
    saved = session.save(doc)
    saved_version = saved.version
    later, again = _opened(box, saved.data, state=saved.state)
    assert later.save(again).version == saved_version
    _append(box, later, again, ["Next."])
    assert later.save(again).version == last + 1
    undone = box.dispatch(later, "undo", {"doc": again})
    assert not undone.ok or later.save(again).version == saved_version


def test_a_state_survives_several_reopens_and_this_session_s_deletions_join_it(box):
    session, doc, gone = _deleted_at_the_end(box)
    saved = session.save(doc)
    second, d2 = _opened(box, saved.data, state=saved.state)
    survivor = _lines(box, second, d2)[-1]
    ok(box, second, "word_delete", doc=d2, target=survivor)
    saved = second.save(d2)
    third, d3 = _opened(box, saved.data, state=saved.state)
    made = _append(box, third, d3, [f"New {k}." for k in range(20)])
    assert not set(made) & {*gone, survivor}
    for identifier in (*gone, survivor):
        refused = box.dispatch(third, "word_set_text", {"doc": d3, "target": identifier, "text": "x"})
        assert "before the document was saved and opened again" in refused.error.message
    # Opened without a change, the state it gives is the one it took.
    last = third.save(d3)
    fourth, d4 = _opened(box, last.data, state=last.state)
    assert fourth.save(d4).state == last.state


def test_a_replaced_block_still_names_its_replacement(box):
    session, doc = _opened(box, word_docs.blank())
    ok(box, session, "word_insert_markdown", doc=doc, at="end", markdown="One.\n\nTwo.")
    old = _lines(box, session, doc, "Two")[-1]
    new = ok(box, session, "word_insert_markdown", doc=doc, at=f"replace:{old}", markdown="Two, again.").data["blocks"][0]
    saved = session.save(doc)
    later, again = _opened(box, saved.data, state=saved.state)
    refused = box.dispatch(later, "word_set_text", {"doc": again, "target": old, "text": "x"})
    assert f"{old} was replaced with {new}" in refused.error.message, refused.error.message


def test_a_deleted_drawing_number_is_not_given_again_after_the_reopen(box):
    session, doc = _opened(box, word_docs.blank())
    anchor = _append(box, session, doc, ["Picture here."])[0]
    image = session.add_blob(png(), "picture.png", "image/png")
    first = ok(box, session, "word_drawings", doc=doc, action="insert_picture", at=f"{anchor}@0", image=image,
               alt_text="One").created[0]
    ok(box, session, "word_delete", doc=doc, target=first)
    saved = session.save(doc)
    later, again = _opened(box, saved.data, state=saved.state)
    image = later.add_blob(png(), "picture.png", "image/png")
    second = ok(box, later, "word_drawings", doc=again, action="insert_picture", at=f"{anchor}@0", image=image,
                alt_text="Two").created[0]
    assert second != first
    refused = box.dispatch(later, "word_drawings", {"doc": again, "action": "set", "target": first, "wrap": "square"})
    assert not refused.ok and f"{first} was deleted" in refused.error.message, refused.to_json()


# -- refusals ------------------------------------------------------------------------------------


#: What a name by place is refused with: the durable forms, and where to read them (decision 4).
POSITIONAL = ("names a block by its place", "word_inspect or word_read")


def test_a_positional_id_held_from_before_stamp_on_open_is_refused_with_guidance(box):
    # A file whose paragraphs have no paraId: every block has a durable id from the first read,
    # and a name by place (what this library gave such a file before) is refused, saying what to do.
    data = word_docs.with_body(word_docs.paragraph("One.") + word_docs.paragraph("Two.")
                               + word_docs.paragraph("Three."))
    session, doc = _opened(box, data)
    listed = [block for block, _ in word_docs.blocks(box, session, doc)]
    assert all(block.startswith("p:") for block in listed), listed
    for tool, arguments in [("word_read", {"range": "p@body/2"}),
                            ("word_set_text", {"target": "p@body/2", "text": "Changed."}),
                            ("word_read", {"range": "s@body/2"})]:
        result = box.dispatch(session, tool, {"doc": doc, **arguments})
        assert not result.ok, result.to_json()
        assert result.error.code == "invalid_arguments", result.to_json()
        assert all(said in result.error.message for said in POSITIONAL), result.to_json()
    assert "Three." in ok(box, session, "word_read", doc=doc, range=listed[2]).data



def test_a_state_of_other_bytes_is_refused_with_the_way_out(box):
    session, doc, _ = _deleted_at_the_end(box)
    state = session.save(doc).state
    _append(box, session, doc, ["Later."])
    with pytest.raises(StateRefused) as caught:
        _opened(box, session.save(doc).data, state=state)
    message = str(caught.value)
    assert "of other bytes" in message and "without ids=" in message and "Session.save" in message


@pytest.mark.parametrize("state", ["nonsense", "docx-ids.1.AAAA", "docx-ids.9.xyz", 42])
def test_a_state_that_is_none_is_refused_with_the_way_out(box, state):
    session, doc, _ = _deleted_at_the_end(box)
    with pytest.raises(StateRefused, match="without ids="):
        _opened(box, session.save(doc).data, state=state)


def test_an_empty_state_is_no_state(box):
    session, doc, _ = _deleted_at_the_end(box)
    later, again = _opened(box, session.save(doc).data, state="")
    assert later.save(again).version == 0


# -- size ---------------------------------------------------------------------------------------


def _batch(box, session, doc, ops):
    for start in range(0, len(ops), 150):           # a batch takes at most 200 ops
        result = box.dispatch(session, "batch", {"ops": [{"tool": tool, "arguments": {"doc": doc, **arguments}}
                                                         for tool, arguments in ops[start:start + 150]]})
        assert result.ok, result.to_json()


def test_the_state_of_a_long_day_s_edits_stays_small(box):
    """A 300-paragraph report rewritten all day: 1,500 text edits, 600 blocks deleted, 600
    added.  The state stays a few kilobytes."""
    session, doc = _opened(box, word_docs.blank())
    _append(box, session, doc, [f"Paragraph {k} of the report." for k in range(300)])
    for round_ in range(5):
        _batch(box, session, doc, [("word_set_text", {"target": block, "text": f"Paragraph, pass {round_}."})
                                   for block, _ in word_docs.paragraphs(session, doc)[:300]])
    for round_ in range(6):
        _batch(box, session, doc, [("word_delete", {"target": block})
                                   for block, _ in word_docs.paragraphs(session, doc)[-100:]])
        _append(box, session, doc, [f"Added {round_}.{k}." for k in range(100)])
    saved = session.save(doc)
    assert len(saved.state) < 8000, len(saved.state)
    later, again = _opened(box, saved.data, state=saved.state)
    assert later.save(again).state


# -- the tools ----------------------------------------------------------------------------------


def _report(box) -> bytes:
    session, doc = _opened(box, word_docs.blank())
    ok(box, session, "word_insert_markdown", doc=doc, at="end",
       markdown="# Part one\n\n" + "\n\n".join(f"Text {k}." for k in range(6)))
    return session.save(doc).data


def _last_text(session, doc) -> str:
    return [block for block, text in word_docs.paragraphs(session, doc) if text][-1]


def test_tools_save_and_open_under_the_same_doc_id_keep_who_deleted(box):
    session = box.session()
    doc = session.open(_report(box), "report.docx")
    writer = session.view(context={"agent": "writer-2"})
    target = _last_text(session, doc)
    deleted = box.dispatch(writer, "word_delete", {"doc": doc, "target": target})
    assert deleted.ok, deleted.to_json()
    saved = session.save(doc)
    assert saved.doc == doc and saved.state and saved.version == deleted.version
    session.close(doc)

    later = box.session()
    again = later.open(saved.data, saved.name, doc_id=saved.doc, state=saved.state)
    assert again == doc
    made = box.dispatch(later, "word_insert_markdown", {"doc": doc, "markdown": "New text.", "at": "end"})
    assert made.ok and target not in made.data["blocks"]
    assert made.version == deleted.version + 1
    result = box.dispatch(later.view(context={"agent": "writer-1"}), "word_set_text",
                          {"doc": doc, "target": target, "text": "Mine."})
    error = result.to_json()["error"]
    assert error["code"] == "not_found"
    assert (f"{target} was deleted by word_delete (agent=writer-2), the change that made version "
            f"{deleted.version}, before the document was saved and opened again") in error["message"]


def test_the_save_tool_s_output_carries_the_state(box):
    session = box.session()
    doc = session.open(_report(box), "report.docx")
    target = _last_text(session, doc)
    assert box.dispatch(session, "word_delete", {"doc": doc, "target": target}).ok
    saved = box.dispatch(session, "save_document", {"doc": doc, "name": "out.docx", "format": "docx"})
    assert saved.ok, saved.to_json()
    assert "state" not in str(saved.to_json())
    output = session.take_outputs()[-1]
    assert output.state
    later = box.session()
    reopened = later.open(output.data, output.name, doc_id=doc, state=output.state)
    blocks = box.dispatch(later, "word_insert_markdown", {"doc": reopened, "markdown": "Again.", "at": "end"})
    assert target not in blocks.data["blocks"]
    markdown = box.dispatch(session, "save_document", {"doc": doc, "name": "out.md", "format": "markdown"})
    assert markdown.ok and session.take_outputs()[-1].state is None


def test_tools_refuse_a_state_of_other_bytes_and_open_without_one_as_before(box):
    session = box.session()
    doc = session.open(_report(box), "report.docx")
    saved = session.save(doc)
    assert box.dispatch(session, "word_insert_markdown", {"doc": doc, "markdown": "More.", "at": "end"}).ok
    newer = session.save(doc)
    later = box.session()
    with pytest.raises(StateRefused, match="of other bytes"):
        later.open(newer.data, "report.docx", doc_id=doc, state=saved.state)
    assert later.open(newer.data, "report.docx", doc_id=doc) == doc
    later.close(doc)
    assert later.open(newer.data, "report.docx", doc_id=doc, state="") == doc


def test_writers_on_threads_after_a_reopen_never_take_a_retired_id(box):
    session = box.session()
    doc = session.open(_report(box), "report.docx")
    handed: list[str] = []
    for k in range(6):
        made = box.dispatch(session, "word_insert_markdown", {"doc": doc, "markdown": f"Early {k}.", "at": "end"})
        handed.extend(made.data["blocks"])
        assert box.dispatch(session, "word_delete", {"doc": doc, "target": made.data["blocks"][0]}).ok
    saved = session.save(doc)
    later = box.session()
    later.open(saved.data, saved.name, doc_id=doc, state=saved.state)
    writers, rounds = 4, 8
    found: list[list[str]] = [[] for _ in range(writers)]
    errors: list[str] = []
    start = threading.Barrier(writers)

    def run(k: int) -> None:
        view = later.view(context={"agent": f"writer-{k}"})
        start.wait()
        try:
            for n in range(rounds):
                made = box.dispatch(view, "word_insert_markdown",
                                    {"doc": doc, "markdown": f"Writer {k} block {n}.", "at": "end"})
                assert made.ok, made.to_json()
                found[k].extend(made.data["blocks"])
                assert box.dispatch(view, "word_delete", {"doc": doc, "target": made.data["blocks"][0]}).ok
        except Exception as error:  # noqa: BLE001 -- any error is the failure
            errors.append(repr(error))

    threads = [threading.Thread(target=run, args=(k,)) for k in range(writers)]
    for thread in threads:
        thread.start()
    for thread in threads:
        thread.join()
    assert errors == []
    every = [identifier for ids in found for identifier in ids]
    assert len(every) == len(set(every)) == writers * rounds
    assert not set(every) & set(handed)
