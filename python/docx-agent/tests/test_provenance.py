"""Source addresses kept in the saved document (once ``docx_agent.edit.provenance``): a table
or chart placed from a source keeps its record in one custom XML item related from the
document part, keyed by the table's ``t:`` id (its first row's ``w14:paraId``) or the
chart's ``d:`` id, read back after a save and a reopen, in the call's undo step, and kept
true -- or dropped -- by later edits.

Made-up data only: the dataset and the toy grid format of ``test_tables_from_data``.
"""

from __future__ import annotations

import io
import json
import zipfile

import pytest

pytest.importorskip("ooxml_edit.provenance", reason="needs ooxml-edit 0.25 (provenance)")

from lxml import etree  # noqa: E402
from ooxml_edit import provenance as records  # noqa: E402
from ooxml_edit.tools import Toolbox, strict_problems  # noqa: E402

from docx_agent.tools import FORMAT, GROUPS, TOOLS  # noqa: E402

import word_docs  # noqa: E402
from kept_records import records as kept  # noqa: E402

import test_tables_from_data as data  # noqa: E402
from test_tables_from_data import CLOCK, call, ds_put, failed, ok  # noqa: E402

REL_CUSTOM_XML = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/customXml"
#: The records' item's namespace, and the schema its properties name.
NS = "urn:uvrt:docx-agent:provenance:v1"
_W = "{%s}" % word_docs.W
PARA_ID = word_docs.W14_PARA


@pytest.fixture(scope="module")
def toolbox():
    with Toolbox([*TOOLS, ds_put], formats=[FORMAT, data.GRID], groups=GROUPS, workers=0) as box:
        yield box


@pytest.fixture
def session(toolbox):
    session = data._session(toolbox)
    ok(call(toolbox, session, "ds_put", id="scores"))
    return session


def saved(session, doc="d1") -> bytes:
    return word_docs.saved(session, doc)


def reopened(toolbox, raw: bytes):
    """``raw`` opened in another session: the session and its document."""
    other = toolbox.session(clock=CLOCK)
    return other, other.open(raw, "report.docx")


def with_parts(raw: bytes, added: dict[str, bytes], overrides: dict[str, str],
               relationships: list[tuple[str, str, str]]) -> bytes:
    """``raw`` with parts added, their content types overridden and relationships added
    (``(source part, type, target)``, the target relative to the source's folder)."""
    import posixpath

    found = parts(raw)
    found.update(added)
    types = found["[Content_Types].xml"].decode()
    types = types.replace("</Types>", "".join(f'<Override PartName="/{name}" ContentType="{kind}"/>'
                                              for name, kind in overrides.items()) + "</Types>")
    found["[Content_Types].xml"] = types.encode()
    for k, (source, kind, target) in enumerate(relationships):
        folder, name = posixpath.split(source)
        rels = f"{folder}/_rels/{name}.rels"
        xml = found.get(rels, b'<?xml version="1.0" encoding="UTF-8" standalone="yes"?>\n<Relationships '
                              b'xmlns="http://schemas.openxmlformats.org/package/2006/relationships"></Relationships>')
        found[rels] = xml.decode().replace(
            "</Relationships>", f'<Relationship Id="rIdTest{k}" Type="{kind}" Target="{target}"/></Relationships>').encode()
    out = io.BytesIO()
    with zipfile.ZipFile(out, "w", zipfile.ZIP_DEFLATED) as archive:
        for name, body in found.items():
            archive.writestr(name, body)
    return out.getvalue()


def valid_long_hex(value: str | None) -> bool:
    """A paraId Word keeps: eight hex digits below 0x80000000, not 0."""
    try:
        return value is not None and len(value) == 8 and 0 < int(value, 16) < 0x80000000
    except ValueError:
        return False


def insert(toolbox, session, **arguments):
    return ok(call(toolbox, session, "word_insert_markdown",
                   **{"doc": "d1", "at": "end", "from": "d:scores", **arguments}))


def edit_table(toolbox, session, table, action, **arguments):
    return ok(call(toolbox, session, "word_edit_table", doc="d1", table=table, action=action,
                   **arguments))


def parts(raw: bytes) -> dict[str, bytes]:
    with zipfile.ZipFile(io.BytesIO(raw)) as archive:
        return {name: archive.read(name) for name in archive.namelist()}


def items(raw: bytes) -> dict[str, bytes]:
    return {name: body for name, body in parts(raw).items() if name.startswith("customXml/")}


