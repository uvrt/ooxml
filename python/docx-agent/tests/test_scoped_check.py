"""Round 35: ``check`` and ``render`` scoped to some blocks (R18), and copies compared with the
reference they were made from (R19).

R18: ``check(range=[...])`` reports the facts about those blocks -- the layout's, on the pages
they fall on; ``validate``'s problems found in them -- and each range's pages;
``render(range=[...])`` draws those pages.  R19: ``check(copies=[...])`` says per copy whether
it follows its reference, what differs (with its fixes and kind), and where the reference's
example text is still in it; each comparison is kept by what both sides hold, so an edit
elsewhere finds it kept.

The shared definitions of ``check`` and ``render`` are ooxml-edit's (``range`` and
``copies`` since 0.81.0, the reference's document locked with the copy's).  Every document
here is made up.
"""

from __future__ import annotations

import io
import zipfile

import pytest

pytest.importorskip("ooxml_edit.tools.shared", reason="needs ooxml-edit (the shared tools)")

from ooxml_edit.tools import Toolbox  # noqa: E402

from layout_reads import block_ids, pages_of  # noqa: E402

from docx_agent.tools import FORMAT, GROUPS, TOOLS  # noqa: E402
import word_docs  # noqa: E402

_W = "{http://schemas.openxmlformats.org/wordprocessingml/2006/main}"

pytestmark = pytest.mark.any_faces

@pytest.fixture
def box():
    with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS, workers=0) as toolbox:
        yield toolbox


def _ok(box, session, tool: str, /, **arguments):
    result = box.dispatch(session, tool, arguments)
    assert result.ok, (tool, arguments, result.error and result.error.to_json())
    return result


# -- a report of chapters ------------------------------------------------------------------------

WORDS = "lantern meadow river stone harbour quiet orchard copper valley signal".split()


def _written(box, session, markdown: str) -> str:
    """A new blank document holding ``markdown`` alone (its empty paragraph taken out, but
    after a table, which Word needs a paragraph after)."""
    doc = session.open(word_docs.blank(), "made.docx")
    _ok(box, session, "word_insert_markdown", doc=doc, at="end", markdown=markdown)
    listed = word_docs.blocks(box, session, doc)
    for k, (block, text) in enumerate(listed):
        if not text and not (k and listed[k - 1][0].startswith("t")):
            _ok(box, session, "word_delete", doc=doc, target=block)
    return doc


def _report() -> tuple[bytes, list[list[str]]]:
    """Three chapters, each a heading and paragraphs over more than a page; in each, one
    paragraph made bold by hand (a direct-formatting fact).  The blocks of each chapter."""
    parts = []
    for c in range(1, 4):
        parts.append(f"# Chapter {c}")
        for p in range(6 if c != 3 else 26):
            parts.append(" ".join(WORDS[(c + p + k) % len(WORDS)] for k in range(70)).capitalize() + ".")
    with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS, workers=0) as box:
        session = box.session()
        doc = _written(box, session, "\n\n".join(parts))
        listed = word_docs.blocks(box, session, doc)
        blocks = [block for block, _ in listed]
        heads = [k for k, (_, text) in enumerate(listed) if text.startswith("#")]
        chapters = [blocks[a:b] for a, b in zip(heads, heads[1:] + [len(blocks)])]
        for chapter in chapters:
            _ok(box, session, "word_format", doc=doc, targets=[chapter[2]], bold=True)
        return word_docs.saved(session, doc), chapters


@pytest.fixture(scope="module")
def made():
    return _report()


def _span(blocks: list[str]) -> str:
    return f"{blocks[0]}..{blocks[-1]}"


# -- R18: check(range) -----------------------------------------------------------------------------


def test_a_range_reports_its_blocks_facts_and_pages(box, made):
    data, chapters = made
    session = box.session()
    d1 = session.open(data, "report.docx")
    whole = _ok(box, session, "check", doc=d1, include=["layout"]).data["layout"]
    flagged = {e["para"] for e in whole["direct_formatting"]}
    assert {chapter[2] for chapter in chapters} <= flagged          # one bold paragraph per chapter
    result = _ok(box, session, "check", doc=d1, range=[_span(chapters[1])])
    scoped_facts = result.data["layout"]
    assert [e["para"] for e in scoped_facts["direct_formatting"]] == [chapters[1][2]]
    pages = sorted({p for b in chapters[1] for p in pages_of(box, session, d1, b)})
    reported = dict(result.data["range"][0])
    reported.pop("extent"), reported.pop("height")          # R26's (tests/test_range_extent.py)
    for key in ("one_page", "kept", "breaks", "fix"):         # round 36, R30's (tests/test_keep_spans.py)
        reported.pop(key, None)
    for key in ("if_kept", "if_released", "effect_left_out"):  # round 40's (tests/test_keep_trials.py)
        reported.pop(key, None)
    assert [reported] == [{"range": _span(chapters[1]), "blocks": len(chapters[1]), "pages": pages}]
    assert "in 1 range(s)" in result.summary
    assert "layout" in result.data and "reflow" in result.data          # range adds layout to the default


