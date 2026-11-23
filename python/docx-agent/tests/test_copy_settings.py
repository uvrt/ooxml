"""Round 42, R49: ``check(copies=[...])`` compares each copy's list items -- level, label,
numbering, indents -- and every paragraph's page settings with its reference's, as Word
applies them (the Rust session's check): a dash among round bullets, an item a
level up, a lost keep are each told, with the call that fixes it; copies that look the same
as their sample are not, however their XML says it.  Every document here is made up."""

from __future__ import annotations

import pytest

pytest.importorskip("ooxml_edit.tools.shared", reason="needs ooxml-edit (the shared tools)")

from ooxml_edit.tools import Toolbox  # noqa: E402

from layout_reads import block_ids, effective, written  # noqa: E402

from docx_agent.tools import FORMAT, GROUPS, TOOLS  # noqa: E402
import word_docs  # noqa: E402

pytestmark = pytest.mark.any_faces

W = "{http://schemas.openxmlformats.org/wordprocessingml/2006/main}"
#: The settings a copy's paragraphs are compared by (as Word applies them).
SETTINGS = ("list_level", "list_format", "list_numbering", "indent_left", "indent_first",
            "keep_with_next", "keep_lines", "page_break_before", "widow_control")

SAMPLE = ("# Part title\n\nDescribe the part here, in a few sentences.\n\n"
          "- First point of the part\n    - A detail under it\n- Second point of the part\n\n"
          "1. Step one of the part\n2. Step two of the part\n\nClose with a short summary of the part.\n")


@pytest.fixture
def box():
    with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS, workers=0) as toolbox:
        yield toolbox


def _ok(box, session, tool: str, /, **arguments):
    result = box.dispatch(session, tool, arguments)
    assert result.ok, (tool, arguments, result.error and result.error.to_json())
    return result


def _sample(box, session) -> tuple[str, list[str]]:
    sample = written(box, session, SAMPLE, "samples.docx")
    blocks = block_ids(box, session, sample)
    _ok(box, session, "word_format", doc=sample, targets=[blocks[1]], keep_with_next=True)   # stays with its list
    return sample, blocks


def _made(box, session, count: int, *, same_document: bool = False) -> tuple[str, str, str, list[list[str]]]:
    """A report of ``count`` copies of the sample: ``(report doc, sample doc, sample span, copies)``.
    ``same_document``: the sample is the report's own first chapter."""
    samples, blocks = _sample(box, session)
    reference = f"{blocks[0]}..{blocks[-1]}"
    if same_document:
        d1 = samples
    else:
        d1 = written(box, session, "# Report\n\nAn opening paragraph.", "report.docx")
    made = [_ok(box, session, "word_copy_from", doc=d1, source_doc=samples, range=reference,
                at="end").data["blocks"] for _ in range(count)]
    return d1, samples, reference, made


def _pairs(made: list[list[str]], samples: str, reference: str) -> list[dict]:
    return [{"blocks": f"{m[0]}..{m[-1]}", "reference": {"doc": samples, "block": reference}} for m in made]


def _restart_numbers(box, session, doc: str, made: list[list[str]]) -> None:
    """Each copy's numbered list starts at 1 (copied by style, it would go on counting)."""
    for blocks in made[1:]:
        _ok(box, session, "word_lists", doc=doc, targets=[blocks[5]], action="restart", start=1)


def _check(box, session, doc: str, made, samples, reference) -> dict:
    return _ok(box, session, "check", doc=doc, include=[], copies=_pairs(made, samples, reference)).data["copies"]


def _settings(item: dict) -> dict[str, dict]:
    return {d["property"]: d for d in item["differences"] if d["property"] in SETTINGS}


