"""Where the tools differ from Word on purpose, or are known to and the fix is still to come:
``{case id: {check: reason}}``, a check one of ``answers.CHECKS`` or ``opens`` (Word opens the
output at all).  A listed check is a strict xfail in the Word run and the recorded tier both, so a
fix (or a change of Word's) shows as an unexpected pass; the fix's lane removes the entry.

Must not: import anything.
"""

from __future__ import annotations

#: After Reject All, Word shows control 309's placeholder spaces; the tools show the control empty
#: (every edit of controls.docx, the fixture's own revisions rejected).
_CONTROL_309 = ("known, fix to come: after Reject All Word shows content control 309's placeholder (spaces), "
                "the tools' reject leaves the control empty")

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
    "gates/generated/controls.docx/objects:02e833d45f2a311e": {"reject": _CONTROL_309},
    "gates/generated/controls.docx/objects:031165bebdad8fb3": {"reject": _CONTROL_309},
    "gates/generated/controls.docx/objects:03acdd1b12512576": {"reject": _CONTROL_309},
    "gates/generated/controls.docx/objects:041ad9ff89bd6d04": {"reject": _CONTROL_309},
    "gates/generated/controls.docx/objects:08f8cd07787e4d2a": {"reject": _CONTROL_309},
    "gates/generated/controls.docx/objects:09dbf7b1f77502ee": {
        "reject": _CONTROL_309,
        "kept": "known, to investigate: Word's re-save adds a tblPrChange (Writer A) the tools don't write; "
                "Word may expect the tracked split to record a table-property change",
    },
    "gates/generated/controls.docx/objects:19bd0fc9862ebeb1": {"reject": _CONTROL_309},
    "gates/generated/controls.docx/objects:1c38f902a0ff6dbe": {"reject": _CONTROL_309},
    "gates/generated/controls.docx/objects:3b9a78c7a9905fea": {"reject": _CONTROL_309},
    "gates/generated/controls.docx/objects:8b6d3f46eaa20267": {"reject": _CONTROL_309},
    "gates/generated/controls.docx/structure:1f624484c0f69a36": {"reject": _CONTROL_309},
    "gates/generated/controls.docx/structure:50664644cd359314": {"reject": _CONTROL_309},
    "gates/generated/controls.docx/structure:b04b88316aae0005": {"reject": _CONTROL_309},
    "gates/generated/controls.docx/structure:e8d562b45b0f0b25": {"reject": _CONTROL_309},
}


def reason(case: str, check: str) -> str | None:
    return DEVIATIONS.get(case, {}).get(check)
