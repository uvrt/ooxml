"""Spacing around punctuation by the text's language (round 35, R21:
measured in ``crates/ooxml-oracle/word/observations/typography-word.json``), through the toolbox.

Text a call writes in French gets a no-break space (U+00A0) before ``: ; ! ?`` and inside
guillemets -- an ordinary space it wrote made U+00A0, a missing one put in -- and nothing
else changes; ``check``'s ``punctuation_spacing`` reports what is left, with fixes.  The
documents are made up here.
"""

from __future__ import annotations

import datetime
import io
import json
import zipfile
from pathlib import Path

import pytest

from ooxml_edit.tools import Toolbox

from docx_agent.tools import GROUPS, TOOLS, docx_format

import word_docs
from conftest import OBSERVATIONS  # noqa: E402

ROOT = Path(__file__).resolve().parent.parent
OBSERVED = json.loads((OBSERVATIONS / "typography-word.json").read_text(encoding="utf-8"))
NB = "{U+00A0}"
NBSP = "\u00a0"
_W = "{http://schemas.openxmlformats.org/wordprocessingml/2006/main}"
CLOCK = lambda: datetime.datetime(2026, 11, 5, 12, 0)  # noqa: E731


def french(*texts: str, language: str = "fr-FR") -> bytes:
    """A blank document in ``language`` holding ``texts`` as written (no call spaced them),
    a paragraph each (paraIds ``2B000001`` on)."""
    body = "".join(f'<w:p w14:paraId="{0x2B000001 + k:08X}" w14:textId="77777777"><w:r>'
                   f'<w:t xml:space="preserve">{text}</w:t></w:r></w:p>' for k, text in enumerate(texts or ("",)))
    return word_docs.with_body(body, word_docs.blank(language=language))


def ids_of(data: bytes) -> list[str]:
    return [f"p:{p.get(word_docs.W14_PARA)}" for p in word_docs.tree(data).iter(_W + "p")]


_OBJECTS = {_W + "footnoteReference", _W + "endnoteReference", _W + "drawing", _W + "pict",
            "{http://schemas.openxmlformats.org/markup-compatibility/2006}AlternateContent"}


def _text(paragraph) -> str:
    """A paragraph's own text (deletions out), an object -- a note's mark, a drawing -- as
    U+FFFC; a text box's paragraphs are their own."""
    out = []

    def walk(node, deleted: bool) -> None:
        for child in node:
            if child.tag in _OBJECTS:
                out.append("\ufffc")
            elif child.tag == _W + "t" and not deleted:
                out.append(child.text or "")
            elif child.tag == _W + "tab" and node.tag == _W + "r":
                out.append("\t")
            elif child.tag == _W + "br":
                out.append("\v")
            elif child.tag not in (_W + "pPr", _W + "rPr", _W + "p"):
                walk(child, deleted or child.tag in (_W + "del", _W + "moveFrom"))

    walk(paragraph, False)
    return "".join(out)


def texts(session, story: str = "word/document.xml") -> list[str]:
    return [_text(p) for p in word_docs.tree(word_docs.saved(session, "d1"), story).iter(_W + "p")]


@pytest.fixture(scope="module")
def box():
    with Toolbox(TOOLS, formats=[docx_format()], groups=GROUPS, workers=0) as toolbox:
        yield toolbox


def _session(box, data: bytes | None = None):
    session = box.session(clock=CLOCK)
    session.open(data if data is not None else french("Début."), "rapport.docx")
    return session


def _ok(box, session, name, **arguments):
    result = box.dispatch(session, name, arguments)
    assert result.ok, json.dumps(result.to_json(), default=str)[:800]
    return result


def _spaced_note(result) -> list[str]:
    return [w for w in result.warnings if "no-break" in w]


def written(box, text: str, language: str = "fr-FR"):
    """``text`` set into a blank document in ``language``: the session and the call's result."""
    data = french("Avant.", language=language)
    session = _session(box, data)
    result = _ok(box, session, "word_set_text", doc="d1", target=ids_of(data)[0], text=text)
    return session, result


# -- the measurements the rules stand on -------------------------------------------------------