def test_dashes_a_wrong_level_and_a_lost_keep_are_each_found_with_their_fix(box):
    session = box.session()
    d1, samples, reference, made = _made(box, session, 4)
    _restart_numbers(box, session, d1, made)
    assert _check(box, session, d1, made, samples, reference)["follow"] == 4
    # Copy 1: its first point made a list of its own, of dashes.
    first = made[1][2]
    _ok(box, session, "word_lists", doc=d1, targets=[first], action="add", kind="bullet")
    _ok(box, session, "word_lists", doc=d1, targets=[first], action="format", level=0, number_format="bullet",
        label="–")
    # Copy 2: the detail a level up.  Copy 3: the description's keep lost.
    _ok(box, session, "word_lists", doc=d1, targets=[made[2][3]], action="level", level=0)
    _ok(box, session, "word_format", doc=d1, targets=[made[3][1]], keep_with_next=False)
    found = _check(box, session, d1, made, samples, reference)
    items = found["items"]
    assert [item["follows"] for item in items] == [True, False, False, False]
    dashes = _settings(items[1])["list_format"]
    assert dashes["value"] == "bullet –" and dashes["reference"] == "bullet •"
    assert dashes["paragraphs"] == [first] and dashes["kind"] == "format"
    level = _settings(items[2])["list_level"]
    assert (level["value"], level["reference"], level["paragraphs"]) == (0, 1, [made[2][3]])
    assert level["fix"] == {"tool": "word_lists", "calls": [{"targets": [made[2][3]], "action": "level",
                                                             "level": 1}]}
    keep = _settings(items[3])["keep_with_next"]
    assert (keep["value"], keep["reference"]) == (False, True)
    assert keep["fix"] == {"tool": "word_format", "calls": [{"targets": [made[3][1]], "keep_with_next": True}]}
    # One difference per cause: neither the dashes' nor the level's indents are told apart.
    assert set(_settings(items[1])) == {"list_format"} and set(_settings(items[2])) == {"list_level"}
    # Rule 5: the list fixes of every copy in one list of calls; follow_sample for the rest.
    assert found["list_fix"]["tool"] == "word_lists"
    assert {c["action"] for c in found["list_fix"]["calls"]} == {"format", "level"}
    assert found["fix"]["calls"] == [{"sample": {"block": _blocks_of(box, session, samples, reference)[1],
                                                 "doc": samples}, "copies": [made[3][1]]}]
    # The fixes make every copy follow again.
    for call in found["list_fix"]["calls"]:
        _ok(box, session, "word_lists", doc=d1, **call)
    for call in keep["fix"]["calls"]:
        _ok(box, session, "word_format", doc=d1, **call)
    again = _check(box, session, d1, made, samples, reference)
    assert again["follow"] == 4, [i["differences"] for i in again["items"]]


def _blocks_of(box, session, doc: str, span: str) -> list[str]:
    """The body's blocks from the span's first to its last."""
    first, _, last = span.partition("..")
    listed = block_ids(box, session, doc)
    return listed[listed.index(first):listed.index(last or first) + 1]


def _num_id(box, session, doc: str, paragraph: str):
    """The numbering instance a paragraph's list item is in (its own or its style's)."""
    found = _ok(box, session, "word_inspect", doc=doc, target=paragraph).data["blocks"][0].get("list")
    return None if found is None else found["num_id"]


def _word_bullets(box) -> object:
    """Word's own bullet definition (``w:abstractNum``), as ``word_lists`` ``add`` writes it
    into a new blank document."""
    session = box.session()
    d = session.open(word_docs.blank(), "bullets.docx")
    first = block_ids(box, session, d)[0]
    _ok(box, session, "word_set_text", doc=d, target=first, text="Item")
    _ok(box, session, "word_lists", doc=d, targets=[first], action="add", kind="bullet")
    return next(word_docs.tree(word_docs.saved(session, d), "word/numbering.xml").iter(W + "abstractNum"))


