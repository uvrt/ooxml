"""Edits of one document side by side, on the Rust session (round 45, lane B; rounds 47-50 and
lane J: P4e, lane CUT-D, from ``test_block_edits.py``, whose drafts are gone).

Calls from several callers at once are each one version and one undo step of their caller.
Two calls started together end as the same two run one at a time in the order they committed:
the same results, the same package, part by part.  That holds for inserts into one gap, links
to one address or two, bookmarks (one name taken twice is refused as it would be one at a
time), tracked edits (their revision ids each the document's own, accepted and rejected by id),
and a table's width of the body beside an insert before it.  A batch's read sees its own
change; a retry key answers "already done"; a cell's empty ``from`` is no source.

Made-up documents and values only.
"""

from __future__ import annotations

import copy
import datetime as dt
import functools
import threading

import pytest
from ooxml_edit.tools import Dataset, Result, Toolbox, string, tool

from docx_agent.tools import FORMAT, GROUPS, TOOLS
from word_docs import blank, blocks, call, saved, table_texts, text_of, tree, validation, element

pytestmark = pytest.mark.any_faces

CLOCK = lambda: dt.datetime(2026, 11, 1, 9, 0, tzinfo=dt.timezone.utc)  # noqa: E731
WORDS = "lantern meadow river stone harbour quiet orchard copper valley signal".split()
_W = "{http://schemas.openxmlformats.org/wordprocessingml/2006/main}"


@tool("ds_rows", "Test only: keep a made-up dataset of rows with a name.",
      {"id": string("Its id."), "rows": string("How many rows.")}, documents=())
def ds_rows(call, id, rows):
    names = ("alpha", "bravo", "charlie", "delta", "echo", "foxtrot", "golf", "hotel")
    call.define_dataset(id, Dataset(("name", "count"), [(names[k % len(names)], k) for k in range(int(rows))]),
                        source=None)
    return Result(summary=f"Kept d:{id}")


def _box() -> Toolbox:
    return Toolbox([*TOOLS, ds_rows], formats=[FORMAT], groups=GROUPS, workers=0)


def _text(seed: int, words: int) -> str:
    return " ".join(WORDS[(seed + k) % len(WORDS)] for k in range(words)).capitalize() + "."


@functools.lru_cache(maxsize=1)
def report() -> bytes:
    """Three chapters -- a heading, four paragraphs, a list, a table, an end paragraph --
    then a table to fill."""
    parts = []
    for c in range(1, 4):
        parts += [f"# Chapter {c}", *(_text(c + p, 40) for p in range(4)), "- item one\n- item two",
                  "| Name | Value |\n|---|---|\n| a | 1 |\n| b | 2 |", f"End of chapter {c}."]
    parts += ["# To fill", "| name | count |\n|---|---|\n|  |  |"]
    with _box() as box:
        session = box.session(clock=CLOCK)
        doc = session.open(blank(), "report.docx")
        call(box, session, "word_insert_markdown", doc=doc, at="end", markdown="\n\n".join(parts))
        return saved(session, doc)


class Layout:
    """The report's ids: each chapter's heading, paragraphs, list items, table and end; the
    table to fill."""

    def __init__(self, box, session, doc) -> None:
        self.heads: list[str] = []
        self.bodies: list[list[str]] = []
        self.items: list[list[str]] = []
        self.tables: list[str] = []
        self.ends: list[str] = []
        self.fill = ""
        filling = False
        for block, text in blocks(box, session, doc):
            text = text.split("\n")[0]
            if text.startswith("# Chapter "):
                self.heads.append(block)
                self.bodies.append([])
                self.items.append([])
            elif text.startswith("# To fill"):
                filling = True
            elif block.startswith("t:"):
                if filling:
                    self.fill = block
                else:
                    self.tables.append(block)
            elif text.startswith("End of chapter"):
                self.ends.append(block)
            elif text.startswith("- ") and self.items:      # a list item is a block of its own
                self.items[-1].append(block)
            elif self.bodies and text:
                self.bodies[-1].append(block)


def _opened(box, tracked: str | None = None):
    session = box.session(clock=CLOCK)
    doc = session.open(report(), "report.docx")
    assert box.dispatch(session, "ds_rows", {"id": "rows", "rows": "6"}).ok
    layout = Layout(box, session, doc)
    if tracked:
        call(box, session, "word_set_tracking", doc=doc, on=True, author=tracked)
        call(box, session, "word_set_text", doc=doc, target=layout.bodies[1][1], text="First tracked.")
    return session, doc, layout


