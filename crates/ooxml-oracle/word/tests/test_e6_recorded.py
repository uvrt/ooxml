"""E6's authoring -- a new document, a template saved, Convert, content copied between documents
whose styles collide -- as the tools do it, held to what Word did (``probes/e6_probe.py``,
``observations/e6-word.json``, Word 16.106 for Mac, Dutch interface, metric)."""

from __future__ import annotations

import io
import re
import zipfile

import pytest
from recorded import Opened, observed, plain  # noqa: I001  (puts the probes on the path)

import e6_probe  # noqa: E402
import word_docs  # noqa: E402

OBSERVED = observed("e6-word.json")
#: The observation's cases these tests compare with (index.py lists why the others are not).
READS = {"new", "new-typed", "new-compared", "as-template", "convert-11", "convert-all-11", "convert-all-12",
         "convert-all-14", "convert-14-fe", "convert-12", "convert-14", "convert-14-sample", "copy-formatted",
         "copy-insert-file", "paste-default", "paste-destination", "paste-keep", "paste-merge",
         "copy-formatted-tracked"}
W = "{http://schemas.openxmlformats.org/wordprocessingml/2006/main}"
TEMPLATE_TYPE = "application/vnd.openxmlformats-officedocument.wordprocessingml.template.main+xml"


def parts(data: bytes) -> dict[str, bytes]:
    with zipfile.ZipFile(io.BytesIO(data)) as archive:
        return {name: archive.read(name) for name in archive.namelist()}


def new_like_words() -> bytes:
    """A new document made with the language and separators of the Word that made its own."""
    import ooxml_core

    locale = e6_probe.word_locale()
    return bytes(ooxml_core.new_document(page=("A4", False), language=locale.language,
                                         separators=(locale.decimal_separator, locale.list_separator),
                                         properties=(None, None, "2026-10-04T09:30:00Z")))


# -- a new document ----------------------------------------------------------------------------


def test_every_part_the_probe_compared_was_the_same():
    """The probe compared the new document with Word's part by part (ids, rsids, dates,
    authors and localised names normalised: Word's own parts are its Normal template and are
    not recorded): all the same, and those are the parts a new document has."""
    compared = OBSERVED["new-compared"]
    assert compared and set(compared.values()) == {"same"}, compared
    assert set(compared) == set(parts(new_like_words()))


def test_the_package_is_words():
    word = OBSERVED["new"]["package"]
    ours = parts(new_like_words())
    assert sorted(ours) == word["parts"]
    from lxml import etree

    types = etree.fromstring(ours["[Content_Types].xml"])
    overrides = sorted((n.get("PartName"), n.get("ContentType")) for n in types if n.get("PartName"))
    assert overrides == [tuple(pair) for pair in word["content_types"]]
    rels = etree.fromstring(ours["word/_rels/document.xml.rels"])
    assert sorted((n.get("Type").rsplit("/", 1)[1], n.get("Target")) for n in rels) == \
        [tuple(pair) for pair in word["relationships"]]


def test_the_settings_are_words_with_its_locale_and_language():
    """The settings are Word's but for the window's view, which Word writes and the tools
    leave out (a file without one opens in Print Layout)."""
    text = parts(new_like_words())["word/settings.xml"].decode("utf-8")
    text = re.sub(r' xmlns:\w+="[^"]*"', "", text)
    text = re.sub(r"<w:rsids>.*?</w:rsids>|<w15:docId [^>]*/>|<w14:docId [^>]*/>", "", text)
    ours = re.sub(r"^<\?xml[^>]*>\s*", "", text)
    assert ours == OBSERVED["new"]["settings"].replace('<w:view w:val="normal"/>', "")


def test_the_body_is_one_empty_paragraph_and_words_section():
    word = OBSERVED["new"]["body"]
    body = re.search(r"<w:body>(.*)</w:body>", plain(parts(new_like_words())["word/document.xml"].decode())).group(1)
    assert re.findall(r"<w:p/>|<w:sectPr>.*?</w:sectPr>", body) == word


def test_a_line_typed_into_a_new_document_is_words(box):
    """``new-typed``: a line written into the new document's paragraph is Word's typed line."""
    document = Opened(box, new_like_words())
    document.call("word_set_text", target=document.paragraph_ids()[0], text=e6_probe.TYPED)
    assert document.body() == OBSERVED["new-typed"]["body"][:1]


