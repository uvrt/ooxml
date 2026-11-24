"""The suite's comparisons, each between the tools' answer and Word's as ``answers.py`` reads
them: a list of differences, empty where they agree.  Shared by the Word run
(``test_word_suite.py``) and the recorded-Word tier (``word/tests/test_suite_recorded.py``),
which compares with Word's answers as the observation recorded them.

Must not: import pytest, or drive Word.
"""

from __future__ import annotations

import json
import re
from typing import Any

#: A paragraph is looked for in Word's PDF by the first characters of its text.
_KEY_CHARS = 30

# -- what depends on the machine running Word, not on the tools ---------------------------------
#
# Three named rules, each as narrow as the measurement behind it (the gates' sample, Word 16.106):
#
# * CLOCK_FIELDS: Word fills a date or time field from its clock when it updates the field (its
#   re-save did, ``gates/docx2svg/layout-sweep.docx/structure:449ba5552704e6ab``: the day Word
#   ran); the tools from the session's clock.  Compared by the field's keyword only.
# * USER_FIELDS: Word fills AUTHOR and its kin from the user Word runs as
#   (``gates/generated/ids-and-markup.docx/tools:8a40725a7ad02617``); the tools from the
#   document's properties.  Compared by the field's keyword only.
# * An empty table of contents: Word's update writes its placeholder in the language of Word's
#   interface (``gates/docx2svg/layout-sweep.docx/structure:0b868ed3a73f3fea``: Dutch), the tools
#   theirs in English.  Word's placeholder in each language measured counts as the tools'.

#: Fields whose result is Word's clock at its update.
CLOCK_FIELDS = frozenset({"DATE", "TIME", "PRINTDATE", "SAVEDATE", "CREATEDATE"})
#: Fields whose result is the user Word runs as.
USER_FIELDS = frozenset({"AUTHOR", "USERNAME", "USERINITIALS"})
#: The fields compared by their keyword only (the results depend on the machine running Word).
KEYWORD_ONLY_FIELDS = CLOCK_FIELDS | USER_FIELDS
#: The tools' result of a table of contents with no entries.
TOOLS_EMPTY_TOC = "No table of contents entries found."
#: Word's, by the language of its interface (measured: Word 16.106 for Mac).
WORD_EMPTY_TOC = frozenset({
    "No table of contents entries found.",          # English
    "Geen inhoudsopgavegegevens gevonden.",         # Dutch
})


def same_field(mine: list[str], theirs: list[str]) -> bool:
    """One field's ``[keyword, result]`` on each side is the same field with the same result,
    under the rules above."""
    if mine[0] != theirs[0]:
        return False
    if mine[0] in KEYWORD_ONLY_FIELDS:
        return True
    if mine[0] == "TOC" and mine[1] == TOOLS_EMPTY_TOC and theirs[1] in WORD_EMPTY_TOC:
        return True
    return mine[1] == theirs[1]


def masked_text(paragraph: dict[str, Any]) -> str:
    """A paragraph's text with the result of each field compared by keyword only (its
    ``fields``, ``answers.kept_facts``) written as ``{KEYWORD}``."""
    text = paragraph.get("text", "")
    for keyword, result in paragraph.get("fields", []):
        if keyword in KEYWORD_ONLY_FIELDS and result:
            text = text.replace(result, "{%s}" % keyword, 1)
    return text


def _group(story: str) -> str:
    """Word numbers header and footer parts as it saves them: they are compared as a set."""
    return re.sub(r"\d+$", "", story) if story.startswith(("header", "footer")) else story


def _grouped(facts: dict[str, Any]) -> dict[str, list[Any]]:
    out: dict[str, list[Any]] = {}
    for story, entry in facts.items():
        out.setdefault(_group(story), []).append(entry)
    return {k: sorted(v, key=lambda e: json.dumps(e, sort_keys=True)) if len(v) > 1 else v for k, v in out.items()}


def _paragraph_differences(story: str, ours: list[dict], word: list[dict]) -> list[str]:
    out = []
    if len(ours) != len(word):
        out.append(f"{story}: {len(ours)} paragraphs, Word kept {len(word)}")
    for k, (mine, theirs) in enumerate(zip(ours, word)):
        if mine.get("id") and mine["id"] != theirs.get("id"):
            out.append(f"{story} paragraph {k + 1}: paraId {mine['id']}, Word's {theirs.get('id')}")
        if masked_text(mine) != masked_text(theirs):
            out.append(f"{story} paragraph {k + 1}: text {mine.get('text', '')!r}, Word's {theirs.get('text', '')!r}")
        if mine.get("deleted", "") != theirs.get("deleted", ""):
            out.append(f"{story} paragraph {k + 1}: deleted {mine.get('deleted', '')!r}, "
                       f"Word's {theirs.get('deleted', '')!r}")
    return out