def normal(result) -> dict:
    """What a result says that is not about time."""
    out = copy.deepcopy(result.to_json())
    data = out.get("data")
    if isinstance(data, dict):
        data.pop("timing", None)
        if isinstance(data.get("reflow"), dict):
            data["reflow"].pop("since_last_check", None)
        for op in data.get("ops") or []:
            if isinstance(op.get("data"), dict):
                op["data"].pop("timing", None)
    if isinstance(out.get("checks"), dict):
        out["checks"].pop("reflow", None)
    out["warnings"] = [w for w in out.get("warnings") or [] if "changed while this call read it" not in w]
    out.pop("version", None)
    return out


def parts(data: bytes) -> dict[str, bytes]:
    import io
    import zipfile

    with zipfile.ZipFile(io.BytesIO(data)) as package:
        return {name: package.read(name) for name in sorted(package.namelist())}


def _side_by_side(first, then, *, tracked: str | None = None):
    """``first`` and ``then`` (each a function of the doc and layout giving (name, arguments))
    started together; then the two one at a time, in the order they committed, in a session
    of their own.  Both results (``first``'s, ``then``'s), the final bytes, and whether the
    results and the package are the same both ways."""
    with _box() as box:
        session, doc, layout = _opened(box, tracked)
        calls = {"first": first(doc, layout), "then": then(doc, layout)}
        found: dict = {}
        go = threading.Barrier(2)

        def run(key):
            go.wait(60)
            found[key] = box.dispatch(session, *calls[key])

        threads = [threading.Thread(target=run, args=(key,)) for key in calls]
        for thread in threads:
            thread.start()
        for thread in threads:
            thread.join(120)
        assert set(found) == {"first", "then"}
        concurrent = saved(session, doc)
    order = sorted(found, key=lambda key: (found[key].version is None, found[key].version or 0))
    with _box() as box:
        session, doc, layout = _opened(box, tracked)
        alone = {key: box.dispatch(session, *calls[key]) for key in order}
        serial = saved(session, doc)
    same = ({k: normal(r) for k, r in found.items()} == {k: normal(r) for k, r in alone.items()}
            and parts(concurrent) == parts(serial))
    return found["first"], found["then"], concurrent, same


def _annotation_ids(data: bytes) -> list[str]:
    root = tree(data)
    tags = {_W + t for t in ("ins", "del", "rPrChange", "pPrChange", "tblPrChange", "trPrChange", "tcPrChange",
                             "moveFrom", "moveTo", "bookmarkStart", "commentRangeStart")}
    return [node.get(_W + "id") for node in root.iter() if node.tag in tags]


# -- one caller --------------------------------------------------------------------------------


def test_one_caller_alone_answers_with_no_block():
    with _box() as box:
        session, doc, layout = _opened(box)
        result = call(box, session, "word_set_text", doc=doc, target=layout.bodies[0][0], text="Alone.")
        assert result.block is None


def test_a_fill_from_a_dataset_grows_the_table():
    with _box() as box:
        session, doc, layout = _opened(box)
        call(box, session, "word_edit_table", doc=doc, table=layout.fill, action="set_cells", grow=True,
             **{"from": "d:rows"})
        rows = table_texts(session, doc, layout.fill)
    assert rows[0] == ["name", "count"] and len(rows) == 7 and rows[1] == ["alpha", "0"]


# -- two at once: as one at a time in commit order ----------------------------------------------


@pytest.mark.parametrize("first_at, then_at", [
    ("before:{end}", "before:{end}"),        # the same place, both before the same block
    ("after:{last}", "before:{end}"),        # the same gap, from either side
    ("before:{end}", "after:{last}"),
])
def test_inserts_into_one_gap_apply_once_each_as_in_commit_order(first_at, then_at):
    def at(text, layout):
        return text.format(end=layout.ends[1], last=layout.tables[1])

    first, then, data, same = _side_by_side(
        lambda doc, layout: ("word_insert_markdown", {"doc": doc, "at": at(first_at, layout), "markdown": "Held."}),
        lambda doc, layout: ("word_insert_markdown", {"doc": doc, "at": at(then_at, layout), "markdown": "Other."}))
    assert first.ok and then.ok and same
    texts = [text_of(p) for p in tree(data).iter(_W + "p")]
    assert texts.count("Held.") == 1 and texts.count("Other.") == 1


