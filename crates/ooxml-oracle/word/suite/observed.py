"""The suite's observation (``word/observations/suite-word.json``): Word's answers about each
case's tool output, as ``answers.py`` reads them, with the Word version that gave them and the
digest of the output they are about.  A Word run replaces the cases it ran and keeps the rest,
so a change in Word's answers is a reviewable diff.

Must not: import pytest.
"""

from __future__ import annotations

import json
from pathlib import Path
from typing import Any

PATH = Path(__file__).resolve().parents[1] / "observations" / "suite-word.json"
NOTES = {
    "_about": "Word's answers about the tools' outputs (crates/ooxml-oracle/word/suite, pytest -m word): "
              "per case, the digest of the output asked about, whether Word opens and exports it "
              "(opens), what its re-save keeps (kept: paraIds, texts, revisions, comments), the final "
              "view of its Accept All and Reject All (accept, reject: word_read's Markdown, ids left "
              "out), its PDF of the accepted output (pages: count and each page's text) and its field "
              "results after its own update (fields).  Authors and dates are the suite's own.",
    "source": "crates/ooxml-oracle/word/suite/test_word_suite.py",
}


def load() -> dict[str, Any]:
    if PATH.exists():
        return json.loads(PATH.read_text(encoding="utf-8"))
    return dict(NOTES)


class Store:
    """The observation as a run changes it: written as each answer comes in (a run stopped
    half-way keeps what Word said so far)."""

    def __init__(self, word: str | None) -> None:
        self.data = load()
        self.data.update(NOTES)
        self.word = word
        self.changed = False

    def entry(self, case: str, output: str) -> dict[str, Any]:
        """The case's answers, started again when they are about another output."""
        current = self.data.get(case)
        if not isinstance(current, dict) or current.get("output") != output or current.get("word") != self.word:
            current = {"output": output, "word": self.word}
            self.data[case] = current
            self.changed = True
        return current

    def record(self, case: str, output: str, key: str, value: Any) -> None:
        entry = self.entry(case, output)
        if entry.get(key) != value:
            entry[key] = value
            self.changed = True
        self.write()

    def write(self) -> None:
        if not self.changed:
            return
        self.changed = False
        self.data["word"] = self.word
        notes = {k: self.data[k] for k in ("_about", "source", "word")}
        cases = {k: v for k, v in sorted(self.data.items()) if k not in notes}
        PATH.write_text(json.dumps({**notes, **cases}, indent=1, ensure_ascii=False) + "\n", encoding="utf-8")
