"""paraIds as the tools stamp them, held to what Word keeps (``probes/paraid_probe.py``,
``observations/paraid-durability.json``, Word 16.106): Word keeps every paragraph's paraId and
textId only when every paragraph of the document has a valid, unique paraId below 0x80000000
(``factors``: one missing or out of range and it drops them all, a repeat it replaces).  The
tools' stamping on a document's first edit makes every paragraph so, and Word kept the ids
of documents the tools stamped (``decided``), and every id of the tools' outputs, also where the
input had a row, header paragraph, picture or paragraph without one and the tool wrote none of
them (``tools``: stamped at commit, whatever the tool).  A drawing without ``wp14:anchorId``
and ``wp14:editId`` makes Word drop every paraId too, and Word never adds those ids itself
(``drawings``): the tools stamp every drawing that lacks them and keep the ids a drawing has."""

from __future__ import annotations

import pytest
from recorded import observed  # noqa: I001  (puts the probes on the path)

import paraid_drawings  # noqa: E402
import paraid_probe  # noqa: E402

OBSERVED = observed("paraid-durability.json")
#: The observation's keys these tests compare with (index.py lists why the others are not).
READS = {"factors", "decided", "tools", "drawings"}


def test_word_keeps_ids_only_where_every_paragraph_has_a_good_one():
    factors = OBSERVED["factors"]
    kept = {verdict for verdict in factors["complete"]["save"]["paragraphs"].values()}
    assert kept == {"paraId kept, textId kept"}
    for change in ("missing", "out-of-range"):
        assert "paraId dropped, textId dropped" in factors[change]["save"]["paragraphs"].values(), change
    assert "paraId replaced, textId kept" in factors["repeat"]["save"]["paragraphs"].values()
    for case, scenarios in OBSERVED["decided"].items():
        if case.startswith("generated"):
            verdicts = {v.split(",")[0] for v in scenarios["save"]["paragraphs"].values()}
            assert verdicts == {"paraId kept"}, case


@pytest.mark.parametrize("mode", [15, 14, None])
def test_the_tools_stamp_every_paragraph_as_word_keeps_them(mode):
    """The tools' stamping, on a document whose paragraphs have none: every paragraph of every
    story ends with a unique paraId below 0x80000000 and a textId -- the ``complete`` case."""
    data = paraid_probe.decided(paraid_probe.complete(mode, change="missing"))
    paragraphs = paraid_probe._paragraphs(data)
    ids = [para_id for _, para_id, _ in paragraphs]
    assert all(ids) and all(int(i, 16) < 0x80000000 for i in ids)
    assert len(set(ids)) == len(ids)
    assert all(text_id for _, _, text_id in paragraphs)


def test_word_keeps_every_id_of_the_tools_outputs():
    for name, result in OBSERVED["tools"].items():
        assert any(kind["without an id"] for kind in result["input"].values()), name
        for kind, counts in result["save"].items():
            assert counts["without an id"] == 0, (name, kind)
            assert counts["ids kept by Word"] == counts["blocks"], (name, kind)


@pytest.mark.parametrize("name", sorted(paraid_probe.TOOL_WRITES))
def test_the_tools_outputs_are_what_word_was_handed(name):
    """Each measured write, made again: no block of its output without an id."""
    _, output = paraid_probe.tool_write(name)
    for kind, counts in paraid_probe.durability(output, output).items():
        assert counts["without an id"] == 0, (name, kind)


def test_word_keeps_paraids_only_where_the_drawings_have_ids():
    drawings = OBSERVED["drawings"]
    for scenario, result in drawings["without ids"].items():
        if scenario == "input":
            continue
        assert result["paraIds kept"].startswith("0 of"), scenario
        for kind in ("inline", "anchor"):     # Word never adds a drawing's ids
            assert {d["anchorId"] for d in result[kind]} == {"none"}, (scenario, kind)
    for scenario, result in drawings["with ids"].items():
        if scenario == "input":
            continue
        total = result["paraIds kept"].split(" of ")
        assert total[0] == total[1], scenario
        for kind in ("inline", "anchor"):
            assert {d["anchorId"] for d in result[kind]} == {"kept"}, (scenario, kind)


@pytest.mark.parametrize("with_ids", [False, True])
def test_the_tools_stamp_drawings_as_word_keeps_them(with_ids):
    """An edit elsewhere: every drawing ends with both ids (``with ids``: its own, kept)."""
    document = paraid_probe.probe_tools.Session(paraid_drawings.document(with_ids))
    paraid_probe._stamp_by_editing(document, "T1")
    found = paraid_drawings.drawings(document.data())
    before = paraid_drawings.drawings(paraid_drawings.document(with_ids))
    for kind in ("inline", "anchor"):
        assert found[kind] and all(anchor and edit for anchor, edit in found[kind]), (kind, found[kind])
        if with_ids:
            assert found[kind] == before[kind], kind