@pytest.mark.parametrize("second", ["https://example.com/y", "https://example.com/x"])
def test_links_side_by_side_merge_or_share_their_relationship(second):
    first, then, data, same = _side_by_side(
        lambda doc, layout: ("word_links", {"doc": doc, "action": "add_hyperlink",
                                            "target": f"{layout.bodies[0][0]}@0:4", "url": "https://example.com/x"}),
        lambda doc, layout: ("word_links", {"doc": doc, "action": "add_hyperlink",
                                            "target": f"{layout.bodies[2][0]}@0:4", "url": second}))
    assert first.ok and then.ok and same
    rels = tree(data, "word/_rels/document.xml.rels")
    targets = [r.get("Target") for r in rels if r.get("TargetMode") == "External"]
    assert sorted(targets) == sorted({"https://example.com/x", second})


def test_bookmarks_side_by_side_draw_ids_of_their_own():
    first, then, data, same = _side_by_side(
        lambda doc, layout: ("word_links", {"doc": doc, "action": "add_bookmark",
                                            "target": f"{layout.bodies[0][0]}@0:4", "bookmark": "held_mark"}),
        lambda doc, layout: ("word_links", {"doc": doc, "action": "add_bookmark",
                                            "target": f"{layout.bodies[2][0]}@0:4", "bookmark": "other_mark"}))
    assert first.ok and then.ok and same
    ids = [b.get(_W + "id") for b in tree(data).iter(_W + "bookmarkStart")]
    assert len(ids) == len(set(ids)) == 2


def test_a_bookmark_name_taken_side_by_side_is_refused_to_the_later():
    first, then, _data, same = _side_by_side(
        lambda doc, layout: ("word_links", {"doc": doc, "action": "add_bookmark",
                                            "target": f"{layout.bodies[0][0]}@0:4", "bookmark": "Mark"}),
        lambda doc, layout: ("word_links", {"doc": doc, "action": "add_bookmark",
                                            "target": f"{layout.bodies[2][0]}@0:4", "bookmark": "mark"}))
    assert sorted([first.ok, then.ok]) == [False, True] and same


def test_a_tracked_batch_and_a_tracked_format_side_by_side():
    """A batch of tracked edits (an insert, text set, a format: insertions, deletions,
    property changes) beside another tracked format."""
    first, then, data, same = _side_by_side(
        lambda doc, layout: ("batch", {"ops": [
            {"tool": "word_insert_markdown", "arguments": {"doc": doc, "at": f"after:{layout.bodies[0][0]}",
                                                           "markdown": "- one\n- two"}},
            {"tool": "word_set_text", "arguments": {"doc": doc, "target": layout.bodies[0][1], "text": "Changed."}},
            {"tool": "word_format", "arguments": {"doc": doc, "targets": [layout.bodies[0][0]], "italic": True}}]}),
        lambda doc, layout: ("word_format", {"doc": doc, "targets": [layout.bodies[2][0]], "bold": True}),
        tracked="Writer")
    assert first.ok and then.ok, first.to_json()
    assert same
    ids = _annotation_ids(data)
    assert len(ids) == len(set(ids))
    assert first.checks["validate"]["new"] == []


@pytest.mark.parametrize("batch", [False, True])
def test_tracked_links_to_one_address_side_by_side_share_its_relationship(batch):
    def link(doc, layout):
        made = ("word_links", {"doc": doc, "action": "add_hyperlink", "url": "https://example.com/a",
                               "target": f"{layout.bodies[0][0]}@0:4"})
        if not batch:
            return made
        return ("batch", {"ops": [
            {"tool": "word_set_text", "arguments": {"doc": doc, "target": layout.bodies[0][1], "text": "Text first."}},
            {"tool": made[0], "arguments": made[1]}]})

    first, then, data, same = _side_by_side(
        link, lambda doc, layout: ("word_links", {"doc": doc, "action": "add_hyperlink", "url": "https://example.com/a",
                                                  "target": f"{layout.bodies[2][0]}@0:4"}),
        tracked="Writer")
    assert first.ok and then.ok, first.to_json()
    assert first.checks["validate"]["new"] == [], first.checks
    assert same
    rels = tree(data, "word/_rels/document.xml.rels")
    assert [r.get("Target") for r in rels if r.get("TargetMode") == "External"] == ["https://example.com/a"]