def test_several_ranges_in_one_check(box, made):
    data, chapters = made
    session = box.session()
    d1 = session.open(data, "report.docx")
    result = _ok(box, session, "check", doc=d1, include=["layout"], range=[_span(chapters[0]), chapters[2][2]])
    assert [r["range"] for r in result.data["range"]] == [_span(chapters[0]), chapters[2][2]]
    assert result.data["range"][1]["blocks"] == 1
    paragraphs = [e["para"] for e in result.data["layout"]["direct_formatting"]]
    assert paragraphs == [chapters[0][2], chapters[2][2]]


def test_a_range_with_pages_keeps_the_facts_on_those_pages(box, made):
    data, chapters = made
    session = box.session()
    d1 = session.open(data, "report.docx")
    on = _ok(box, session, "check", doc=d1, include=["layout"], range=[_span(chapters[2])]).data["range"][0]["pages"]
    assert len(on) > 4                                                   # the long chapter
    elsewhere = [p for p in range(1, on[0])]
    result = box.dispatch(session, "check", {"doc": d1, "include": ["layout"], "range": [_span(chapters[2])],
                                             "pages": elsewhere})
    assert result.ok
    assert result.data["layout"]["direct_formatting"] == []
    assert any("none of the range's blocks" in w for w in result.warnings)


def _styled_wrongly(data: bytes, paragraph: str) -> bytes:
    """``data`` with ``paragraph`` in a style the document does not have."""
    from lxml import etree

    root = word_docs.tree(data)
    element = word_docs.find(root, paragraph)
    properties = element.find(_W + "pPr")
    if properties is None:
        properties = etree.Element(_W + "pPr")
        element.insert(0, properties)
    style = etree.Element(_W + "pStyle")
    style.set(_W + "val", "NoSuchStyle")
    properties.insert(0, style)
    return word_docs.replaced(data, {"word/document.xml": etree.tostring(root, xml_declaration=True,
                                                                         encoding="UTF-8", standalone=True)})


def test_validate_problems_are_found_in_their_blocks(box, made):
    data, chapters = made
    session = box.session()
    d1 = session.open(_styled_wrongly(data, chapters[0][1]), "report.docx")
    first = _ok(box, session, "check", doc=d1, include=["validate"], range=[_span(chapters[0])]).data["validate"]
    assert [p for p in first["problems"] if "style-missing" in p] == ["[style-missing] word/document.xml: pStyle NoSuchStyle"]
    assert first["elsewhere"] == 0
    second = _ok(box, session, "check", doc=d1, include=["validate"], range=[_span(chapters[1])]).data["validate"]
    assert second["problems"] == [] and second["elsewhere"] == 1 and second["baseline"] == 1


def _codes(box, session, doc: str, target: str) -> set[str]:
    """The codes of the problems ``check`` finds in a range of one block."""
    found = _ok(box, session, "check", doc=doc, include=["validate"], range=[target]).data["validate"]["problems"]
    return {problem[1:problem.index("]")] for problem in found}


