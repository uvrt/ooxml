#!/usr/bin/env python3
"""Measure what Word records when paragraphs are replaced by new ones, tracked, and what it makes
of the tools' replacement (round 60, R141: ``word_insert_markdown`` at ``replace:``).

The document and the case are ``cases/replace_markdown_cases.py``'s: ``First`` and ``Second``
(each with settings of its own) replaced by two new paragraphs.  Word -- through
``crates/ooxml-oracle/word/oracle.py``'s machine-wide lock --

* ``word-replace``: opens the document, turns tracking on, selects the two paragraphs whole and
  types the new ones over them (as a person replaces them), and saves;
* ``tools-opened``: opens the document the tools made (``word_insert_markdown`` at
  ``replace:First..Second``, tracked) and saves it again: whether it opens without a prompt;
* ``tools-accept``, ``tools-reject``: Word's Accept All and Reject All of the tools' document.

Recorded per paragraph (by its key, the first word): its text as runs (``[kind, text]``, kind
``text``, ``ins`` or ``del``), its mark's revision (``ins``, ``del`` or none) and its own
``w:pPr`` children (revision records aside); after Accept All and Reject All, each key's text and
``w:pPr`` children, and whether any revision was left.

    python crates/ooxml-oracle/word/probes/replace_markdown_probe.py   # writes observations/replace-markdown-word.json

Nothing Word writes is committed: only the facts read from it.
"""

from __future__ import annotations

import json
import re
import sys
from pathlib import Path

from lxml import etree

sys.path.insert(0, str(Path(__file__).resolve().parent))
import probe_paths  # noqa: E402  (the driver, the fixture makers and the cases on the path)

import e3_probe  # noqa: E402
import oracle  # noqa: E402
import probe_tools  # noqa: E402

from replace_markdown_cases import AUTHOR, MARKDOWN, NEW, REPLACED, document  # noqa: E402

OBSERVATIONS = probe_paths.OBSERVATIONS / "replace-markdown-word.json"
W = "http://schemas.openxmlformats.org/wordprocessingml/2006/main"
_W = "{%s}" % W
#: A paragraph's properties that are its revision records, not its settings.
_RECORDS = {"rPr", "pPrChange", "sectPr"}


def word_script() -> str:
    first, last = REPLACED
    lines = ["set track revisions of d to true", e3_probe._locate(first), "set a to s", e3_probe._locate(last),
             "set e to end of content of text object of paragraph k of d",
             "select (create range d start a end e)"]
    for text in NEW:
        lines += [e3_probe._type(text), "type paragraph selection"]
    return e3_probe._script(lines)


def tools_document() -> bytes:
    """The document after the tools' tracked ``word_insert_markdown`` at ``replace:First..Second``."""
    session = probe_tools.Session(document())
    by_key = {re.match(r"\W*(\w+)", text).group(1): block for block, text in session.blocks()
              if re.match(r"\W*\w+", text)}
    session.call("word_set_tracking", on=True, author=AUTHOR)
    first, last = REPLACED
    session.call("word_insert_markdown", at=f"replace:{by_key[first]}..{by_key[last]}", markdown=MARKDOWN)
    return session.data()


def runs(paragraph) -> list[list[str]]:
    """A paragraph's text as ``[kind, text]`` runs: ``text``, ``ins`` or ``del``, neighbours of
    one kind joined."""
    out: list[list[str]] = []
    for node in paragraph.iter(_W + "t", _W + "delText"):
        kinds = {etree.QName(a).localname for a in node.iterancestors() if isinstance(a.tag, str)}
        kind = "del" if "del" in kinds else "ins" if "ins" in kinds else "text"
        if out and out[-1][0] == kind:
            out[-1][1] += node.text or ""
        else:
            out.append([kind, node.text or ""])
    return out


def _mark(paragraph) -> str | None:
    mark = paragraph.find(f"{_W}pPr/{_W}rPr")
    if mark is None:
        return None
    for kind in ("ins", "del"):
        if mark.find(_W + kind) is not None:
            return kind
    return None


def _settings(paragraph) -> list[str]:
    properties = paragraph.find(_W + "pPr")
    if properties is None:
        return []
    return [etree.QName(c).localname for c in properties if isinstance(c.tag, str)
            and etree.QName(c).localname not in _RECORDS]


def keyed(data: bytes) -> dict[str, dict]:
    """Each body paragraph's runs, mark revision and settings, by key."""
    root = etree.fromstring(e3_probe.parts(data)["word/document.xml"])
    out: dict[str, dict] = {}
    for paragraph in root.find(_W + "body").iter(_W + "p"):
        text = "".join(t.text or "" for t in paragraph.iter(_W + "t", _W + "delText"))
        found = re.match(r"(\w+)", text)
        if found:
            out[found.group(1)] = {"runs": runs(paragraph), "mark": _mark(paragraph),
                                   "settings": _settings(paragraph)}
    return out


def reviewed(data: bytes) -> dict:
    """After a review: each key's text and settings, and whether any revision is left."""
    part = e3_probe.parts(data)["word/document.xml"].decode()
    return {"paragraphs": {key: {"text": "".join(t for _, t in entry["runs"]), "settings": entry["settings"]}
                           for key, entry in keyed(data).items()},
            "revisions": any(f"<w:{k} " in part for k in ("ins", "del", "pPrChange"))}


def main() -> None:
    out: dict = {}
    ours = tools_document()
    with oracle.session() as session:
        session.recover()
        made = session.run_script(word_script(), document(), name="replace-markdown", tag="word", timeout=200)
        if not made:
            raise SystemExit(f"Word did not replace the paragraphs: {made.outcome} {made.detail}")
        data = made.path.read_bytes()
        out["word-replace"] = {"blocks": e3_probe.blocks(data), "paragraphs": keyed(data)}
        opened = session.resave(ours, name="replace-markdown-tools", timeout=200)
        out["tools-opened"] = {"outcome": opened.outcome, "detail": opened.detail[:200]}
        for action in ("accept", "reject"):
            done = session.run_script(e3_probe.review_script(action), ours, name="replace-markdown-tools",
                                      tag=action, timeout=200)
            if not done:
                raise SystemExit(f"Word did not {action} the tools' replacement: {done.outcome} {done.detail}")
            out[f"tools-{action}"] = reviewed(done.path.read_bytes())
    OBSERVATIONS.write_text(json.dumps({
        "_about": ("What Word 16.106 for Mac wrote for crates/ooxml-oracle/word/probes/replace_markdown_probe.py "
                   "(document and case in cases/replace_markdown_cases.py): 'word-replace', two paragraphs selected "
                   "whole and new ones typed over them, tracked (blocks normalised as e3's; per paragraph key its "
                   "runs as [kind, text], its mark's revision and its own pPr children); 'tools-opened', whether it "
                   "opened and saved the tools' tracked word_insert_markdown at replace:; 'tools-accept'/"
                   "'tools-reject', each key's text and pPr children after its Accept All / Reject All of the "
                   "tools' document, and whether any revision was left. Regenerate with python "
                   "crates/ooxml-oracle/word/probes/replace_markdown_probe.py."),
        "word": "16.106 (macOS), Dutch interface",
        **out}, indent=1, ensure_ascii=False) + "\n", encoding="utf-8")
    print(f"wrote {OBSERVATIONS}")


if __name__ == "__main__":
    main()