def _in_a_list_of_its_own(box, session, report: str, samples: str, paragraph: str, level: int):
    """``report`` with ``paragraph`` in a new list instance (``w:num``) over Word's own bullet
    definition at ``level``, as Word's Bullets button starts one; the samples and the report
    opened in a new session in the order they were (their ids and paraIds kept):
    ``(session, report, samples)``."""
    import copy

    from lxml import etree

    data = word_docs.saved(session, report)
    root, numbering = word_docs.tree(data), word_docs.tree(data, "word/numbering.xml")
    abstract = copy.deepcopy(_word_bullets(box))
    abstract_id = str(max(int(a.get(W + "abstractNumId")) for a in numbering.iter(W + "abstractNum")) + 1)
    abstract.set(W + "abstractNumId", abstract_id)
    nsid = abstract.find(W + "nsid")
    if nsid is not None:
        nsid.set(W + "val", "5C0FFEE1")
    last_abstract = list(numbering.iter(W + "abstractNum"))[-1]
    last_abstract.addnext(abstract)
    num_id = str(max(int(n.get(W + "numId")) for n in numbering.iter(W + "num")) + 1)
    num = etree.SubElement(numbering, W + "num", {W + "numId": num_id})
    etree.SubElement(num, W + "abstractNumId", {W + "val": abstract_id})
    properties = word_docs.find(root, paragraph).find(W + "pPr")
    numbered = properties.find(W + "numPr")
    if numbered is not None:
        properties.remove(numbered)
    numbered = etree.fromstring(f'<w:numPr xmlns:w="{W[1:-1]}"><w:ilvl w:val="{level}"/>'
                                f'<w:numId w:val="{num_id}"/></w:numPr>')
    style = properties.find(W + "pStyle")
    (style.addnext if style is not None else lambda node: properties.insert(0, node))(numbered)

    def xml(node) -> bytes:
        return etree.tostring(node, xml_declaration=True, encoding="UTF-8", standalone=True)

    changed = word_docs.replaced(data, {"word/document.xml": xml(root), "word/numbering.xml": xml(numbering)})
    fresh = box.session()
    reopened_samples = fresh.open(word_docs.saved(session, samples), "samples.docx")
    reopened = fresh.open(changed, "report.docx")
    assert (reopened_samples, reopened) == (samples, report)
    return fresh, reopened, reopened_samples


def test_copies_that_look_the_same_do_not_differ(box):
    """The same look stated otherwise: a keep the heading's style gives, stated again; a
    list item's indent stated as its list gives it; the bullets restarted (a new instance,
    the same bullets); an item in another list that draws the same."""
    session = box.session()
    d1, samples, reference, made = _made(box, session, 3)
    _restart_numbers(box, session, d1, made)
    heading, point, detail = made[0][0], made[0][2], made[0][3]
    _ok(box, session, "word_format", doc=d1, targets=[heading], keep_with_next=True)
    indent = effective(box, session, d1, point)["indent"]
    _ok(box, session, "word_format", doc=d1, targets=[point], indent_left=indent["left"],
        indent_first=indent["first_line"] - indent["hanging"])
    _ok(box, session, "word_lists", doc=d1, targets=[made[1][2]], action="restart", start=1)
    # Copy 2's detail: a new list of Word's own bullets, the same "o" at the same place (an
    # instance of its own, which no tool makes: written in the package, the documents reopened).
    session, d1, samples = _in_a_list_of_its_own(box, session, d1, samples, made[2][3], level=1)
    found = _check(box, session, d1, made, samples, reference)
    assert found["follow"] == 3, [item["differences"] for item in found["items"]]
    assert "list_fix" not in found and "fix" not in found
    # Their XML differs all the same.
    assert _num_id(box, session, d1, made[2][3]) != _num_id(box, session, d1, detail)
    assert _num_id(box, session, d1, made[1][2]) != _num_id(box, session, d1, made[0][2])
    states = [word_docs.element(session, d1, i).find(f"{W}pPr/{W}{tag}") is not None
              for i, tag in ((heading, "keepNext"), (made[1][0], "keepNext"), (point, "ind"), (made[1][2], "ind"))]
    assert states == [True, False, True, False]


def test_a_numbered_list_going_on_from_the_chapter_before_is_told(box):
    """Copies of a numbered list in its style count on from the copy before them, as Word
    numbers them: told as where each starts, fixed by a restart."""
    session = box.session()
    d1, samples, reference, made = _made(box, session, 3)
    found = _check(box, session, d1, made, samples, reference)
    numbering = [_settings(item).get("list_numbering") for item in found["items"]]
    assert numbering[0] is None
    assert [(n["value"], n["reference"]) for n in numbering[1:]] == [("starts at 3", "starts at 1"),
                                                                     ("starts at 5", "starts at 1")]
    assert numbering[1]["fix"] == {"tool": "word_lists", "calls": [
        {"targets": [made[1][5]], "action": "restart", "start": 1}]}
    for call in found["list_fix"]["calls"]:
        _ok(box, session, "word_lists", doc=d1, **call)
    assert _check(box, session, d1, made, samples, reference)["follow"] == 3