def columns(record) -> list[tuple[int, str]]:
    return [(c["col"], c["column"]) for c in record.get("columns", [])]


# -- write, then read back ------------------------------------------------------------------------


def test_a_table_from_a_source_is_kept_in_one_custom_xml_item(toolbox, session):
    result = insert(toolbox, session, from_columns=["check", "rating"])
    table = result.data["id"]
    raw = saved(session)
    found = parts(raw)
    assert sorted(items(raw)) == ["customXml/_rels/item1.xml.rels", "customXml/item1.xml",
                                  "customXml/itemProps1.xml"]
    root = etree.fromstring(found["customXml/item1.xml"])
    assert root.tag == f"{{{NS}}}sources"
    [entry] = list(root)
    assert (entry.get("kind"), entry.get("key")) == ("table", table)
    assert records.loads(entry.text) == {"v": 1, "source": "d:scores", "header_row": 0, "first_row": 1,
                                         "columns": result.data["from"]["columns"]}
    props = etree.fromstring(found["customXml/itemProps1.xml"])
    assert props.find(".//{*}schemaRef").get(
        "{http://schemas.openxmlformats.org/officeDocument/2006/customXml}uri") == NS
    rels = found["word/_rels/document.xml.rels"].decode()
    assert f'Type="{REL_CUSTOM_XML}" Target="../customXml/item1.xml"' in rels
    assert b"customXmlProps" in found["customXml/_rels/item1.xml.rels"]
    assert b'PartName="/customXml/itemProps1.xml"' in found["[Content_Types].xml"]
    # Read back after a save and a reopen, in another session.
    other, d = reopened(toolbox, raw)
    assert other.provenance(d) == {table: records.loads(entry.text)}
    assert kept(other, d).get("ignored", 0) == 0
    assert word_docs.validation(toolbox, other, d)["baseline"] == 0
    assert session.provenance("d1") == other.provenance(d)


def test_the_table_key_is_its_first_rows_para_id_and_every_paragraph_is_stamped(toolbox, session):
    table = insert(toolbox, session).data["id"]
    raw = saved(session)
    tree = word_docs.tree(raw)
    tbl = next(t for t in tree.iter(_W + "tbl"))
    first_row = tbl.find(_W + "tr")
    assert table == f"t:{first_row.get(PARA_ID)}"
    # The key holds through Word's save only when every paragraph and row has a valid,
    # unique paraId (ROADMAP.md, "Durable across a Word save -- measured"): docx-agent
    # stamps every one on the first edit.
    seen = set()
    for node in tree.iter(_W + "p", _W + "tr"):
        found = node.get(PARA_ID)
        assert valid_long_hex(found), etree.tostring(node)[:80]
        assert found not in seen
        seen.add(found)
    # The ids survive a save and a reopen unchanged, so the key still resolves.
    other, d = reopened(toolbox, raw)
    assert [block for block, _ in word_docs.blocks(toolbox, other, d) if block.startswith("t:")] == [table]
    assert list(other.provenance(d)) == [table]


def test_undo_takes_the_item_away_with_the_table(toolbox, session):
    before = saved(session)
    table = insert(toolbox, session).data["id"]
    ok(call(toolbox, session, "undo", doc="d1"))
    assert session.provenance("d1") == {}
    assert items(saved(session)) == {}
    assert parts(saved(session)) == parts(before)
    ok(call(toolbox, session, "undo", doc="d1", redo=True))
    assert list(session.provenance("d1")) == [table]


