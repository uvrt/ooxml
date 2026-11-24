"""Where the tools differ from Word on purpose, or are known to and the fix is still to come:
``{case id: {check: reason}}``, a check one of ``answers.CHECKS`` or ``opens`` (Word opens the
output at all).  A listed check is a strict xfail in the Word run and the recorded tier both, so a
fix (or a change of Word's) shows as an unexpected pass; the fix's lane removes the entry.

Must not: import anything.
"""

from __future__ import annotations

DEVIATIONS: dict[str, dict[str, str]] = {
    "gates/docx2svg/style-document.docx/style:1b2189e4005924c7": {
        "accept": "known, fix to come: after Accept All Word joins p@body/11 and the next paragraph under "
                  "'1.' numbering; the tools keep two paragraphs and a '*' bullet",
    },
    "gates/generated/charts/chart-places.docx/objects:0ab11bf053a1ef1d": {
        "pages": "tools limitation: the layout refuses a text box it cannot lay out yet (not supported)",
    },
    "gates/generated/charts/chart-places.docx/structure:2059124caf17ded5": {
        "opens": "known, fix to come: word_notes move puts the note's reference inside a text box (in both "
                 "its wps and its VML copy); Word stops on an alert as it opens the file, and opens it with "
                 "the reference moved out of the text box",
    },
    "gates/generated/controls.docx/objects:09dbf7b1f77502ee": {
        "kept": "known, to investigate: Word's re-save adds a tblPrChange (Writer A) the tools don't write; "
                "Word may expect the tracked split to record a table-property change",
    },
    "gates/generated/controls.docx/structure:9c1e3b8d58c39cab": {
        "fields": "known, fix to come: Word's update finds no bookmark 'around' (Error! Invalid bookmark "
                  "reference); the tools resolve it",
    },
}


def reason(case: str, check: str) -> str | None:
    return DEVIATIONS.get(case, {}).get(check)
