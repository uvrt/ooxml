"""Content controls as Word reads them (lane fu-tools; Word's answers in
``crates/ooxml-oracle/word/observations/e5-word.json``): a data-bound plain text control whose
cached text is stale shows its custom XML node from the moment the document opens -- read,
edited and saved so, as Word opens it -- and a date control takes a date typed in its own
picture (tool rule 1) or refuses with both forms as the way out (rule 2).  Made-up documents
only."""

from __future__ import annotations

import io
import zipfile

import pytest

from docx_agent.tools import FORMAT, GROUPS, TOOLS
from ooxml_edit.tools import Toolbox

import word_docs

STORE = "{0B5C6A3E-7D21-4F0A-9E4B-1C2D3E4F5A6B}"
_R = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/"
_NODE = '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>\n<root xmlns="urn:made-up"><name>{}</name></root>'
_PROPS = ('<?xml version="1.0" encoding="UTF-8" standalone="no"?>\n'
          f'<ds:datastoreItem ds:itemID="{STORE}" '
          'xmlns:ds="http://schemas.openxmlformats.org/officeDocument/2006/customXml"><ds:schemaRefs/>'
          "</ds:datastoreItem>")
_BINDING = (f'<w:dataBinding w:prefixMappings="xmlns:ns0=\'urn:made-up\'" w:xpath="/ns0:root[1]/ns0:name[1]" '
            f'w:storeItemID="{STORE}"/>')


@pytest.fixture(scope="module")
def box():
    with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS, workers=0) as toolbox:
        yield toolbox


def _bound(content: str, node: str) -> bytes:
    """A document with one bound plain text control holding ``content`` (its runs), its
    node holding ``node``."""
    body = (f'<w:p><w:r><w:t xml:space="preserve">Name: </w:t></w:r><w:sdt><w:sdtPr><w:id w:val="701"/>'
            f'<w:tag w:val="Name"/>{_BINDING}<w:text/></w:sdtPr><w:sdtContent>{content}</w:sdtContent></w:sdt>'
            '<w:r><w:t xml:space="preserve"> end.</w:t></w:r></w:p>' + word_docs.paragraph("After."))
    data = word_docs.with_body(body)
    out = io.BytesIO()
    with zipfile.ZipFile(io.BytesIO(data)) as source, zipfile.ZipFile(out, "w", zipfile.ZIP_DEFLATED) as target:
        for info in source.infolist():
            text = source.read(info.filename)
            if info.filename == "[Content_Types].xml":
                text = text.replace(b"</Types>", b'<Override PartName="/customXml/itemProps1.xml" ContentType="'
                                    b'application/vnd.openxmlformats-officedocument.customXmlProperties+xml"/>'
                                    b"</Types>")
            elif info.filename == "word/_rels/document.xml.rels":
                text = text.replace(b"</Relationships>", f'<Relationship Id="rIdMadeUpXml" Type="{_R}customXml" '
                                    'Target="../customXml/item1.xml"/></Relationships>'.encode())
            target.writestr(info, text)
        target.writestr("customXml/item1.xml", _NODE.format(node))
        target.writestr("customXml/itemProps1.xml", _PROPS)
        target.writestr("customXml/_rels/item1.xml.rels",
                        '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>\n<Relationships xmlns="http://'
                        f'schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" '
                        f'Type="{_R}customXmlProps" Target="itemProps1.xml"/></Relationships>')
    return out.getvalue()


def _control(box, session, doc) -> dict:
    return word_docs.call(box, session, "word_controls", doc=doc, action="list").data[0]


