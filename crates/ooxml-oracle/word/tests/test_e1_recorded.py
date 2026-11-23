"""E1's built-in styles, default lists and pictures as the tools write them, held to what Word
wrote on applying them (``probes/e1_probe.py``, ``observations/e1-word.json``, Word 16.106 for
Mac, Dutch interface): the styles' kinds and references, the default bullet and number lists,
the size Word gives a picture."""

from __future__ import annotations

import re

import pytest
from recorded import Opened, observed, plain  # noqa: I001  (puts the probes on the path)

import e1_probe  # noqa: E402
import probe_paths  # noqa: E402

OBSERVED = observed("e1-word.json")
#: The observation's keys these tests compare with (index.py lists why the others are not).
READS = {"styles", "applied", "default_lists", "pictures"}
#: The styles every probe document has already (``e1_probe.minimal_styles``): not Word's to define.
OWN = {"Normal", "Default Paragraph Font", "Normal Table", "No List"}
#: The measured built-in styles' table the tools write definitions from.
TABLE = probe_paths.ROOT / "crates" / "ooxml-word-edit" / "data" / "builtin-styles.tsv"


def table() -> dict[str, dict]:
    out = {}
    for line in TABLE.read_text(encoding="utf-8").splitlines():
        if line.startswith("#"):
            continue
        name, kind, based, nxt, link = line.split("\t")[:5]
        refs = {k: v for k, v in (("basedOn", based), ("next", nxt), ("link", link)) if v}
        out[name] = {"type": kind, "refs": refs}
    return out


def test_the_built_in_styles_table_is_words():
    """Every style Word defined in the probe is in the tools' table, of Word's kind, pointing at
    the styles Word's points at."""
    ours = table()
    word = {name: entry for name, entry in OBSERVED["styles"].items() if name not in OWN}
    assert word
    assert {name: ours.get(name) for name in word} == word


def _styles(document: Opened) -> dict[str, dict]:
    """The document's styles by name: kind and references (by name)."""
    xml = plain(document.part("word/styles.xml"))
    found = re.findall(r'<w:style w:type="(\w+)"[^>]*w:styleId="([^"]*)"[^>]*>(.*?)</w:style>', xml)
    names = {sid: re.search(r'<w:name w:val="([^"]*)"', body).group(1) for _, sid, body in found}
    out = {}
    for kind, sid, body in found:
        refs = {tag: names.get(val, val) for tag, val in re.findall(r'<w:(basedOn|next|link) w:val="([^"]*)"', body)}
        out[names[sid]] = {"type": kind, "refs": refs}
    return out


def test_applying_each_built_in_style_writes_words_definition(box):
    """``applied``: each style the probe applied, applied through the tools to the same
    paragraph, is defined as Word defined it (kind, references); its linked style too."""
    document = Opened(box, e1_probe.probe_document())
    ids = document.paragraph_ids()
    paragraphs = len(e1_probe.PARAGRAPH_STYLES)
    for k, (_, name) in enumerate(OBSERVED["applied"]):
        if k < paragraphs:
            document.call("word_format", targets=[ids[k]], paragraph_style=name)
        else:
            document.call("word_format", targets=[ids[k]], character_style=name)
    ours = _styles(document)
    word = OBSERVED["styles"]
    applied = {name for _, name in OBSERVED["applied"]}
    linked = {word[name]["refs"]["link"] for name in applied if "link" in word[name]["refs"]}
    for name in sorted(applied | linked):
        assert ours.get(name) == word[name], name


@pytest.mark.parametrize("kind, fmt", [("bullet", "bullet"), ("number", "decimal")])
def test_a_default_list_is_words(box, kind, fmt):
    """``default_lists``: Word's default bullets and numbers on two paragraphs -- List
    Paragraph, and a single-level list of Word's level."""
    document = Opened(box, e1_probe.probe_document())
    ids = document.paragraph_ids()
    first = len(e1_probe.PARAGRAPH_STYLES) + len(e1_probe.CHARACTER_STYLES)
    document.call("word_lists", targets=ids[first:first + 2], action="add", kind=kind)
    body = plain(document.part())
    for block in re.findall(r"<w:p>.*?</w:p>", body)[first:first + 2]:
        assert '<w:pStyle w:val="ListParagraph"/>' in block
    word = next(a for a in OBSERVED["default_lists"]["abstracts"]
                if a["levels"][0]["pStyle"] is None and a["levels"][0]["numFmt"] == fmt)["levels"][0]
    numbering = plain(document.part("word/numbering.xml"))
    level = re.search(r'<w:lvl w:ilvl="0"[^>]*>(.*?)</w:lvl>', numbering).group(1)
    assert re.search(r'<w:numFmt w:val="(\w+)"/>', level).group(1) == word["numFmt"]
    assert re.search(r'<w:lvlText w:val="([^"]*)"/>', level).group(1) == word["lvlText"]
    ind = dict(re.findall(r'w:(left|hanging)="(\d+)"', re.search(r"<w:ind [^>]*/>", level).group(0)))
    assert ind == word["ind"]
    assert {p["style"] for p in OBSERVED["default_lists"]["paragraphs"]} == {"List Paragraph"}


#: The recorded no-density case disagrees with the tools and with docx-agent's ROADMAP (E1: "a
#: picture stating no density placed at 96 dpi"): Word's extent for 96 x 48 px is 609600 x
#: 304800 EMU, 144 dpi.  To measure again in Word before either side changes.
NO_DENSITY = ("the observation's no-density picture is 144 dpi (609600 EMU for 96 px), the tools place it at "
              "96 dpi as docx-agent's E1 notes say Word does: to remeasure")


@pytest.mark.parametrize("case, picture", [
    pytest.param("no-density 96x48 px", e1_probe.png(96, 48, (200, 30, 30)),
                 marks=pytest.mark.xfail(reason=NO_DENSITY, strict=True)),
    ("300 dpi 300x150 px", e1_probe.png(300, 150, (30, 30, 200), dpi=300)),
])
def test_a_picture_takes_the_size_word_gives_it(box, case, picture):
    """``pictures``: a picture without a density is 96 dpi; one with is its own."""
    document = Opened(box, e1_probe.probe_document())
    blob = document.session.add_blob(picture, "picture.png")
    target = document.paragraph_ids()[-1]
    document.call("word_drawings", action="insert_picture", image=blob, at=f"{target}@0", alt_text=case)
    extent = re.search(r'<wp:extent cx="(\d+)" cy="(\d+)"/>', document.part())
    assert {"cx": int(extent.group(1)), "cy": int(extent.group(2))} == OBSERVED["pictures"][case]
