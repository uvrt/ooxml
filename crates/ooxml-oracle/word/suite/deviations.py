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
    "gates/generated/charts/chart-places.docx/objects:0ab11bf053a1ef1d": {
        "pages": "tools limitation: the layout refuses a text box it cannot lay out yet (not supported)",
    },
    "gates/generated/controls.docx/objects:02e833d45f2a311e": {"reject": _CONTROL_309},
    "gates/generated/controls.docx/objects:031165bebdad8fb3": {"reject": _CONTROL_309},
    "gates/generated/controls.docx/objects:03acdd1b12512576": {"reject": _CONTROL_309},
    "gates/generated/controls.docx/objects:041ad9ff89bd6d04": {"reject": _CONTROL_309},
    "gates/generated/controls.docx/objects:08f8cd07787e4d2a": {"reject": _CONTROL_309},
    "gates/generated/controls.docx/objects:09dbf7b1f77502ee": {"reject": _CONTROL_309},
    "gates/generated/controls.docx/objects:19bd0fc9862ebeb1": {"reject": _CONTROL_309},
    "gates/generated/controls.docx/objects:1c38f902a0ff6dbe": {"reject": _CONTROL_309},
    "gates/generated/controls.docx/objects:3b9a78c7a9905fea": {"reject": _CONTROL_309},
    "gates/generated/controls.docx/objects:8b6d3f46eaa20267": {"reject": _CONTROL_309},
    "gates/generated/controls.docx/structure:1f624484c0f69a36": {"reject": _CONTROL_309},
    "gates/generated/controls.docx/structure:50664644cd359314": {"reject": _CONTROL_309},
    "gates/generated/controls.docx/structure:9c1e3b8d58c39cab": {
        "reject": _CONTROL_309,
        "fields": "known, fix to come: Word's update finds no bookmark 'around' (Error! Invalid bookmark "
                  "reference); the tools resolve it",
    },
    "gates/generated/controls.docx/structure:e8d562b45b0f0b25": {"reject": _CONTROL_309},
}


def reason(case: str, check: str) -> str | None:
    return DEVIATIONS.get(case, {}).get(check)