def test_problems_in_blocks_by_what_they_carry(box):
    """The locator: problems a block's elements carry, or that its XML alone shows again."""
    from lxml import etree

    session = box.session()
    made = _written(box, session, "First paragraph.\n\nSecond paragraph.\n\n| A | B |\n|---|---|\n| 1 | 2 |\n")
    first_id, second_id, table_id = [block for block, _ in word_docs.blocks(box, session, made)][:3]
    root = word_docs.tree(word_docs.saved(session, made))
    paragraphs = root.findall(f".//{_W}body/{_W}p")
    table = root.find(f".//{_W}tbl")
    mark = etree.SubElement(paragraphs[0], _W + "bookmarkStart")
    mark.set(_W + "id", "7")
    mark.set(_W + "name", "Lonely")
    cell = table.find(f".//{_W}tc")
    for p in cell.findall(_W + "p"):
        cell.remove(p)
    etree.SubElement(cell, _W + "tbl")                     # a cell ending in a table

    def opened(name: str) -> str:
        xml = etree.tostring(root, xml_declaration=True, encoding="UTF-8", standalone=True)
        # A part no relationship names: a problem of the package, in no block.
        data = word_docs.replaced(word_docs.saved(session, made), {"word/document.xml": xml})
        with_orphan = io.BytesIO()
        with zipfile.ZipFile(io.BytesIO(data)) as source, zipfile.ZipFile(with_orphan, "w") as target:
            for info in source.infolist():
                target.writestr(info, source.read(info.filename))
            target.writestr("word/media/x.png", b"not an image")
        return session.open(with_orphan.getvalue(), name)

    d = opened("located.docx")
    whole = _ok(box, session, "check", doc=d, include=["validate"]).data["validate"]["problems"]
    codes = {problem[1:problem.index("]")] for problem in whole}
    assert {"bookmark-unpaired", "cell-not-ending-in-paragraph"} <= codes
    first = _codes(box, session, d, first_id)
    assert "bookmark-unpaired" in first and "cell-not-ending-in-paragraph" not in first
    assert not _codes(box, session, d, second_id) & {"bookmark-unpaired", "cell-not-ending-in-paragraph"}
    assert "cell-not-ending-in-paragraph" in _codes(box, session, d, table_id)
    assert not any(code.startswith("orphan") for code in first)
    # A table given as a block holds its own grid's problems.
    row = table.find(f"{_W}tr")
    row.append(etree.fromstring(etree.tostring(row.find(f"{_W}tc"))))   # the row past its grid
    widened = opened("widened.docx")
    assert any(code.startswith("grid-") for code in _codes(box, session, widened, table_id))


def test_a_range_that_names_nothing_is_refused_with_the_headings(box, made):
    """Rule 2: the way out -- the sections to name, as heading ranges (h:)."""
    data, chapters = made
    session = box.session()
    d1 = session.open(data, "report.docx")
    result = box.dispatch(session, "check", {"doc": d1, "range": ["p:DEADBEEF"]})
    assert not result.ok
    error = result.error.to_json()
    assert error["field"] == "range" and "range[0]" in error["message"]
    assert error["valid_options"][0].startswith("h:" + chapters[0][0])     # round 36: a heading's range
    assert any("Chapter 2" in option for option in error["valid_options"])


def test_one_range_said_as_a_string_and_placeholders(box, made):
    """Rule 1: a lone string is one range; blanks and repeats are dropped."""
    data, chapters = made
    session = box.session()
    d1 = session.open(data, "report.docx")
    span = _span(chapters[0])
    lone = _ok(box, session, "check", doc=d1, include=["validate"], range=span).data["range"]
    assert [r["range"] for r in lone] == [span]
    listed = _ok(box, session, "check", doc=d1, include=["validate"], range=["", f" {span} ", span]).data["range"]
    assert [r["range"] for r in listed] == [span]


def test_an_empty_range_is_no_range(box, made):
    data, _ = made
    session = box.session()
    d1 = session.open(data, "report.docx")
    result = _ok(box, session, "check", doc=d1, include=["validate"], range=[])
    assert "range" not in result.data and "elsewhere" not in result.data["validate"]


def test_a_cell_range_is_about_the_cell_and_its_table(box):
    """A range naming a cell is about the cell and its table: the facts of the table's
    paragraphs, never of a paragraph elsewhere, and the pages it is on."""
    session = box.session()
    d1 = _written(box, session, "Before.\n\n| A | B |\n|---|---|\n| one | two |\n\nAfter.\n")
    before, table = block_ids(box, session, d1)[:2]
    inspected = _ok(box, session, "word_inspect", doc=d1, target=table).data["blocks"][0]
    cell = inspected["rows"][1][1]["blocks"][0]["id"]
    _ok(box, session, "word_format", doc=d1, targets=[before, cell], alignment="center")
    table = block_ids(box, session, d1)[1]
    whole = _ok(box, session, "check", doc=d1, include=["layout"]).data["layout"]["direct_formatting"]
    assert {before, cell} <= {e["para"] for e in whole}
    result = _ok(box, session, "check", doc=d1, include=["layout"], range=[f"{table}/c1,1"]).data
    scoped = {e["para"] for e in result["layout"]["direct_formatting"]}
    assert cell in scoped and before not in scoped
    table_paragraphs = {block["id"] for row in inspected["rows"] for c in row for block in c["blocks"]}
    assert scoped <= table_paragraphs
    [entry] = result["range"]
    assert entry["range"] == f"{table}/c1,1" and entry["pages"] == [1]