def test_the_rules_are_the_measured_ones(box):
    proofing = OBSERVED["proofing"]
    # French (and fr alone, fr-CH): no space before a mark, or inside guillemets, is flagged;
    # an ordinary space, U+00A0 and U+202F all pass.
    for lang in ("fr", "fr-FR", "fr-CH"):
        flagged = proofing[lang]
        assert "Bonjour: le monde." in flagged and "Vraiment? Oui." in flagged
        assert "Il a dit «bonjour» hier." in flagged
        assert not any(t.startswith(("Bonjour :", f"Bonjour{NB}", "Bonjour{U+202F}")) for t in flagged)
        session, _ = written(box, "Vraiment? Il a dit «bonjour».", lang)
        assert texts(session) == [f"Vraiment{NBSP}? Il a dit «{NBSP}bonjour{NBSP}»."]
    # Canadian French: the guillemets only.
    assert proofing["fr-CA"] == ["Il a dit «bonjour» hier."]
    session, _ = written(box, "Vraiment? Il a dit «bonjour».", "fr-CA")
    assert texts(session) == [f"Vraiment? Il a dit «{NBSP}bonjour{NBSP}»."]
    # English, German, Dutch, Spanish, Italian, Portuguese flag any space before a mark; a
    # call writes their text as given.
    for lang in ("en-US", "en-GB", "de-DE", "nl-NL", "es-ES", "it-IT", "pt-PT"):
        flagged = proofing[lang]
        assert "Vraiment ? Oui." in flagged and f"Vraiment{NB}? Oui." in flagged
        assert "Vraiment? Oui." not in flagged
        session, _ = written(box, "Vraiment? Oui.", lang)
        assert texts(session) == ["Vraiment? Oui."]
    # Line breaks: an ordinary space lets a mark start a line; a no-break one never does.
    assert OBSERVED["line_breaks"]["ordinary"]["lines_starting_with_a_mark"]
    assert not OBSERVED["line_breaks"]["U+00A0"]["lines_starting_with_a_mark"]
    assert not OBSERVED["line_breaks"]["U+202F"]["lines_starting_with_a_mark"]
    # What Word writes: U+00A0 inside guillemets.
    assert OBSERVED["autoformat"]["fr-FR"][0] == f"Il a dit «{NB}bonjour{NB}» hier."


def test_marks_between_words_are_left_alone_as_their_fix_is_not_a_space(box):
    """Word flags 12:30 and x?y in French too; URLs, paths and (?) it does not."""
    edges = OBSERVED["edges_fr"]
    assert "Rendez-vous à 12:30 demain." in edges["flagged"]
    assert "Voir http://example.com/page pour plus." in edges["passed"]
    session, _ = written(box, "Rendez-vous à 12:30, voir http://example.com/page ou C:\\dossier (?) x?y v2:beta.")
    assert NBSP not in texts(session)[0]


# -- writing -----------------------------------------------------------------------------------


def test_french_text_gets_no_break_spaces_where_written(box):
    session, result = written(box, "Bonjour : le monde! Pourquoi? « oui » et «non». Quoi?! Fin :")
    assert texts(session) == [f"Bonjour{NBSP}: le monde{NBSP}! Pourquoi{NBSP}? «{NBSP}oui{NBSP}» et "
                              f"«{NBSP}non{NBSP}». Quoi{NBSP}?! Fin{NBSP}:"]
    assert _spaced_note(result) and _spaced_note(result)[0].startswith("9 space(s)")


def test_a_space_already_there_is_not_changed_by_a_later_edit(box):
    data = french("Avant : après")
    session = _session(box, data)
    _ok(box, session, "word_insert_text", doc="d1", at=f"{ids_of(data)[0]}@13", text=" ; suite")
    assert texts(session) == [f"Avant : après{NBSP}; suite"]   # the old ' :' stays


def test_other_languages_are_written_as_given(box):
    session, result = written(box, "Note : this ! and « that »", "en-US")
    assert texts(session) == ["Note : this ! and « that »"]
    assert not _spaced_note(result)


def test_canadian_french_gets_the_guillemets_only(box):
    session, _ = written(box, "Il a dit «oui» : non!", "fr-CA")
    assert texts(session) == [f"Il a dit «{NBSP}oui{NBSP}» : non!"]


def test_belgian_french_takes_frenchs_rule(box):
    session, _ = written(box, "Vraiment?", "fr-BE")
    assert texts(session) == [f"Vraiment{NBSP}?"]


