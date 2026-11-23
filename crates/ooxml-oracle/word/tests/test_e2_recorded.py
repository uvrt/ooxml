"""E2's notes, tables and tracked typing as the tools write them, held to what Word wrote
(``probes/e2_probe.py``, ``observations/e2-word.json``, Word 16.106 for Mac, Dutch interface):
the parts a first footnote brings, a new table's form, two paragraphs added after a heading
with tracking on and Word's Accept All of them."""

from __future__ import annotations

import re

from recorded import Opened, observed, plain  # noqa: I001  (puts the probes on the path)

import e2_probe  # noqa: E402

OBSERVED = observed("e2-word.json")
#: The observation's keys these tests compare with (index.py lists why the others are not).
READS = {"footnote", "table", "typing_at_the_end"}
STYLE_IDS = {"Voetnootmarkering": "FootnoteReference", "Tabelraster": "TableGrid", "Kop1": "Heading1"}


def english(xml: str) -> str:
    xml = plain(xml)
    for dutch, name in STYLE_IDS.items():
        xml = xml.replace(f'w:val="{dutch}"', f'w:val="{name}"')
    return xml


def test_a_first_footnote_brings_words_parts(box):
    """``footnote``: the notes parts, their content types and relationships, the settings'
    separators and the reference's run."""
    word = OBSERVED["footnote"]
    document = Opened(box, e2_probe.probe_document())
    third = document.paragraph_ids()[2]
    document.call("word_notes", action="insert", kind="footnote", at=third, text="A note.")
    names = document.names()
    assert {f"word/{n}" for n in ("footnotes.xml", "endnotes.xml")} <= names
    assert set(word["parts"]) <= names
    types = document.part("[Content_Types].xml")
    for part in word["content_types"]:
        assert f'PartName="{part}"' in types
    rels = document.part("word/_rels/document.xml.rels")
    for kind in word["relationships"]:
        assert f'/relationships/{kind}"' in rels
    settings = plain(document.part("word/settings.xml"))
    for element in word["settings"]:
        assert element in settings
    for part in ("footnotes.xml", "endnotes.xml"):
        separators = re.compile(r'<w:(?:foot|end)note w:type="\w+" w:id="-?\d+">.*?</w:(?:foot|end)note>')
        assert separators.findall(plain(document.part(f"word/{part}"))) == separators.findall(plain(word[part]))
    paragraph = next(b for b in document.body() if "footnoteReference" in b)
    assert english(paragraph) == english(word["reference"])


def test_a_new_table_is_words(box):
    """``table``: a table of two rows and three columns across the text's width, in Table Grid --
    Word's grid, cell widths, style and width.  Word's Insert Table also fixes the layout and
    turns the header row's look on; the tools' table comes from Markdown, whose layout fits
    the text and whose look follows the rows written (no Insert Table of its own)."""
    word = OBSERVED["table"]
    document = Opened(box, e2_probe.probe_document())
    second = document.paragraph_ids()[1]
    document.call("word_insert_markdown", at=f"before:{second}", markdown="| | | |\n|---|---|---|\n| | | |\n")
    table = next(b for b in document.body() if b.startswith("<w:tbl>"))
    grid = [int(w) for w in re.findall(r'<w:gridCol w:w="(\d+)"/>', table)]
    assert grid == word["grid"] and sum(grid) == word["text_width"]
    assert re.search(r"<w:tcPr>.*?</w:tcPr>", table).group(0) == word["cell"]
    ours = english(re.search(r"<w:tblPr>.*?</w:tblPr>", table).group(0))
    for element in ("tblStyle", "tblW"):
        pattern = re.compile(rf"<w:{element} [^>]*/>")
        assert pattern.search(ours).group(0) == pattern.search(english(word["properties"])).group(0)


def test_paragraphs_added_after_a_heading_are_words_once_accepted(box):
    """``typing_at_the_end``: two Normal paragraphs added, tracked, after the last paragraph (a
    heading): accepting them gives Word's accepted body; rejecting them, Word's text."""
    word = OBSERVED["typing_at_the_end"]
    for review in ("accept", "reject"):
        document = Opened(box, e2_probe.typed_document())
        heading = document.paragraph_ids()[1]
        document.track()
        document.call("word_insert_markdown", at=f"after:{heading}", markdown="First new.\n\nSecond new.\n")
        document.call("word_changes", action=review, all=True)
        ours = english("".join(document.body()))
        if review == "accept":
            assert ours == english(word["accept"])
        else:
            assert re.findall(r"<w:t>([^<]*)</w:t>", ours) == re.findall(r"<w:t>([^<]*)</w:t>", word["reject"])
