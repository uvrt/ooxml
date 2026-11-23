"""E4's structure -- sections, headers and footers, notes, fields, the table of contents --
as the tools write it, held to what Word wrote for the same edits (``probes/e4_probe.py``,
``observations/e4-word.json``, Word 16.106 for Mac, Dutch interface)."""

from __future__ import annotations

import re

from recorded import Opened, observed, plain, section_properties  # noqa: I001  (puts the probes on the path)

import e4_probe  # noqa: E402

OBSERVED = observed("e4-word.json")
#: The observation's cases these tests compare with (index.py lists why the others are not).
READS = {"breaks", "setup", "headers", "headers2", "notes", "fields", "tocfield", "tocstyles", "removal",
         "tracked", "tracked-removal"}

#: Dutch Word's ids for the built-in styles the probes used, and English Word's.
STYLE_IDS = {"Koptekst": "Header", "Voettekst": "Footer", "Voetnoottekst": "FootnoteText",
             "Voetnootmarkering": "FootnoteReference", "Eindnoottekst": "EndnoteText",
             "Eindnootmarkering": "EndnoteReference", "Bijschrift": "Caption", "Kop1": "Heading1",
             "Kop2": "Heading2", "Kop3": "Heading3", **{f"Inhopg{n}": f"TOC{n}" for n in range(1, 10)}}


def english(xml: str) -> str:
    """``xml``, :func:`plain`, with Dutch Word's style ids in English Word's."""
    xml = plain(xml)
    for dutch, name in STYLE_IDS.items():
        xml = xml.replace(f'w:val="{dutch}"', f'w:val="{name}"')
    return xml


def word_sections(case: str) -> list[str]:
    """Each section's properties Word wrote in ``case``, its header and footer references aside
    (Word's AppleScript makes empty headers and footers on asking for one)."""
    found = section_properties("".join(OBSERVED[case]["blocks"]))
    return [re.sub(r"<w:(header|footer)Reference [^>]*/>", "", s) for s in found]


def sections(document: Opened) -> list[str]:
    return [s["id"] for s in document.call("word_sections", action="list").data]


def headings(box) -> Opened:
    """The headings probe document with its headings styled, as the probe's script styled them."""
    document = Opened(box, e4_probe.headings_document())
    ids = document.paragraph_ids()
    for k, level in e4_probe.heading_levels().items():
        document.call("word_format", targets=[ids[k - 1]], paragraph_style=f"heading {level}")
    return document


# -- sections ----------------------------------------------------------------------------------


def test_section_breaks_of_each_kind_split_the_section_as_word_does(box):
    document = Opened(box, e4_probe.plain_document())
    ids = document.paragraph_ids()
    document.call("word_sections", action="insert_break", after=ids[8], start="evenPage")
    document.call("word_sections", action="insert_break", before=ids[5], start="continuous")
    document.call("word_sections", action="insert_break", after=ids[2], start="nextPage")
    assert section_properties(document.part()) == word_sections("breaks")


def test_page_setup_is_written_as_word_writes_it(box):
    """``setup``: the second section's orientation, top and left margins and columns, as far as
    the tools set them (gutter, column spacing and separator, vertical alignment, line numbers
    and page numbering are Word's alone: index.py)."""
    document = Opened(box, e4_probe.sectioned_document())
    second = sections(document)[1]
    document.call("word_sections", action="set", section=second, orientation="landscape",
                  margins={"top": 54, "left": 90}, columns=2)
    ours = section_properties(document.part())[1]
    word = word_sections("setup")[1]
    for element in ("pgSz",):
        assert re.search(rf"<w:{element} [^>]*/>", ours).group(0) == re.search(rf"<w:{element} [^>]*/>", word).group(0)
    margins = re.search(r"<w:pgMar [^>]*/>", word).group(0)
    for side in ("top", "left", "right", "bottom", "header", "footer"):
        attribute = re.compile(rf' w:{side}="\d+"')
        assert attribute.search(ours).group(0) == attribute.search(margins).group(0), side
    assert re.search(r'<w:cols [^>]*w:num="2"', ours) and re.search(r'<w:cols [^>]*w:num="2"', word)


# -- headers and footers -----------------------------------------------------------------------


def test_a_header_part_is_words_and_its_references_are_in_words_order(box):
    """``headers``: default, first and even headers and a footer of each kind in section 1."""
    document = Opened(box, e4_probe.sectioned_document())
    first = sections(document)[0]
    for kind, text in (("default", "Default header one"), ("first", "First header one"), ("even", "Even header one")):
        document.call("word_headers_footers", section=first, which="header", action="set", type=kind, text=text)
    for kind in ("even", "default", "first"):
        document.call("word_headers_footers", section=first, which="footer", action="set", type=kind,
                      text="Footer one" if kind == "default" else "")
    ours = section_properties(document.part())[0]
    order = re.findall(r'<w:(header|footer)Reference w:type="(\w+)"', ours)
    word = re.findall(r'<w:(header|footer)Reference w:type="(\w+)"', section_properties(
        "".join(OBSERVED["headers"]["blocks"]))[0])
    assert order == word
    assert "<w:titlePg/>" in ours
    assert '<w:evenAndOddHeaders/>' in document.part("word/settings.xml")
    parts = {name: english(document.part(name)) for name in document.names() if re.match(r"word/header\d+\.xml$", name)}
    observed_parts = {english(v) for k, v in OBSERVED["headers"].items() if k.startswith("word/header")}
    assert next(p for p in parts.values() if "Default header one" in p) in observed_parts