# -- R18: render(range) ----------------------------------------------------------------------------


def test_render_draws_the_pages_a_range_falls_on(box, made):
    pytest.importorskip("resvg_py")
    data, chapters = made
    session = box.session()
    d1 = session.open(data, "report.docx")
    result = _ok(box, session, "render", doc=d1, range=[chapters[1][1]], width=300)
    assert result.data["pages"] == pages_of(box, session, d1, chapters[1][1])
    assert result.data["range"][0]["pages"] == result.data["pages"]
    # More pages than a call's images: one sheet, said.
    long = _ok(box, session, "render", doc=d1, range=[_span(chapters[2])], width=300)
    assert "sheet" in long.data and long.data["pages"] == long.data["range"][0]["pages"]
    assert any("drawn as one sheet" in w for w in long.warnings)
    refused = box.dispatch(session, "render", {"doc": d1, "range": [chapters[1][1]], "pages": [99]})
    assert not refused.ok and refused.error.to_json()["valid_options"] == result.data["pages"]


# -- R18: no wait for a layout under the lock ------------------------------------------------------

DEADLINE = 90.0


# -- R19: copies against their reference -----------------------------------------------------------

SAMPLE = ("# Part title\n\n"
          "Describe the purpose of this part here, in a few sentences. Then name who owns it.\n\n"
          "| Item | Owner | Status |\n| --- | --- | --- |\n| Write the first item here | Someone | Open |\n\n"
          "Close with a short summary of the part.\n")


def _samples(box, session) -> tuple[str, str]:
    """A document holding the sample, its table's first row a header row; its span."""
    sample = _written(box, session, SAMPLE)
    table = next(block for block in block_ids(box, session, sample) if block.startswith("t"))
    _ok(box, session, "word_format_table", doc=sample, table=table, scope="table", header_rows=1)
    return sample, _span(block_ids(box, session, sample))


def _copies(box, session, count: int = 6) -> tuple[str, str, str, list[str]]:
    """A samples document and a report holding ``count`` copies of its one sample."""
    samples, reference = _samples(box, session)
    d1 = _written(box, session, "# Report\n\nAn opening paragraph.")
    made = []
    for _ in range(count):
        blocks = _ok(box, session, "word_copy_from", doc=d1, source_doc=samples, range=reference,
                     at="end").data["blocks"]
        made.append(_span(blocks))
    return d1, samples, reference, made


def _pairs(made: list[str], samples: str, reference: str) -> list[dict]:
    return [{"blocks": c, "reference": {"doc": samples, "block": reference}} for c in made]


def _blocks(box, session, doc: str, span: str) -> list[str]:
    """The body's blocks from the span's first to its last."""
    first, _, last = span.partition("..")
    listed = block_ids(box, session, doc)
    return listed[listed.index(first):listed.index(last or first) + 1]


def test_copies_follow_or_not_with_fixes_and_example_text(box):
    session = box.session()
    d1, samples, reference, made = _copies(box, session)
    # Copy 0 filled in full; copy 1 filled, its table's first body row shaded by hand; the
    # rest left with the sample's text.
    for k in (0, 1):
        blocks = _blocks(box, session, d1, made[k])
        _ok(box, session, "word_set_text", doc=d1, target=blocks[1], text=f"Part {k} keeps the books in order.")
        _ok(box, session, "word_set_text", doc=d1, target=blocks[3], text=f"Part {k} is done.")
        _ok(box, session, "word_set_text", doc=d1, target=f"{blocks[2]}/c1,0", text=f"Ledger {k}")
    second_table = _blocks(box, session, d1, made[1])[2]
    _ok(box, session, "word_format_table", doc=d1, table=second_table, scope="row", row=1, shading="F2F2F2")
    result = _ok(box, session, "check", doc=d1, include=[], copies=_pairs(made, samples, reference))
    found = result.data["copies"]
    items = found["items"]
    assert found["compared"] == 6 and found["follow"] == 5
    assert [item["follows"] for item in items] == [True, False, True, True, True, True]
    assert "example_text" not in items[0] and "example_text" not in items[1]
    assert found["with_example_text"] == 4
    left = items[2]["example_text"]
    assert [e["text"] for e in left][0] == ["describe the purpose of this part here, in a few sentences.",
                                            "then name who owns it."]
    assert all("part title" not in t for e in left for t in e["text"])     # a two-word heading: a label
    assert all("item" != t for e in left for t in e["text"])                # the header row: kept by design
    differences = items[1]["differences"]
    assert differences and all(d["kind"] == "format" for d in differences)
    assert {d["property"] for d in differences} == {"shading"}
    # Rule 5: one call makes every copy follow its reference.
    assert found["fix"] == {"tool": "word_follow_sample",
                            "calls": [{"sample": {"block": _blocks(box, session, samples, reference)[2],
                                                  "doc": samples}, "copies": [second_table]}]}
    for call in found["fix"]["calls"]:
        _ok(box, session, "word_follow_sample", doc=d1, **call)
    again = _ok(box, session, "check", doc=d1, include=[], copies=_pairs(made, samples, reference))
    assert again.data["copies"]["follow"] == 6
    assert "copies: 6 of 6 follow their reference, 4 with its example text" in again.summary