def test_the_language_is_the_runs_through_its_styles(box):
    def normal_in_french(xml: str) -> str:
        from lxml import etree

        root = etree.fromstring(xml.encode("utf-8"))
        normal = next(s for s in root.iter(_W + "style") if s.get(_W + "styleId") == "Normal")
        rpr = normal.find(_W + "rPr")
        if rpr is None:
            rpr = etree.SubElement(normal, _W + "rPr")
        etree.SubElement(rpr, _W + "lang").set(_W + "val", "fr-FR")
        return etree.tostring(root, xml_declaration=True, encoding="UTF-8", standalone=True).decode()

    data = word_docs.replaced(french("Un.", "Deux.", language="en-US"), {"word/styles.xml": normal_in_french})
    session = _session(box, data)
    _ok(box, session, "word_set_text", doc="d1", target=ids_of(data)[0], text="Un : deux")
    assert texts(session)[0] == f"Un{NBSP}: deux"


def test_undo_takes_the_text_and_its_spacing_together(box):
    session, _ = written(box, "Début : fin")
    assert texts(session) == [f"Début{NBSP}: fin"]
    _ok(box, session, "undo", doc="d1")
    assert texts(session) == ["Avant."]


def test_off_writes_the_text_as_given():
    with Toolbox(TOOLS, formats=[docx_format(typography=False)], groups=GROUPS, workers=0) as toolbox:
        session, result = written(toolbox, "Bonjour : le monde!")
        assert texts(session) == ["Bonjour : le monde!"]
        assert not _spaced_note(result)


# -- the tools ---------------------------------------------------------------------------------


def test_every_writing_tool_spaces_what_it_writes(box):
    session = _session(box)
    first = word_docs.paragraph_ids(box, session, "d1")[0]
    result = _ok(box, session, "word_insert_markdown", doc="d1", at="end", markdown=(
        "# Titre: un\n\nBonjour : le **monde**!\n\n- point: un\n\n| Col: A | B |\n|---|---|\n| x ? | «y» |\n\n"
        "Code `a ? b : c` ici!\n\n```\nx = a ? b : c;\n```\n\nNote[^1].\n\n[^1]: Voir: ici!\n"))
    assert _spaced_note(result) and _spaced_note(result)[0].startswith("11 space(s)")
    _ok(box, session, "word_set_text", doc="d1", target=first, text="Début : fin")
    _ok(box, session, "word_insert_text", doc="d1", find="fin", where="after", text=" encore?")
    _ok(box, session, "replace_text", doc="d1", find="encore", replace="toujours!", expect="one")
    table = "t:" + word_docs.rows_of(word_docs.tables(session, "d1")[0])[0].get(word_docs.W14_PARA)
    _ok(box, session, "word_edit_table", doc="d1", table=table, action="set_cells",
        cells=[{"row": 1, "col": 1, "text": "Cellule: oui\nAutre?"}])
    _ok(box, session, "word_comments", doc="d1", action="add", author="Relecteur",
        items=[{"target": first, "text": "Pourquoi? Voir: ici"}])
    _ok(box, session, "word_headers_footers", doc="d1", section="s:body", which="header", action="set",
        text="En-tête: rapport")
    _ok(box, session, "word_notes", doc="d1", action="insert", kind="endnote", find="Début", text="Fin: note!")
    _ok(box, session, "word_drawings", doc="d1", action="insert_text_box", find="Début", text="Boîte: texte!",
        width=200, height=50)
    data = word_docs.saved(session, "d1")
    body = texts(session)
    assert body[0] == f"Début\ufffc\ufffc{NBSP}: fin toujours{NBSP}!{NBSP}?"
    assert f"Titre{NBSP}: un" in body and f"Bonjour{NBSP}: le monde{NBSP}!" in body
    assert f"point{NBSP}: un" in body and f"Col{NBSP}: A" in body and f"x{NBSP}?" in body
    assert f"Cellule{NBSP}: oui" in body and f"Autre{NBSP}?" in body
    assert f"Code a ? b : c ici{NBSP}!" in body and "x = a ? b : c;" in body     # code as written
    assert f"Boîte{NBSP}: texte{NBSP}!" in body
    assert texts(session, "word/comments.xml") == [f"Pourquoi{NBSP}? Voir{NBSP}: ici"]
    with zipfile.ZipFile(io.BytesIO(data)) as package:
        headers = [n for n in package.namelist() if n.startswith("word/header")]
    assert f"En-tête{NBSP}: rapport" in [t for n in headers for t in texts(session, n)]
    assert texts(session, "word/footnotes.xml")[-1].endswith(f"Voir{NBSP}: ici{NBSP}!")
    assert texts(session, "word/endnotes.xml")[-1].endswith(f"Fin{NBSP}: note{NBSP}!")
    assert word_docs.validation(box, session, "d1")["new"] == []