def test_a_page_number_in_a_footer_is_words_field(box):
    """``headers2``: Word's PAGE field in the default footer, its cached number aside."""
    document = Opened(box, e4_probe.sectioned_document())
    first = sections(document)[0]
    made = document.call("word_headers_footers", section=first, which="footer", action="set", type="default",
                         text="Footer one", page_number="after_text").data
    ours = english(document.part(f"word/{made['story']}.xml"))
    word = next(english(v) for k, v in OBSERVED["headers2"].items() if k.startswith("word/footer") and " PAGE " in v)

    def field(xml: str) -> str:
        found = re.search(r'<w:r>(?:<w:rPr>.*?</w:rPr>)?<w:fldChar w:fldCharType="begin"/>.*?'
                          r'<w:fldChar w:fldCharType="end"/></w:r>', xml)
        return re.sub(r"<w:t>\d+</w:t>", "<w:t>n</w:t>", found.group(0))

    assert field(ours) == field(word)


def test_unlinking_a_footer_copies_the_one_before(box):
    """``headers2``: the second section's footer, unlinked without being written, starts as a
    copy of the first's (Word's footer4 is its footer2)."""
    document = Opened(box, e4_probe.sectioned_document())
    first, second, _ = sections(document)
    source = document.call("word_headers_footers", section=first, which="footer", action="set", type="default",
                           text="Footer one", page_number="after_text").data["story"]
    made = document.call("word_headers_footers", section=second, which="footer", action="unlink",
                         type="default").data["story"]
    assert made != source
    assert english(document.part(f"word/{made}.xml")) == english(document.part(f"word/{source}.xml"))
    word = OBSERVED["headers2"]
    assert english(word["word/footer4.xml"]) == english(word["word/footer2.xml"])


# -- notes -------------------------------------------------------------------------------------


def test_a_footnote_is_written_as_word_writes_one(box):
    document = Opened(box, e4_probe.sectioned_document())
    target = document.paragraph_ids()[1]
    document.call("word_notes", action="insert", kind="footnote", at=target, text="No Reference")
    ours = re.search(r'<w:footnote w:id="1">.*?</w:footnote>', english(document.part("word/footnotes.xml"))).group(0)
    word = re.search(r'<w:footnote w:id="1">.*?</w:footnote>', english(OBSERVED["notes"]["word/footnotes.xml"])).group(0)
    # Word gives the note's text and mark the Dutch interface's language; that aside, alike.
    word = word.replace('<w:rPr><w:lang w:val="nl-NL"/></w:rPr>', "")
    assert re.sub(r"<w:t>[^<]*</w:t>", "<w:t/>", ours) == re.sub(r"<w:t>[^<]*</w:t>", "<w:t/>", word)
    separators = re.findall(r'<w:footnote w:type="\w+" w:id="-?\d+">.*?</w:footnote>',
                            english(document.part("word/footnotes.xml")))
    assert separators == re.findall(r'<w:footnote w:type="\w+" w:id="-?\d+">.*?</w:footnote>',
                                    english(OBSERVED["notes"]["word/footnotes.xml"]))
    reference = re.search(r"<w:r><w:rPr><w:rStyle w:val=\"FootnoteReference\"/></w:rPr><w:footnoteReference "
                          r'w:id="1"/></w:r>', english(document.part()))
    assert reference and reference.group(0) in english("".join(OBSERVED["notes"]["blocks"]))


# -- fields ------------------------------------------------------------------------------------


def test_a_date_and_a_caption_are_words_field_forms(box):
    document = headings(box)
    ids = document.paragraph_ids()
    last = len(ids)
    document.call("word_fields", action="insert_field", at=f"{ids[last - 1]}@0",
                  instruction='DATE \\@ "d MMMM yyyy"')
    document.call("word_fields", action="insert_caption", after=ids[last - 3], label="Figure", text=": a figure")
    observed_blocks = english("".join(OBSERVED["fields"]["blocks"]))
    ours = english(document.part())
    word_date = re.search(r'<w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText xml:space="preserve"> '
                          r'DATE.*?w:fldCharType="end"/></w:r>', observed_blocks).group(0)
    date = re.search(r'<w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText xml:space="preserve"> '
                     r'DATE.*?w:fldCharType="end"/></w:r>', ours).group(0)
    shown = re.compile(r'(<w:fldChar w:fldCharType="separate"/></w:r>).*?(<w:r><w:fldChar w:fldCharType="end")')
    assert shown.sub(r"\1\2", date) == shown.sub(r"\1\2", word_date)
    word_caption = re.search(r'<w:r><w:t xml:space="preserve">Figure </w:t></w:r><w:fldSimple.*?</w:fldSimple>'
                             r'<w:r><w:t>: a figure</w:t></w:r>', observed_blocks).group(0)
    assert word_caption in ours
    caption = next(p for p in re.findall(r"<w:p>.*?</w:p>", ours) if word_caption in p)
    assert '<w:pStyle w:val="Caption"/>' in caption


