"""Where the tools differ from Word on purpose, or are known to and the fix is still to come:
``{case id: {check: reason}}``.  A listed check is a strict xfail in the Word run and the
recorded tier both, so a fix (or a change of Word's) shows as an unexpected pass.

Must not: import anything.
"""

from __future__ import annotations

DEVIATIONS: dict[str, dict[str, str]] = {}


def reason(case: str, check: str) -> str | None:
    return DEVIATIONS.get(case, {}).get(check)
