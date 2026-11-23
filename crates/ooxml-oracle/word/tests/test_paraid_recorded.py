"""paraIds as the tools stamp them, held to what Word keeps (``probes/paraid_probe.py``,
``observations/paraid-durability.json``, Word 16.106): Word keeps every paragraph's paraId and
textId only when every paragraph of the document has a valid, unique paraId below 0x80000000
(``factors``: one missing or out of range and it drops them all, a repeat it replaces).  The
tools' stamping on a document's first edit makes every paragraph so, and Word kept the ids
of documents the tools stamped (``decided``)."""

from __future__ import annotations

import pytest
from recorded import observed  # noqa: I001  (puts the probes on the path)

import paraid_probe  # noqa: E402

OBSERVED = observed("paraid-durability.json")
#: The observation's keys these tests compare with (index.py lists why the others are not).
READS = {"factors", "decided"}


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