def test_saving_as_a_template_changes_the_main_content_type_only(box):
    """``as-template``: Word's Save As template changed the main part's content type."""
    word = OBSERVED["as-template"]["package"]["content_types"]
    assert ["/word/document.xml", TEMPLATE_TYPE] in word
    document = Opened(box, e6_probe.template("docx"))
    before = parts(document.data())
    document.call("save_document", name="mine.dotx", format="dotx")
    saved = parts(document.session.take_outputs()[-1].data)
    assert set(saved) == set(before)
    changed = {name for name in saved if saved[name] != before[name]}
    assert changed <= {"[Content_Types].xml", "docProps/app.xml"}
    assert TEMPLATE_TYPE.encode() in saved["[Content_Types].xml"]


# -- Convert -----------------------------------------------------------------------------------

#: Each convert case: the document Word converted.
CONVERTED = {
    "convert-11": lambda: e6_probe.legacy_document(),
    "convert-all-11": lambda: e6_probe.legacy_document(11, e6_probe.EVERY_COMPAT),
    "convert-all-12": lambda: e6_probe.legacy_document(12, e6_probe.EVERY_COMPAT),
    "convert-all-14": lambda: e6_probe.legacy_document(14, e6_probe.EVERY_COMPAT),
    "convert-14-fe": lambda: e6_probe.legacy_document(14, e6_probe.MODE14_COMPAT),
    "convert-12": lambda: (e6_probe.FIXTURES / "docx2svg" / "layout-sweep.docx").read_bytes(),
    "convert-14": lambda: (e6_probe.FIXTURES / "generated" / "mode14.docx").read_bytes(),
    "convert-14-sample": lambda: (e6_probe.FIXTURES / "samplelib" / "sample-simple.docx").read_bytes(),
}


def _compat(settings: str) -> list[tuple[str, ...]]:
    block = re.search(r"<w:compat>(.*?)</w:compat>", settings)
    return re.findall(r"<w:(\w+)/>|<w:compatSetting w:name=\"(\w+)\"[^>]*w:val=\"(\d+)\"", block.group(1)) \
        if block else []


@pytest.mark.parametrize("case", list(CONVERTED))
def test_convert_as_word_converts(box, case):
    document = Opened(box, CONVERTED[case]())
    document.call("word_template", action="upgrade_to_modern")
    ours = _compat(re.sub(r' xmlns:\w+="[^"]*"', "", document.part("word/settings.xml")))
    word = _compat(OBSERVED[case]["settings"])
    # useFELayout: Word drops it on any save of a document without East Asian languages.
    assert [c for c in ours if c[0] != "useFELayout"] == [c for c in word if c[0] != "useFELayout"]


# -- copying between documents -----------------------------------------------------------------


def copied(box, policy: str) -> Opened:
    """The probe's source copied whole to the end of its destination, by ``policy``."""
    document = Opened(box, e6_probe.copy_document("destination"), "destination.docx")
    document.session.open(e6_probe.copy_document("source"), "source.docx")
    source = [block for block, _ in word_docs.blocks(document.box, document.session, "d2")]
    document.call("word_copy_from", source_doc="d2", at="end", range=f"{source[0]}..{source[-1]}", styles=policy)
    return document


def styles_by_name(document: Opened) -> dict[str, str]:
    xml = plain(document.part("word/styles.xml"))
    return {re.search(r'<w:name w:val="([^"]*)"', s).group(1): s
            for s in re.findall(r"<w:style [^>]*>.*?</w:style>", xml)}


def test_use_destination_takes_its_styles_and_imports_the_rest_as_word(box):
    """``copy-formatted`` (and ``copy-insert-file``, ``paste-default``, ``paste-destination``,
    the same styles): the destination's Brand kept, Source Only imported with the source's
    default face folded in, a character style not."""
    word = OBSERVED["copy-formatted"]["styles"]
    for case in ("copy-insert-file", "paste-default", "paste-destination"):
        assert set(OBSERVED[case]["styles"]) == set(word), case
    styles = styles_by_name(copied(box, "use_destination"))
    assert styles["Brand"].count("7030A0") == 1 and word["Brand"].count("7030A0") == 1
    for face in ("Georgia", 'w:sz w:val="22"'):
        assert face in styles["Source Only"] and face in word["Source Only"]
    assert "Georgia" not in styles["Hyperlink"] and "Georgia" not in word["Hyperlink"]
    assert set(styles) == set(word) - {"annotation text", "annotation reference", "Tekst opmerking Char",
                                       "(docDefaults)"} | {"Hyperlink"}


