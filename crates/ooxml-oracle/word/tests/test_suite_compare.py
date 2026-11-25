"""The Word suite's comparison rules for what depends on the machine running Word, not on the
tools (``word/suite/compare.py``), and the field results ``answers.kept_facts`` lists for them:
each rule holds where it is meant to and nowhere else."""

from __future__ import annotations

import io
import sys
import zipfile
from pathlib import Path

SUITE = Path(__file__).resolve().parents[1] / "suite"
sys.path.insert(0, str(SUITE))

import answers  # noqa: E402
import compare  # noqa: E402


# -- fields: compared by keyword only -----------------------------------------------------------


def test_a_clock_field_is_compared_by_its_keyword():
    for keyword in ("DATE", "TIME", "PRINTDATE", "SAVEDATE", "CREATEDATE"):
        assert compare.fields([[keyword, "17 November 2026"]], [[keyword, "24 November 2026"]]) == []


def test_a_user_field_is_compared_by_its_keyword():
    for keyword in ("AUTHOR", "USERNAME", "USERINITIALS"):
        assert compare.fields([[keyword, "Someone"]], [[keyword, "Evert Koks"]]) == []


def test_other_fields_are_still_compared_by_their_results():
    assert compare.fields([["PAGE", "1"]], [["PAGE", "2"]])
    assert compare.fields([["TITLE", "Someone"]], [["TITLE", "Evert Koks"]])
    assert compare.fields([["NUMPAGES", "3"]], [["NUMPAGES", "4"]])


def test_a_keyword_only_field_still_needs_the_same_keyword():
    assert compare.fields([["AUTHOR", "Someone"]], [["DATE", "Someone"]])


# -- an empty table of contents -----------------------------------------------------------------


def test_an_empty_toc_in_word_s_interface_language_is_the_tools_placeholder():
    ours = [["TOC", compare.TOOLS_EMPTY_TOC]]
    assert compare.fields(ours, [["TOC", "Geen inhoudsopgavegegevens gevonden."]]) == []
    assert compare.fields(ours, [["TOC", "No table of contents entries found."]]) == []


def test_a_toc_with_entries_is_still_compared():
    assert compare.fields([["TOC", compare.TOOLS_EMPTY_TOC]], [["TOC", "1 Introduction 1"]])
    assert compare.fields([["TOC", "1 Introduction 1"]], [["TOC", "Geen inhoudsopgavegegevens gevonden."]])
    assert compare.fields([["TOC", "1 Introduction 1"]], [["TOC", "1 Introduction 2"]])


# -- kept: a paragraph's text with its keyword-only fields' results masked ------------------------


def _kept(ours_text, ours_fields, word_text, word_fields):
    def facts(text, fields):
        entry = {"id": "1A2B3C4D", "text": text}
        if fields:
            entry["fields"] = fields
        return {"document": {"paragraphs": [entry]}}

    return compare.kept(facts(ours_text, ours_fields), facts(word_text, word_fields))


def test_kept_text_takes_a_clock_field_by_its_keyword():
    assert _kept("Date: 17 November 2026", [["DATE", "17 November 2026"]],
                 "Date: 24 November 2026", [["DATE", "24 November 2026"]]) == []


def test_kept_text_outside_the_field_is_still_compared():
    assert _kept("Date: 17 November 2026", [["DATE", "17 November 2026"]],
                 "Datum: 24 November 2026", [["DATE", "24 November 2026"]])


def test_kept_text_is_compared_whole_without_the_field_on_both_sides():
    assert _kept("Date: 17 November 2026", [["DATE", "17 November 2026"]], "Date: 24 November 2026", [])


# -- kept_facts: the field results it lists ---------------------------------------------------------

_HEAD = ('<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
         '<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>')


def _docx(body: str) -> bytes:
    buffer = io.BytesIO()
    with zipfile.ZipFile(buffer, "w") as archive:
        archive.writestr("word/document.xml", _HEAD + body + "</w:body></w:document>")
    return buffer.getvalue()


def _complex(instr: str, result: str) -> str:
    return (f'<w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText> {instr} </w:instrText></w:r>'
            f'<w:r><w:fldChar w:fldCharType="separate"/></w:r><w:r><w:t>{result}</w:t></w:r>'
            '<w:r><w:fldChar w:fldCharType="end"/></w:r>')