def test_a_tables_width_of_the_body_beside_an_insert_before_it():
    """Lane J (0.1.120): ``word_format_table``'s width "of the body" reads the nearest
    paragraph before the table; a paragraph put between them at the same time gives the
    width and indent of the two calls one at a time in commit order."""
    first, then, _data, same = _side_by_side(
        lambda doc, layout: ("word_format_table", {"doc": doc, "table": layout.tables[0], "scope": "table",
                                                   "width_of": "body"}),
        lambda doc, layout: ("word_insert_markdown", {"doc": doc, "at": f"after:{layout.items[0][-1]}",
                                                      "markdown": "A plain paragraph."}))
    assert first.ok and then.ok and same


def test_tracked_edits_made_side_by_side_are_accepted_and_rejected_by_id():
    """Revisions of edits committed side by side are each the document's own: accepted or
    rejected by id, by author, all."""
    with _box() as box:
        session, doc, layout = _opened(box)
        call(box, session, "word_set_tracking", doc=doc, on=True, author="Writer")
        results: list = []
        go = threading.Barrier(3)

        def run(k):
            go.wait(60)
            results.append(box.dispatch(session, "word_set_text", {"doc": doc, "target": layout.bodies[k][0],
                                                                   "text": f"Tracked {k}."}))

        threads = [threading.Thread(target=run, args=(k,)) for k in range(3)]
        for thread in threads:
            thread.start()
        for thread in threads:
            thread.join(120)
        assert len(results) == 3 and all(r.ok for r in results)
        ids = _annotation_ids(saved(session, doc))
        assert len(ids) == len(set(ids)) and validation(box, session, doc)["new"] == []
        listed = call(box, session, "word_changes", doc=doc, action="list").data
        changes = listed["changes"] if isinstance(listed, dict) else listed
        call(box, session, "word_changes", doc=doc, action="accept", ids=[changes[0]["id"]])
        call(box, session, "word_changes", doc=doc, action="reject", all=True)
        assert not [c for c in (lambda d: d["changes"] if isinstance(d, dict) else d)(
            call(box, session, "word_changes", doc=doc, action="list").data)]
        assert validation(box, session, doc)["new"] == []
        texts = [text_of(element(session, doc, layout.bodies[k][0])) for k in range(3)]
    assert sum(text.startswith("Tracked ") for text in texts) <= 1


# -- versions, steps and reads ------------------------------------------------------------------


def test_each_commit_is_one_version_and_one_step_of_its_callers():
    with _box() as box:
        session, doc, layout = _opened(box)
        a, b = session.view(context={"agent": "a"}), session.view(context={"agent": "b"})
        first = call(box, a, "word_set_text", doc=doc, target=layout.bodies[0][0], text="A's.")
        later = call(box, b, "word_set_text", doc=doc, target=layout.bodies[2][0], text="B's.")
        assert (first.version, later.version) == (session.save(doc).version - 1, session.save(doc).version)
        # A's own undo is refused: B's change came after it, and the document undoes whole.
        refused = box.dispatch(a, "undo", {"doc": doc})
        assert not refused.ok and refused.error.code == "entangled", refused.to_json()
        assert "word_set_text by agent=b" in refused.error.message
        assert text_of(element(session, doc, layout.bodies[0][0])) == "A's."
        call(box, b, "undo", doc=doc)            # B's is the latest: undone
        assert text_of(element(session, doc, layout.bodies[2][0])) != "B's."
        assert text_of(element(session, doc, layout.bodies[0][0])) == "A's."


def test_a_batch_s_read_is_of_the_version_its_change_made():
    with _box() as box:
        session, doc, layout = _opened(box)
        result = call(box, session, "batch", ops=[
            {"tool": "word_insert_markdown", "arguments": {"doc": doc, "at": f"before:{layout.ends[2]}",
                                                           "markdown": "Seen by the batch's read."}},
            {"tool": "word_read", "arguments": {"doc": doc, "range": f"{layout.heads[2]}..{layout.ends[2]}"}}])
    read = result.data["ops"][1]
    assert "Seen by the batch's read." in str(read["data"]) and "version" not in read
    assert result.checks["validate"]["new"] == []


def test_a_retry_key_used_again_answers_as_done():
    with _box() as box:
        session, doc, layout = _opened(box)
        arguments = {"doc": doc, "at": f"before:{layout.ends[0]}", "markdown": "Once.", "key": "k1"}
        first = call(box, session, "word_insert_markdown", **arguments)
        again = call(box, session, "word_insert_markdown", **arguments)
        texts = [text_of(p) for p in tree(saved(session, doc)).iter(_W + "p")]
    assert first.ok and again.summary.startswith("already done")
    assert texts.count("Once.") == 1