def test_a_comparison_is_kept_while_neither_side_changes(box):
    session = box.session()
    d1, samples, reference, made = _copies(box, session)
    pairs = _pairs(made, samples, reference)
    first = _ok(box, session, "check", doc=d1, include=[], copies=pairs).data["copies"]
    assert first["kept"] == 0
    # An edit elsewhere in the report: every comparison kept.
    opening = block_ids(box, session, d1)[1]
    _ok(box, session, "word_set_text", doc=d1, target=opening, text="Another opening paragraph.")
    second = _ok(box, session, "check", doc=d1, include=[], copies=pairs).data["copies"]
    assert second["kept"] == 6 and all(item["cached"] for item in second["items"])
    assert [i["follows"] for i in second["items"]] == [i["follows"] for i in first["items"]]
    # An edit inside copy 3: that one compared again.
    _ok(box, session, "word_set_text", doc=d1, target=_blocks(box, session, d1, made[3])[1], text="Filled in.")
    third = _ok(box, session, "check", doc=d1, include=[], copies=pairs).data["copies"]
    assert [item["cached"] for item in third["items"]] == [True, True, True, False, True, True]
    assert "example_text" in third["items"][2] and len(third["items"][3]["example_text"]) == 2
    # The reference changed: every copy compared again.
    _ok(box, session, "word_format", doc=samples, targets=[_blocks(box, session, samples, reference)[1]],
        alignment="center")
    fourth = _ok(box, session, "check", doc=d1, include=[], copies=pairs).data["copies"]
    assert fourth["kept"] == 0 and fourth["follow"] == 0
    assert all(any(d["property"] == "alignment" for d in item["differences"]) for item in fourth["items"])


def test_copies_refused_with_the_way_out(box):
    """Rule 2: a reference document not open, a copy naming nothing, a malformed item."""
    session = box.session()
    d1, samples, reference, made = _copies(box, session, count=1)
    result = box.dispatch(session, "check", {"doc": d1, "copies": [{"blocks": made[0],
                                                                    "reference": {"doc": "d9", "block": reference}}]})
    assert not result.ok
    result = box.dispatch(session, "check", {"doc": d1, "include": [], "copies": [
        {"blocks": "p:DEADBEEF", "reference": {"doc": samples, "block": reference}}]})
    assert not result.ok and result.error.to_json()["valid_options"]
    assert "copies[0].blocks" in result.error.message


def test_example_text_rules(box):
    """Units: sentences of three words or twenty characters, not in a header row; matched as
    whole words; a unit from an unmarked first row is marked."""
    session = box.session()
    markdown = ("{aim}\n\n| Area name | Value |\n|---|---|\n| Put the area here | 0 |\n")
    reference = _written(box, session, "Owner:\n\n" + markdown.format(aim="State the aim. Keep it short."))
    copy = _written(box, session, "Owner:\n\n" + markdown.format(aim="State the aims. Keep it short."))
    pair = {"blocks": _span(block_ids(box, session, copy)),
            "reference": {"doc": reference, "block": _span(block_ids(box, session, reference))}}
    [item] = _ok(box, session, "check", doc=copy, include=[], copies=[pair]).data["copies"]["items"]
    found = item["example_text"]
    texts = [t for e in found for t in e["text"]]
    assert "keep it short." in texts and "state the aim." not in texts     # "aims": another word
    assert "put the area here" in texts
    assert "owner:" not in texts and "area name" not in texts               # a label: no unit
    assert not any(e.get("first_row") for e in found)                       # the second row