def test_another_custom_xml_item_is_left_as_it_is(toolbox):
    other = (b"<?xml version='1.0' encoding='UTF-8' standalone='yes'?>\n"
             b'<CoverPageProperties xmlns="http://schemas.microsoft.com/office/2006/coverPageProps">'
             b"<PublishDate>2026-10-14</PublishDate><Abstract>Made up.</Abstract></CoverPageProperties>")
    props = (b"<?xml version='1.0' encoding='UTF-8' standalone='yes'?>\n"
             b'<ds:datastoreItem ds:itemID="{11111111-2222-3333-4444-555555555555}" '
             b'xmlns:ds="http://schemas.openxmlformats.org/officeDocument/2006/customXml"><ds:schemaRefs>'
             b'<ds:schemaRef ds:uri="http://schemas.microsoft.com/office/2006/coverPageProps"/>'
             b"</ds:schemaRefs></ds:datastoreItem>")
    raw = with_parts(word_docs.with_body(word_docs.paragraph("Cover")),
                     {"customXml/item1.xml": other, "customXml/itemProps1.xml": props},
                     {"customXml/itemProps1.xml": "application/vnd.openxmlformats-officedocument.customXmlProperties+xml"},
                     [("customXml/item1.xml", REL_CUSTOM_XML + "Props", "itemProps1.xml"),
                      ("word/document.xml", REL_CUSTOM_XML, "../customXml/item1.xml")])
    start = items(raw)
    session = toolbox.session(clock=CLOCK)
    d = session.open(raw, "cover.docx")
    ok(call(toolbox, session, "ds_put", id="scores"))
    table = ok(call(toolbox, session, "word_insert_markdown", doc=d, at="end", **{"from": "d:scores"})).data["id"]
    after = items(saved(session, d))
    assert {k: after[k] for k in start} == start                 # untouched, byte for byte
    assert "customXml/item2.xml" in after and "customXml/itemProps2.xml" in after
    assert list(session.provenance(d)) == [table]
    # The last record dropped takes our item away, never the other one.
    ok(call(toolbox, session, "word_delete", doc=d, target=table))
    assert items(saved(session, d)) == start
    assert word_docs.validation(toolbox, session, d)["new"] == []


# -- set_cells and the table's structure -----------------------------------------------------------


def test_set_cells_from_keeps_its_own_record(toolbox, session):
    table = ok(call(toolbox, session, "word_insert_markdown", doc="d1", at="end",
                    markdown="| a | b | c |\n|---|---|---|\n| 1 | 2 | 3 |\n| 4 | 5 | 6 |\n| 7 | 8 | 9 |\n"
                             "| x | y | z |\n| total | | |")).data["blocks"]
    table = next(b for b in table if b.startswith("t:"))
    assert session.provenance("d1") == {}
    result = edit_table(toolbox, session, table, "set_cells", index=1,
                        **{"from": "d:scores", "from_columns": ["check", "rating"], "header": []})
    record = session.provenance("d1")[table]
    assert record["first_row"] == 1 and "header_row" not in record
    assert record["columns"] == result.data["from"]["columns"]


def test_set_cells_without_from_drops_only_the_columns_it_overwrites(toolbox, session):
    table = insert(toolbox, session, from_columns=["check", "rating", "due"]).data["id"]
    edit_table(toolbox, session, table, "insert_row")               # a totals row below
    edit_table(toolbox, session, table, "set_cells", cells=[{"row": 4, "col": 1, "text": "8.75"}])
    assert columns(session.provenance("d1")[table]) == [(0, "check"), (1, "rating"), (2, "due")]
    edit_table(toolbox, session, table, "set_cells", cells=[{"row": 2, "col": 1, "text": "5"}])
    assert columns(session.provenance("d1")[table]) == [(0, "check"), (2, "due")]
    edit_table(toolbox, session, table, "set_cells", cells=[{"row": 0, "col": 0, "text": "Name"},
                                                            {"row": 1, "col": 2, "text": "soon"}])
    assert session.provenance("d1") == {}                           # no column left
    assert items(saved(session)) == {}


def test_rows_above_shift_the_block_below_keep_it_inside_drop_it(toolbox, session):
    table = insert(toolbox, session, from_columns=["check", "rating"]).data["id"]
    edit_table(toolbox, session, table, "insert_row", index=3)      # below the block
    assert session.provenance("d1")[table]["first_row"] == 1
    # A row above the first renames the table: the record moves to the new id.
    result = edit_table(toolbox, session, table, "insert_row", index=0, side="before")
    renamed = result.data["id"]
    assert renamed != table
    kept = session.provenance("d1")
    assert list(kept) == [renamed]
    assert (kept[renamed]["header_row"], kept[renamed]["first_row"]) == (1, 2)
    result = edit_table(toolbox, session, renamed, "delete_row", index=0)
    assert result.data["id"] == table
    assert (session.provenance("d1")[table]["header_row"], session.provenance("d1")[table]["first_row"]) == (0, 1)
    edit_table(toolbox, session, table, "delete_row", index=4)      # the row added below
    assert table in session.provenance("d1")
    edit_table(toolbox, session, table, "insert_row", index=1)      # inside the block
    assert session.provenance("d1") == {}


def test_deleting_a_row_inside_the_block_drops_the_record(toolbox, session):
    table = insert(toolbox, session).data["id"]
    edit_table(toolbox, session, table, "delete_row", index=2)
    assert session.provenance("d1") == {}