def _word_blocks(case: str) -> dict[str, str]:
    """Word's copies, by their text's start: the observation's blocks after the destination's end."""
    out, seen = {}, False
    for block in OBSERVED[case]["body"]:
        text = "".join(re.findall(r"<w:t(?: [^>]*)?>([^<]*)</w:t>", block))
        if text.startswith("Destination end."):
            seen = True
        elif seen and text:
            out[text[:20]] = plain(block)
    return out


def _tags(xml: str, path: str) -> list[str]:
    """The child elements' names of the first element at ``path`` (``pPr``, ``pPr/rPr``, ``r/rPr``)."""
    found = xml
    for step in path.split("/"):
        match = re.search(rf"<w:{step}>(.*?)</w:{step}>", found)
        if not match:
            return []
        found = match.group(1)
    return sorted(set(re.findall(r"<w:(\w+)[ />]", re.sub(r"<w:rPr>.*?</w:rPr>", "", found) if path == "pPr"
                                  else found)))


@pytest.mark.parametrize("policy, case", [("keep_source", "paste-keep"), ("merge", "paste-merge")])
def test_keep_source_and_merge_write_the_look_direct_as_word(box, policy, case):
    """Keep Source Formatting and Merge Formatting: the copied paragraphs' properties, mark and
    runs carry the same properties Word wrote."""
    ours = {}
    for block in copied(box, policy).body():
        text = "".join(re.findall(r"<w:t(?: [^>]*)?>([^<]*)</w:t>", block))
        if text.startswith(("Source brand", "Source only")):
            ours[text[:20]] = block
    word = _word_blocks(case)
    assert ours
    for text, block in ours.items():
        for path in ("pPr/rPr", "r/rPr"):
            assert _tags(block, path) == _tags(word[text], path), (text, path)


# -- copying with tracking on ------------------------------------------------------------------

#: Where the tools' tracked copy is shaped otherwise than Word's.
TRACKED_COPY_DIFFERENCES = {
    "Source last paragrap": "Word inserts the copy before the destination's empty last paragraph, every "
                            "copied mark inserted; the tools fill that paragraph with the copy's last, so its "
                            "mark is the destination's own (not inserted) and Reject All leaves it with an "
                            "empty w:pPr",
}


def _shape(block: str) -> tuple[bool, bool]:
    """A copied block's revisions: whether each paragraph mark in it is inserted, and whether
    all its text is inside insertions."""
    paragraphs = re.findall(r"<w:p(?: [^>]*)?>.*?</w:p>", block)
    marks = all(re.match(r"<w:p(?: [^>]*)?><w:pPr>.*?<w:rPr>.*?<w:ins\b.*?</w:pPr>", p) for p in paragraphs)
    outside = re.sub(r"<w:ins\b[^>]*>.*?</w:ins>", "", block)
    return marks, not re.search(r"<w:t(?: [^>]*)?>[^<]+</w:t>", outside)


def _tracked_copies() -> list[str]:
    return list(_word_blocks("copy-formatted-tracked"))


@pytest.mark.parametrize("key", [pytest.param(k, marks=pytest.mark.xfail(reason=TRACKED_COPY_DIFFERENCES[k],
                                                                           strict=True))
                                 if k in TRACKED_COPY_DIFFERENCES else k for k in _tracked_copies()])
def test_a_tracked_copy_is_inserted_as_word_inserts_it(box, key):
    """``copy-formatted-tracked``: with tracking on, every copied paragraph's mark and text (a
    table's rows too) are insertions, as Word's."""
    document = Opened(box, e6_probe.copy_document("destination"), "destination.docx")
    document.session.open(e6_probe.copy_document("source"), "source.docx")
    document.track("Writer")
    source = [block for block, _ in word_docs.blocks(document.box, document.session, "d2")]
    document.call("word_copy_from", source_doc="d2", at="end", range=f"{source[0]}..{source[-1]}",
                  styles="use_destination")
    ours = {}
    seen = False
    for block in document.body():
        text = "".join(re.findall(r"<w:t(?: [^>]*)?>([^<]*)</w:t>", block))
        if text.startswith("Destination end."):
            seen = True
        elif seen and text:
            ours[text[:20]] = block
    assert _shape(ours[key]) == _shape(_word_blocks("copy-formatted-tracked")[key])
