"""The tools held to Word's recorded answers about their outputs (``observations/suite-word.json``,
written by the Word-oracle suite, ``word/suite``), on any machine: each case is run through the
tools again and every check Word answered is compared as the suite compares it.  The layout's
checks (pages, fields) need the faces Word laid the outputs out in; without them they skip.

An output that is no longer the one Word was asked about (its digest changed) still has to read
as Word read the old one; where it does not, run the suite again (``pytest -m word``) to ask Word
about the new output."""

from __future__ import annotations

import sys
from pathlib import Path

import pytest

SUITE = Path(__file__).resolve().parents[1] / "suite"
sys.path.insert(0, str(SUITE))

import recorded  # noqa: E402,F401  (the probes and docx-agent's test helpers on the path)

import answers  # noqa: E402
import compare  # noqa: E402
import edit_sets  # noqa: E402
import observed  # noqa: E402
from deviations import reason  # noqa: E402

OBSERVED = observed.load()                    # suite-word.json
#: Every case of the observation is read here (a case the suite no longer makes fails).
READS = set(OBSERVED)
#: The faces the outputs are laid out in by Word; the layout's checks need them installed.
_OFFICE_FACES = ("Calibri", "Cambria", "Aptos")
_CASES = {case.id: case for case in edit_sets.all_cases()}
_PARAMS = [(case, check) for case, entry in sorted(OBSERVED.items()) if isinstance(entry, dict)
           for check in answers.CHECKS if check in entry]


def _faces_installed() -> bool:
    try:
        import docx2svg
        from docx2svg import rust
    except ImportError:
        return False
    # The faces Word's folders hold (a substitute is not the face), the application's left out.
    fonts = rust.word_fonts(docx2svg.ConvertOptions(font_dirs=[]))
    return all(fonts.face(family) is not None for family in _OFFICE_FACES)


@pytest.fixture(scope="module")
def made(box):
    outputs: dict[str, bytes] = {}

    def output(case_id: str) -> bytes:
        if case_id not in outputs:
            outputs[case_id] = edit_sets.run(box, _CASES[case_id])
        return outputs[case_id]

    return output


def test_every_recorded_case_is_still_made():
    cases = {k for k, v in OBSERVED.items() if isinstance(v, dict)}
    gone = sorted(c for c in cases - set(_CASES) if not c.startswith("gates/"))
    assert not gone, f"recorded but no longer a case: {gone}"


def test_word_opened_every_recorded_output():
    """Word opened every output it was asked about, but those ``deviations.py`` lists as
    known not to open (strict: one Word opened is no longer a deviation)."""
    entries = {k: v for k, v in OBSERVED.items() if isinstance(v, dict) and "opens" in v}
    refused = {k: v["opens"] for k, v in entries.items() if v["opens"] != "done" and not reason(k, "opens")}
    assert not refused, refused
    opened = sorted(k for k, v in entries.items() if v["opens"] == "done" and reason(k, "opens"))
    assert not opened, f"listed as a deviation, but Word opened it: {opened}"


@pytest.mark.parametrize("case, check", _PARAMS, ids=[f"{c}:{k}" for c, k in _PARAMS])
def test_the_tools_answer_as_word_did(box, made, case, check):
    if case not in _CASES:
        pytest.skip("a gates case: its frozen cases are not here")
    if check in ("pages", "fields") and not _faces_installed():
        pytest.skip("the faces Word laid the output out in are not installed here")
    entry = OBSERVED[case]
    data = made(case)
    differences = compare.judge(check, answers.answer(answers.Reader(box), data, check), entry[check])
    why = reason(case, check)
    if why:
        if differences:
            pytest.xfail(why)
        pytest.fail(f"listed as a deviation ({why}), but the tools now agree with Word", pytrace=False)
    if differences and edit_sets.digest(data) != entry["output"]:
        differences.append("(the output changed since Word was asked: run pytest -m word again)")
    assert not differences, "\n".join(differences)