def test_columns_shift_and_a_deleted_one_drops_its_entry(toolbox, session):
    table = insert(toolbox, session, from_columns=["check", "rating", "due"]).data["id"]
    edit_table(toolbox, session, table, "insert_column", index=0, side="after")
    assert columns(session.provenance("d1")[table]) == [(0, "check"), (2, "rating"), (3, "due")]
    edit_table(toolbox, session, table, "insert_column", index=0, side="before")
    assert columns(session.provenance("d1")[table]) == [(1, "check"), (3, "rating"), (4, "due")]
    edit_table(toolbox, session, table, "delete_column", index=3)
    assert columns(session.provenance("d1")[table]) == [(1, "check"), (3, "due")]
    edit_table(toolbox, session, table, "delete_column", index=0)
    assert columns(session.provenance("d1")[table]) == [(0, "check"), (2, "due")]


def test_merge_and_split_drop_the_record(toolbox, session):
    table = insert(toolbox, session).data["id"]
    edit_table(toolbox, session, table, "merge", first={"row": 1, "col": 0}, last={"row": 1, "col": 1})
    assert session.provenance("d1") == {}
    other = insert(toolbox, session).data["id"]
    edit_table(toolbox, session, other, "split", first={"row": 3, "col": 4})
    assert session.provenance("d1") == {}


# -- copies and deletes ----------------------------------------------------------------------------


def test_a_deleted_table_drops_its_entry_in_the_same_undo_step(toolbox, session):
    first = insert(toolbox, session).data["id"]
    second = insert(toolbox, session, from_columns=["check"]).data["id"]
    ok(call(toolbox, session, "word_delete", doc="d1", target=first))
    assert list(session.provenance("d1")) == [second]
    root = etree.fromstring(items(saved(session))["customXml/item1.xml"])
    assert [e.get("key") for e in root] == [second]                  # pruned, not just ignored
    ok(call(toolbox, session, "undo", doc="d1"))
    assert list(session.provenance("d1")) == [first, second]


def test_an_entry_whose_table_is_gone_is_ignored_on_read_and_pruned_by_the_next_edit(toolbox):
    # Our item, its table taken out as another program would.
    item = (f'<?xml version="1.0" encoding="UTF-8" standalone="yes"?>\n<prov:sources xmlns:prov="{NS}">'
            '<prov:source kind="table" key="t:0BADF00D">{"v":1,"source":"d:x"}</prov:source></prov:sources>').encode()
    props = ('<?xml version="1.0" encoding="UTF-8" standalone="yes"?>\n<ds:datastoreItem '
             'ds:itemID="{22222222-3333-4444-5555-666666666666}" '
             'xmlns:ds="http://schemas.openxmlformats.org/officeDocument/2006/customXml"><ds:schemaRefs>'
             f'<ds:schemaRef ds:uri="{NS}"/></ds:schemaRefs></ds:datastoreItem>').encode()
    raw = with_parts(word_docs.with_body('<w:p w14:paraId="1A2B3C4D" w14:textId="77777777"><w:r><w:t>Kept</w:t>'
                                         "</w:r></w:p>"),
                     {"customXml/item1.xml": item, "customXml/itemProps1.xml": props},
                     {"customXml/itemProps1.xml": "application/vnd.openxmlformats-officedocument.customXmlProperties+xml"},
                     [("customXml/item1.xml", REL_CUSTOM_XML + "Props", "itemProps1.xml"),
                      ("word/document.xml", REL_CUSTOM_XML, "../customXml/item1.xml")])
    session, d = reopened(toolbox, raw)
    assert session.provenance(d) == {} and kept(session, d).get("ignored") == 1
    assert items(saved(session, d))                                  # a save prunes nothing
    ok(call(toolbox, session, "word_set_text", doc=d, target="p:1A2B3C4D", text="Edited"))   # the next edit does
    assert items(saved(session, d)) == {}
    assert word_docs.validation(toolbox, session, d)["new"] == []


def test_a_copied_table_gets_no_record(toolbox, session):
    table = insert(toolbox, session).data["id"]
    target = session.open(word_docs.blank(), "copy.docx")
    result = ok(call(toolbox, session, "word_copy_from", doc=target, source_doc="d1", range=table, at="end"))
    copy = result.data["copied"][table]
    assert session.provenance(target) == {}
    assert items(saved(session, target)) == {}
    assert list(session.provenance("d1")) == [table]
    assert copy


# -- charts -----------------------------------------------------------------------------------------