def test_a_cells_empty_from_is_no_source():
    """Lane D's find: a strict decoder's ``from: ""`` beside a cell's text is no source."""
    with _box() as box:
        session, doc, layout = _opened(box)
        call(box, session, "word_edit_table", doc=doc, table=layout.tables[0], action="set_cells",
             cells=[{"row": 1, "col": 1, "text": "7", "from": ""}], **{"from": ""})
        assert table_texts(session, doc, layout.tables[0])[1][1] == "7"


def test_a_dataset_table_put_alone_after_edits_side_by_side():
    """Round 48 (R85, the team's case): after callers edited side by side, a table from a
    dataset put in alone (`from: "d:..."`) is made, its id the document's."""
    with _box() as box:
        session, doc, layout = _opened(box)

        def edit(k):
            view = session.view(context={"agent": f"c{k}"})
            for c in range(3):
                assert box.dispatch(view, "batch", {"ops": [
                    {"tool": "word_insert_markdown", "arguments": {"doc": doc, "at": f"before:{layout.ends[k]}",
                                                                   "markdown": f"Note {k}.{c}"}},
                    {"tool": "word_set_text", "arguments": {"doc": doc, "target": layout.bodies[k][c],
                                                            "text": f"Text {k}.{c}."}}]}).ok

        threads = [threading.Thread(target=edit, args=(k,)) for k in range(3)]
        for thread in threads:
            thread.start()
        for thread in threads:
            thread.join(120)
        result = call(box, session.view(context={"agent": "alone"}), "word_insert_markdown",
                      doc=doc, at=f"before:{layout.ends[1]}", **{"from": "d:rows"})
        assert result.created[0].startswith("t:")
        assert element(session, doc, result.created[0]) is not None


def test_edits_of_known_blocks_each_answer_and_keep_the_document_valid():
    """Round 48 (R83): replace_text in a span, word_format, word_format_table, word_delete and
    word_links (a link, a bookmark) -- alone and mixed in one batch.  (replace_text within a
    range is a deferred form: here it replaces in the whole body.)"""
    with _box() as box:
        session, doc, layout = _opened(box)
        body, other, span = layout.bodies[1], layout.bodies[2], f"{layout.heads[1]}..{layout.ends[1]}"
        word = text_of(element(session, doc, body[0])).split()[1].strip(".,").lower()
        for name, arguments in [
            ("replace_text", {"doc": doc, "find": word, "replace": word.upper(), "expect": "all"}),
            ("word_format", {"doc": doc, "targets": [body[0]], "bold": True}),
            ("word_format", {"doc": doc, "targets": [f"{body[1]}@0:5", span], "italic": True}),
            ("word_format", {"doc": doc, "find": [{"text": word.upper(), "within": span, "expect": "all"}],
                             "underline": True}),
            ("word_format_table", {"doc": doc, "table": layout.tables[1], "scope": "table", "width_of": "body"}),
            ("word_format_table", {"doc": doc, "table": layout.tables[1], "scope": "row", "row": 0,
                                   "shading": "DDEEFF"}),
            ("word_delete", {"doc": doc, "target": f"{body[2]}@0:3"}),
            ("word_links", {"doc": doc, "action": "add_bookmark", "target": f"{body[0]}@1:6", "bookmark": "a_mark"}),
            ("word_links", {"doc": doc, "action": "add_hyperlink", "target": f"{body[3]}@0:4",
                            "url": "https://example.com/a"}),
            ("word_links", {"doc": doc, "action": "add_hyperlink", "target": f"{body[3]}@6:9", "bookmark": "a_mark"}),
            ("batch", {"ops": [
                {"tool": "replace_text", "arguments": {"doc": doc, "find": word.upper(), "replace": word,
                                                       "expect": "all"}},
                {"tool": "word_format", "arguments": {"doc": doc, "targets": [body[1]], "bold": True}},
                {"tool": "word_format_table", "arguments": {"doc": doc, "table": layout.tables[1], "scope": "table",
                                                            "borders": "single"}},
                {"tool": "word_delete", "arguments": {"doc": doc, "target": other[0]}},
                {"tool": "word_links", "arguments": {"doc": doc, "action": "add_hyperlink",
                                                     "target": f"{body[0]}@0:3", "url": "https://example.com/b"}}]}),
            ("word_delete", {"doc": doc, "target": other[2]}),
        ]:
            found = box.dispatch(session, name, arguments)
            assert found.ok, (name, found.to_json())
        assert validation(box, session, doc)["new"] == []