# -- the table of contents ---------------------------------------------------------------------


def _entries(blocks: list[str]) -> list[tuple[str, ...]]:
    return [tuple(re.findall(r"<w:t>([^<]*)</w:t>", b)) for b in blocks if 'w:val="TOC' in b]


def test_the_table_of_contents_is_words_and_its_page_numbers_are_words(box):
    """``tocfield``: Word's own table of contents of the probe, updated by Word, on a page of
    another text width -- every entry, every page number."""
    document = headings(box)
    document.call("word_sections", action="set", section="s:body", margins={"left": 90, "right": 36})
    document.call("word_fields", action="insert_toc", before=document.paragraph_ids()[0])
    ours = [b for b in document.body() if 'w:val="TOC' in english(b)]
    word = [english(b).replace(" \\* MERGEFORMAT", "") for b in OBSERVED["tocfield"]["blocks"]
            if "Inhopg" in b]
    assert [english(b) for b in ours] == word


def test_the_toc_styles_are_words(box):
    """``tocstyles``: a table of contents over nine levels, its entries in Word's toc 1-9."""
    document = Opened(box, e4_probe.nine_document())
    ids = document.paragraph_ids()
    for level in range(1, 10):
        document.call("word_format", targets=[ids[2 * level - 1]], paragraph_style=f"heading {level}")
    document.call("word_fields", action="insert_toc", before=ids[0], levels="1-9")
    ours = [english(b) for b in document.body()]
    assert _entries(ours) == _entries([english(b) for b in OBSERVED["tocstyles"]["blocks"]])


# -- removals and tracked edits ----------------------------------------------------------------


def test_removing_a_break_keeps_the_next_sections_properties(box):
    """``removal``: the break between sections 1 and 2 deleted -- the text joins section 2's
    properties (its left margin)."""
    document = Opened(box, e4_probe.sectioned_document())
    first, second, _ = sections(document)
    document.call("word_sections", action="set", section=first, margins={"left": 36})
    document.call("word_sections", action="set", section=second, margins={"left": 108})
    document.call("word_sections", action="remove_break", section=first)
    ours = word_sections_of(document)
    word = word_sections("removal")
    assert len(ours) == len(word) == 2
    assert [re.search(r' w:left="\d+"', s).group(0) for s in ours] == \
        [re.search(r' w:left="\d+"', s).group(0) for s in word]
    texts = [re.sub(r"<[^>]+>", "", b)[:24] for b in OBSERVED["removal"]["blocks"][:-1]]
    assert [re.sub(r"<[^>]+>", "", b)[:24] for b in document.body()] == texts


def word_sections_of(document: Opened) -> list[str]:
    return [re.sub(r"<w:(header|footer)Reference [^>]*/>", "", s) for s in section_properties(document.part())]


def test_a_tracked_break_removal_deletes_its_mark_keeping_the_section(box):
    """``tracked-removal``: Word deletes the break's paragraph mark as a revision, the section's
    properties staying on it until accepted."""
    document = Opened(box, e4_probe.sectioned_document())
    first = sections(document)[0]
    document.track()
    document.call("word_sections", action="remove_break", section=first)
    word = next(plain(b) for b in OBSERVED["tracked-removal"]["blocks"] if "<w:del " in b)
    ours = next(b for b in document.body() if "<w:del " in b)
    mark = re.compile(r"<w:pPr><w:rPr><w:del [^>]*/></w:rPr><w:sectPr>")
    assert mark.search(word) and mark.search(ours)
    # Word drops a next-page break's type on saving (the default); the rest is the same.

    def alike(xml: str) -> str:
        return re.sub(r"<w:del [^>]*/>", "<w:del/>", xml.replace('<w:type w:val="nextPage"/>', ""))

    assert alike(ours) == alike(word)


def test_a_tracked_break_is_an_inserted_mark_holding_the_section(box):
    """``tracked``: Word's tracked section break before Chapter 2 is the heading's copy, its
    mark inserted, holding the section's properties."""
    document = headings(box)
    chapter2 = [k for k, level in e4_probe.heading_levels().items() if level == 1][1]
    document.track()
    document.call("word_sections", action="insert_break", before=document.paragraph_ids()[chapter2 - 1],
                  start="nextPage")
    word = next(english(b) for b in OBSERVED["tracked"]["blocks"] if "<w:sectPr>" in b and "<w:ins " in b
                and 'w:val="Heading1"' in english(b))
    ours = next(english(b) for b in document.body() if "<w:sectPr>" in b)
    inserted = re.compile(r"<w:pPr><w:pStyle w:val=\"Heading1\"/><w:rPr><w:ins [^>]*/></w:rPr><w:sectPr>")
    assert inserted.search(word) and inserted.search(ours)