def chart(toolbox, session, **arguments):
    paragraph = word_docs.paragraph_ids(toolbox, session, "d1")[0]
    return ok(call(toolbox, session, "edit_chart", doc="d1", target=paragraph, action="add",
                   chart_type="column", **{"from": "d:scores", "columns": ["check", "rating"], **arguments}))


def test_a_chart_from_a_source_is_keyed_by_its_drawing_id(toolbox, session):
    result = chart(toolbox, session)
    address = result.created[-1]
    assert address.startswith("d:")
    record = session.provenance("d1")[address]
    assert record["source"] == "d:scores" and record["series"][0]["name"] == "rating"
    other, d = reopened(toolbox, saved(session))
    assert other.provenance(d) == {address: record}
    entry = etree.fromstring(items(saved(session))["customXml/item1.xml"])[0]
    assert (entry.get("kind"), entry.get("key")) == ("chart", address)
    # add_category without from: every series' rows changed, the record goes.
    ok(call(toolbox, session, "edit_chart", doc="d1", target=address, action="add_category",
            text="Delta", values=[2]))
    assert session.provenance("d1") == {}
    ok(call(toolbox, session, "undo", doc="d1"))
    assert list(session.provenance("d1")) == [address]
    ok(call(toolbox, session, "word_delete", doc="d1", target=result.data["paragraph"]))
    assert session.provenance("d1") == {}                            # pruned with its paragraph


# -- tracked changes ----------------------------------------------------------------------------------


def track(toolbox, session):
    ok(call(toolbox, session, "word_set_tracking", doc="d1", on=True, author="Reviewer"))


def test_tracked_the_record_describes_the_document_with_changes_accepted(toolbox, session):
    table = insert(toolbox, session, from_columns=["check", "rating"]).data["id"]
    track(toolbox, session)
    # A tracked row above the block shifts it and renames the table; a tracked deletion
    # of a row there before leaves it in the file, but with changes accepted it is gone:
    # the block shifts back.  Accepting keeps the record.  (Round 51: the row inserted
    # above, one's own pending insertion, deleted goes outright, as in Word: the table has
    # its name back.)
    edit_table(toolbox, session, table, "insert_row", index=0, side="before")
    kept = session.provenance("d1")
    [renamed] = kept
    assert (kept[renamed]["header_row"], kept[renamed]["first_row"]) == (1, 2)
    edit_table(toolbox, session, renamed, "delete_row", index=0)     # one's own: gone
    assert list(session.provenance("d1")) == [table]
    assert (session.provenance("d1")[table]["header_row"], session.provenance("d1")[table]["first_row"]) == (0, 1)
    ok(call(toolbox, session, "word_changes", doc="d1", action="accept", all=True))
    assert session.provenance("d1")[table]["first_row"] == 1


def test_rejecting_a_change_in_a_recorded_table_drops_its_record(toolbox, session):
    table = insert(toolbox, session, from_columns=["check", "rating"]).data["id"]
    track(toolbox, session)
    edit_table(toolbox, session, table, "set_cells",
               **{"from": "d:scores", "from_columns": ["rating", "check"]})
    assert columns(session.provenance("d1")[table]) == [(0, "rating"), (1, "check")]
    ok(call(toolbox, session, "word_changes", doc="d1", action="reject", all=True))
    assert session.provenance("d1") == {}                            # its old text is back


def test_a_tracked_table_from_a_source_is_kept_and_rejecting_it_drops_it(toolbox, session):
    track(toolbox, session)
    table = insert(toolbox, session).data["id"]
    assert list(session.provenance("d1")) == [table]
    ok(call(toolbox, session, "word_changes", doc="d1", action="reject", all=True))
    assert session.provenance("d1") == {}
    assert items(saved(session)) == {}


# -- the document stays valid and renders ----------------------------------------------------------


def test_the_document_validates_renders_and_the_definitions_are_unchanged(toolbox, session):
    insert(toolbox, session)
    chart(toolbox, session)
    assert word_docs.validation(toolbox, session, "d1")["new"] == []
    drawn = ok(call(toolbox, session, "render", doc="d1", pages=[1], width=300))
    assert drawn.images and all(image.data.startswith(b"\x89PNG") for image in drawn.images)
    assert strict_problems(list(TOOLS)) == []
    failed(call(toolbox, session, "word_edit_table", doc="d1", table="t:7FFFFFFF", action="set_cells",
                cells=[]), "not_found")
    assert json.dumps(session.provenance("d1"))