def kept(ours: dict[str, Any], word: dict[str, Any]) -> list[str]:
    """Word's re-save keeps every paraId, every paragraph's text (:func:`masked_text`), every
    revision's author and date and every comment (``answers.kept_facts`` on each side)."""
    mine, theirs = _grouped(ours), _grouped(word)
    out = []
    if sorted(mine) != sorted(theirs):
        out.append(f"stories {sorted(mine)}, Word's {sorted(theirs)}")
    for story in sorted(set(mine) & set(theirs)):
        if len(mine[story]) != len(theirs[story]):
            out.append(f"{story}: {len(mine[story])} parts, Word's {len(theirs[story])}")
        for a, b in zip(mine[story], theirs[story]):
            out += _paragraph_differences(story, a["paragraphs"], b["paragraphs"])
            for key in ("revisions", "comments"):
                if a.get(key, []) != b.get(key, []):
                    out.append(f"{story} {key}: {a.get(key, [])}, Word's {b.get(key, [])}")
    return out


def markdown(action: str, ours: str, word: str) -> list[str]:
    """The tools' review (``action``) reads as Word's does."""
    if ours == word:
        return []
    import difflib

    diff = list(difflib.unified_diff(word.splitlines(), ours.splitlines(), "Word", "tools", lineterm="", n=1))
    return [f"{action}: the final views differ\n" + "\n".join(diff[:40])]


def _squeezed(text: str) -> str:
    return re.sub(r"\s+", "", text)


def _key(text: str) -> str:
    """What a paragraph is looked for by in Word's PDF: its text up to its first tab (Word's PDF
    spells a tab's leader as dots), whitespace left out, its first characters, without the
    punctuation it ends on (a note's reference mark is drawn before it)."""
    return _squeezed(text.split("\t", 1)[0])[:_KEY_CHARS].rstrip(".,;:!?")


def pages(ours: dict[str, Any], word: dict[str, Any]) -> tuple[list[str], list[str]]:
    """The tools' page count is Word's, and every paragraph found in Word's PDF is on the page
    the tools lay its first line on.  The paragraphs are found in order in the text of all
    Word's pages, each after the place the one before was found (so a heading is not taken for
    its entry in a table of contents), by :func:`_key`; one not found (a glyph Word's PDF spells
    otherwise) is listed apart, not a difference.  Returns (differences, not found)."""
    word_pages = [_squeezed(page) for page in word["text"]]
    starts, text = [], ""
    for page in word_pages:
        starts.append(len(text))
        text += page
    out, missing = [], []
    if ours["pages"] != word["count"]:
        out.append(f"{ours['pages']} pages, Word's {word['count']}")
    at = 0
    for page, paragraph in ours["paragraphs"]:
        key = _key(paragraph)
        found = text.find(key, at) if key else -1
        if found < 0:
            missing.append(paragraph[:60])
            continue
        at = found + len(key)
        word_page = max(k for k, start in enumerate(starts) if start <= found) + 1
        if word_page != page:
            out.append(f"{paragraph[:60]!r}: page {page}, Word's {word_page}")
    return out, missing


def fields(ours: list[list[str]], word: list[list[str]]) -> list[str]:
    """After an update of every field, each field's result is Word's (:func:`same_field`)."""
    if ours == word:
        return []
    out = [f"{len(ours)} fields, Word's {len(word)}"] if len(ours) != len(word) else []
    for k, (mine, theirs) in enumerate(zip(ours, word)):
        if not same_field(mine, theirs):
            out.append(f"field {k + 1} {mine[0]}: {mine[1]!r}, Word's {theirs[0]}: {theirs[1]!r}")
    return out


def judge(check: str, ours: Any, word: Any) -> list[str]:
    """The differences between the tools' answer to ``check`` and Word's."""
    if check == "kept":
        return kept(ours, word)
    if check in ("accept", "reject"):
        return markdown(check, ours, word)
    if check == "pages":
        return pages(ours, word)[0]
    if check == "fields":
        return fields(ours, word)
    raise ValueError(check)