def test_a_kept_comparison_follows_the_numbers_before_it(box):
    """A comparison is kept by what both sides hold -- and by the numbers the copy's lists
    show, which the items before the copy decide: an item added before it compares it again."""
    session = box.session()
    d1, samples, reference, made = _made(box, session, 2)
    _restart_numbers(box, session, d1, made)
    pairs = _pairs(made[1:], samples, reference)
    first = _ok(box, session, "check", doc=d1, include=[], copies=pairs).data["copies"]
    assert first["follow"] == 1
    kept = _ok(box, session, "check", doc=d1, include=[], copies=pairs).data["copies"]
    assert kept["kept"] == 1
    # The second copy's numbers restarted from the first's: undo it, its list goes on again.
    _ok(box, session, "word_lists", doc=d1, targets=[made[1][5]], action="continue")
    again = _ok(box, session, "check", doc=d1, include=[], copies=pairs).data["copies"]
    assert again["kept"] == 0 and again["follow"] == 0


def test_the_sample_in_the_same_document(box):
    """The sample the report's own chapter: its copies' lists by style go on from it."""
    session = box.session()
    d1, _, reference, made = _made(box, session, 1, same_document=True)
    found = _ok(box, session, "check", doc=d1, include=[], copies=[
        {"blocks": f"{made[0][0]}..{made[0][-1]}", "reference": {"block": reference}}]).data["copies"]
    numbering = _settings(found["items"][0])["list_numbering"]
    assert (numbering["value"], numbering["reference"]) == ("starts at 3", "starts at 1")


def test_clear_is_the_fix_where_the_style_gives_the_reference(box):
    """Rule 6: a keep the copy turned off directly, where its style keeps it: cleared."""
    session = box.session()
    d1, samples, reference, made = _made(box, session, 1)
    _ok(box, session, "word_format", doc=d1, targets=[made[0][0]], keep_with_next=False)   # the heading's
    found = _check(box, session, d1, made, samples, reference)
    keep = _settings(found["items"][0])["keep_with_next"]
    assert keep["fix"] == {"tool": "word_format", "calls": [{"targets": [made[0][0]], "clear": ["keep_with_next"]}]}


def test_a_sample_in_the_same_document_is_followed_with_like(box):
    """A reference in the same document: each difference's fix is ``word_format`` ``like``
    the reference's paragraph where taking its look changes nothing else compared; where a
    numbered list starts stays ``word_lists`` (``like`` would count on from the sample).
    The fixes make the copy follow; a reference in another document keeps the explicit
    calls (the tests above)."""
    session = box.session()
    d1, _, reference, made = _made(box, session, 1, same_document=True)
    sample = _blocks_of(box, session, d1, reference)
    copy = made[0]
    _ok(box, session, "word_lists", doc=d1, targets=[copy[2]], action="add", kind="bullet")
    _ok(box, session, "word_lists", doc=d1, targets=[copy[2]], action="format", level=0, number_format="bullet",
        label="–")
    _ok(box, session, "word_lists", doc=d1, targets=[copy[3]], action="level", level=0)
    _ok(box, session, "word_format", doc=d1, targets=[copy[1]], keep_with_next=False)
    pairs = [{"blocks": f"{copy[0]}..{copy[-1]}", "reference": {"block": reference}}]
    found = _ok(box, session, "check", doc=d1, include=[], copies=pairs).data["copies"]
    settings = _settings(found["items"][0])
    assert settings["list_format"]["fix"] == {"tool": "word_format", "calls": [{"targets": [copy[2]],
                                                                                "like": sample[2]}]}
    assert settings["list_level"]["fix"] == {"tool": "word_format", "calls": [{"targets": [copy[3]],
                                                                               "like": sample[3]}]}
    assert settings["keep_with_next"]["fix"] == {"tool": "word_format", "calls": [{"targets": [copy[1]],
                                                                                   "like": sample[1]}]}
    numbering = settings["list_numbering"]
    assert numbering["fix"]["tool"] == "word_lists" and numbering["fix"]["calls"][0]["action"] == "restart"
    # list_fix holds only word_lists calls: the restart.
    assert [c["action"] for c in found["list_fix"]["calls"]] == ["restart"]
    for difference in found["items"][0]["differences"]:
        for call in difference["fix"]["calls"]:
            _ok(box, session, difference["fix"]["tool"], doc=d1, **call)
    again = _ok(box, session, "check", doc=d1, include=[], copies=pairs).data["copies"]
    assert again["follow"] == 1, again["items"][0]["differences"]