def test_kept_facts_lists_a_complex_clock_field_s_result():
    data = _docx(f'<w:p><w:r><w:t>Date: </w:t></w:r>{_complex("DATE \\@ &quot;d MMMM yyyy&quot;", "17 November 2026")}</w:p>')
    paragraph = answers.kept_facts(data)["document"]["paragraphs"][0]
    assert paragraph["text"] == "Date: 17 November 2026"
    assert paragraph["fields"] == [["DATE", "17 November 2026"]]


def test_kept_facts_lists_a_simple_user_field_s_result():
    data = _docx('<w:p><w:r><w:t>By </w:t></w:r><w:fldSimple w:instr=" AUTHOR "><w:r><w:t>Someone</w:t></w:r>'
                 '</w:fldSimple></w:p>')
    assert answers.kept_facts(data)["document"]["paragraphs"][0]["fields"] == [["AUTHOR", "Someone"]]


def test_kept_facts_lists_no_other_field():
    data = _docx(f'<w:p><w:r><w:t>Page </w:t></w:r>{_complex("PAGE", "1")}</w:p>')
    assert "fields" not in answers.kept_facts(data)["document"]["paragraphs"][0]


def test_kept_facts_takes_the_innermost_field():
    nested = ('<w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText> IF 1 = 1 </w:instrText></w:r>'
              + _complex("DATE", "17 November 2026") +
              '<w:r><w:fldChar w:fldCharType="separate"/></w:r><w:r><w:t>yes</w:t></w:r>'
              '<w:r><w:fldChar w:fldCharType="end"/></w:r>')
    data = _docx(f"<w:p>{nested}</w:p>")
    assert answers.kept_facts(data)["document"]["paragraphs"][0]["fields"] == [["DATE", "17 November 2026"]]


# -- kept: the tblPrChange Word's re-save adds to a table with a tracked cell or row change ---------

_A = "2026-11-17T10:00:00Z"
_B = "2026-11-18T10:00:00Z"


def _revisions(ours, word):
    def facts(revisions):
        return {"document": {"paragraphs": [{"id": "1A2B3C4D", "text": "In a cell"}], "revisions": revisions}}

    return compare.kept(facts(ours), facts(word))


def test_kept_takes_word_s_tblprchange_beside_a_cell_change_by_the_same_author():
    ours = [["ins", "Writer A", _A], ["tcPrChange", "Writer A", _A]]
    word = [["ins", "Writer A", _A], ["tblPrChange", "Writer A", _A], ["tcPrChange", "Writer A", _A]]
    assert _revisions(ours, word) == []


def test_kept_takes_word_s_tblprchange_beside_a_row_change_by_the_same_author():
    ours = [["trPrChange", "Writer A", _A]]
    assert _revisions(ours, [["tblPrChange", "Writer A", _A], ["trPrChange", "Writer A", _A]]) == []


def test_kept_still_finds_a_tblprchange_on_a_table_without_cell_or_row_changes():
    """The negative: Word's tblPrChange where the tools tracked no cell or row change."""
    ours = [["ins", "Writer A", _A]]
    assert _revisions(ours, [["ins", "Writer A", _A], ["tblPrChange", "Writer A", _A]])


def test_kept_still_finds_word_s_tblprchange_by_another_author_or_date():
    ours = [["tcPrChange", "Writer A", _A]]
    assert _revisions(ours, [["tblPrChange", "Writer B", _A], ["tcPrChange", "Writer A", _A]])
    assert _revisions(ours, [["tblPrChange", "Writer A", _B], ["tcPrChange", "Writer A", _A]])


def test_kept_still_finds_a_tblprchange_word_drops():
    ours = [["tblPrChange", "Writer A", _A], ["tcPrChange", "Writer A", _A]]
    assert _revisions(ours, [["tcPrChange", "Writer A", _A]])


def test_kept_still_finds_a_cell_change_word_drops():
    ours = [["tcPrChange", "Writer A", _A]]
    assert _revisions(ours, [["tblPrChange", "Writer A", _A]])
