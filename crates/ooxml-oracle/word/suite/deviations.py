"""Where the tools differ from Word on purpose, or are known to and the fix is still to come:
``{case id: {check: reason}}``, a check one of ``answers.CHECKS`` or ``opens`` (Word opens the
output at all).  A listed check is a strict xfail in the Word run and the recorded tier both, so a
fix (or a change of Word's) shows as an unexpected pass; the fix's lane removes the entry.

Must not: import anything.
"""

from __future__ import annotations

DEVIATIONS: dict[str, dict[str, str]] = {
    "gates/generated/charts/chart-places.docx/objects:0ab11bf053a1ef1d": {
        "pages": "tools limitation: the layout refuses a text box it cannot lay out yet (not supported)",
    },
    "gates/generated/controls.docx/structure:9c1e3b8d58c39cab": {
        "fields": "known, fix to come: Word's update finds no bookmark 'around' (Error! Invalid bookmark "
                  "reference); the tools resolve it",
    },
}


def reason(case: str, check: str) -> str | None:
    return DEVIATIONS.get(case, {}).get(check)