def test_a_control_filled_is_spaced(box):
    session = _session(box)
    made = _ok(box, session, "word_controls", doc="d1", action="insert", find="Début", type="text", title="Titre")
    result = _ok(box, session, "word_controls", doc="d1", action="fill", control=made.data["id"],
                 value="Valeur: oui!")
    assert texts(session) == [f"DébutValeur{NBSP}: oui{NBSP}!."]
    assert _spaced_note(result)


def test_tracked_writing_is_spaced_inside_the_insertion(box):
    session = _session(box)
    _ok(box, session, "word_set_tracking", doc="d1", on=True, author="Relecteur")
    first = word_docs.paragraph_ids(box, session, "d1")[0]
    _ok(box, session, "word_set_text", doc="d1", target=first, text="Début : fin!")
    assert texts(session) == [f"Début{NBSP}: fin{NBSP}!"]
    assert word_docs.validation(box, session, "d1")["new"] == []
    _ok(box, session, "word_changes", doc="d1", action="reject", all=True)
    assert texts(session) == ["Début."]


def test_writing_what_is_there_already_is_a_no_op(box):
    data = french(f"Début{NBSP}: fin{NBSP}!")
    session = _session(box, data)
    result = _ok(box, session, "word_set_text", doc="d1", target=ids_of(data)[0], text=f"Début{NBSP}: fin{NBSP}!")
    assert not result.changed and not _spaced_note(result)


# -- check -------------------------------------------------------------------------------------


def spacing(box, session, **arguments) -> list[dict]:
    return _ok(box, session, "check", doc="d1", include=["layout"], **arguments).data["layout"]["punctuation_spacing"]


def test_problems_list_what_is_left_with_fixes_that_clear_them(box):
    data = french("Bonjour : le monde! « oui » et «non».", "Rien à voir.", "Une : deux : trois")
    ids = ids_of(data)
    session = _session(box, data)
    report = spacing(box, session)
    assert [e["para"] for e in report] == [ids[0], ids[2]]
    assert [m["kind"] for m in report[0]["marks"]] == ["breakable", "missing", "breakable", "breakable",
                                                       "missing", "missing"]
    assert report[0]["language"] == "fr-FR"
    for entry in report:
        assert entry["fix"]["tool"] == "replace_text"
        for call in entry["fix"]["calls"]:
            _ok(box, session, "replace_text", doc="d1", **call)
    assert spacing(box, session) == []
    assert texts(session)[0] == (f"Bonjour{NBSP}: le monde{NBSP}! «{NBSP}oui{NBSP}» et «{NBSP}non{NBSP}».")


def test_check_reports_the_spaces_english_does_not_want_without_deleting_them(box):
    """Round 35, R24: French spacing in English text is most often French under the wrong
    tag -- an entry of its own, whose fix sets the language (tests/test_language.py)."""
    found = spacing(box, _session(box, french("Note : this! Really ?", "Fine: yes.", language="en-US")))
    assert len(found) == 1 and found[0]["language"] == "en-US" and found[0]["kind"] == "language_mismatch"
    assert [m["kind"] for m in found[0]["marks"]] == ["space_before", "space_before"]
    assert found[0]["fix"]["tool"] == "word_styles"


def test_check_is_scoped_to_a_range(box):
    data = french("Un : deux", "Trois : quatre")
    second = ids_of(data)[1]
    report = _ok(box, _session(box, data), "check", doc="d1", range=[second]).data["layout"]
    assert [e["para"] for e in report["punctuation_spacing"]] == [second]


def test_a_paragraph_lists_ten_marks_and_counts_the_rest(box):
    entry = spacing(box, _session(box, french(" ".join(f"mot{k} :" for k in range(14)))))[0]
    assert len(entry["marks"]) == 10 and entry["marks_left_out"] == 4