def test_a_stale_bound_control_opens_showing_its_node(box):
    session = box.session()
    doc = session.open(_bound("<w:r><w:rPr><w:b/></w:rPr><w:t>Stale cache</w:t></w:r>", "From the node"), "b.docx")
    assert _control(box, session, doc)["value"] == "From the node"
    saved = word_docs.saved(session, doc)
    document = word_docs.part(saved, "word/document.xml")
    assert "<w:r><w:rPr><w:b/></w:rPr><w:t>From the node</w:t></w:r></w:sdtContent>" in document
    assert "Stale cache" not in document
    assert "<name>From the node</name>" in word_docs.part(saved, "customXml/item1.xml")
    # how the document opens, not an edit: nothing to undo
    undo = box.dispatch(session, "undo", {"doc": doc}).to_json()
    assert undo["version"] == 0 and undo["error"]["message"] == f"nothing to undo in {doc}"


def test_a_bound_control_whose_text_is_its_nodes_opens_untouched(box):
    data = _bound("<w:r><w:t>Same</w:t></w:r>", "Same")
    session = box.session()
    doc = session.open(data, "same.docx")
    with zipfile.ZipFile(io.BytesIO(data)) as before, zipfile.ZipFile(io.BytesIO(word_docs.saved(session, doc))) as after:
        assert before.read("word/document.xml") == after.read("word/document.xml")


@pytest.mark.parametrize("content", [
    '<w:ins w:id="9" w:author="A" w:date="2026-10-04T00:00:00Z"><w:r><w:t>Stale</w:t></w:r></w:ins>',
    '<w:commentRangeStart w:id="3"/><w:r><w:t>Stale</w:t></w:r><w:commentRangeEnd w:id="3"/>',
    "<w:r><w:t>Stale</w:t><w:br/><w:t>cache</w:t></w:r>",
])
def test_a_stale_bound_control_holding_more_than_text_is_left(box, content):
    """Only the measured form is refreshed: runs of text; a revision, a mark or a break in the
    content leaves it as it is."""
    session = box.session()
    doc = session.open(_bound(content, "From the node"), "kept.docx")
    assert "From the node" not in word_docs.part(word_docs.saved(session, doc), "word/document.xml")


def _date_control(box):
    body = ('<w:p><w:r><w:t xml:space="preserve">Due: </w:t></w:r><w:sdt><w:sdtPr><w:id w:val="702"/>'
            '<w:date w:fullDate="2026-10-04T00:00:00Z"><w:dateFormat w:val="dd/MM/yyyy"/><w:lid w:val="en-GB"/>'
            '<w:storeMappedDataAs w:val="dateTime"/><w:calendar w:val="gregorian"/></w:date></w:sdtPr>'
            "<w:sdtContent><w:r><w:t>04/10/2026</w:t></w:r></w:sdtContent></w:sdt></w:p>")
    session = box.session()
    doc = session.open(word_docs.with_body(body), "date.docx")
    return session, doc


@pytest.mark.parametrize("typed", ["05/10/2026", "5/10/2026", "2026-10-05"])
def test_a_date_control_takes_iso_or_its_own_picture(box, typed):
    session, doc = _date_control(box)
    result = word_docs.call(box, session, "word_controls", doc=doc, action="fill", control="cc:702", value=typed)
    assert _control(box, session, doc)["value"] == "2026-10-05"
    assert "<w:t>05/10/2026</w:t>" in word_docs.part(word_docs.saved(session, doc), "word/document.xml")
    notes = result.to_json()["warnings"]
    assert notes == ([] if typed.startswith("2026") else [f"'{typed}' read in cc:702's picture 'dd/MM/yyyy' as 2026-10-05"])


@pytest.mark.parametrize("typed", ["10/5/26", "31/09/2026", "next week"])
def test_a_date_neither_form_reads_is_refused_with_both_forms(box, typed):
    session, doc = _date_control(box)
    result = box.dispatch(session, "word_controls", {"doc": doc, "action": "fill", "control": "cc:702", "value": typed})
    error = result.to_json()["error"]
    assert error["code"] == "refused"
    assert "YYYY-MM-DD" in error["message"] and "'dd/MM/yyyy'" in error["message"]
    assert error["try_call"]["tool"] == "word_controls"
