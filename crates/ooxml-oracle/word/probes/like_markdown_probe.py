#!/usr/bin/env python3
"""Measure what Word makes when a person adds items to a list, tracked, and what it makes of the
tools' items written like a sample (round 60, R141: ``word_insert_markdown`` ``like``).

The document and the cases are ``cases/like_markdown_cases.py``'s: a numbered list and a bullet
list, each added to.  Word -- through ``crates/ooxml-oracle/word/oracle.py``'s machine-wide
lock --

* ``word-added``: opens the document, turns tracking on and, per case, presses Enter at the end
  of the list's last item and types the new items (an item a level down: Word's list indent, as
  Tab demotes one), and saves;
* ``tools-opened``: opens the document the tools made (``word_insert_markdown`` ``like`` the
  sample, after the list's last item, tracked) and saves it again: whether it opens without a
  prompt;
* ``word-labels``, ``tools-labels``: Accept All of Word's document and of the tools', then the
  list numbers converted to text (Word's ``convert numbers to text``): each paragraph's label as
  Word numbers it;
* ``tools-reject``: Word's Reject All of the tools' document.

Recorded per paragraph (by its key, the first word): its text as runs (``[kind, text]``, kind
``text``, ``ins`` or ``del``), its mark's revision (``ins``, ``del`` or none) and its own
``w:pPr`` settings (revision records aside) with their values; after a review, each key's text
and settings, and whether any revision was left.

    python crates/ooxml-oracle/word/probes/like_markdown_probe.py   # writes observations/like-markdown-word.json

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

from like_markdown_cases import AUTHOR, BASE, CASES  # noqa: E402

OBSERVATIONS = probe_paths.OBSERVATIONS / "like-markdown-word.json"
W = "http://schemas.openxmlformats.org/wordprocessingml/2006/main"
_W = "{%s}" % W
#: A paragraph's properties that are its revision records, not its settings.
_RECORDS = {"rPr", "pPrChange", "sectPr"}


#: A block's text as the tools read it: its list label (``1.``, ``-``, ``*``), then its key.
_KEYED = re.compile(r"(?:\d+\.|[-*])?\s*(\w+)")


def _keys(session: probe_tools.Session) -> dict[str, str]:
    return {found.group(1): block for block, text in session.blocks() if (found := _KEYED.match(text))}


def document() -> bytes:
    """The document before the edits: the lists as the tools write them into a new document."""
    session = probe_tools.Session(probe_tools.new())
    for markdown, style_map in BASE:
        session.call("word_insert_markdown", at="end", markdown=markdown,
                     **({"style_map": style_map} if style_map else {}))
    return session.data()


def tools_document(said: list[str] | None = None) -> bytes:
    """The document after the tools' tracked ``word_insert_markdown`` ``like`` of each case
    (their warnings added to ``said``)."""
    said = [] if said is None else said
    session = probe_tools.Session(document())
    keys = _keys(session)
    session.call("word_set_tracking", on=True, author=AUTHOR)
    for case in CASES.values():
        made = session.call("word_insert_markdown", at=f"after:{keys[case['after']]}", markdown=case["markdown"],
                            like=[keys[case["sample"]]])
        said.extend(made.to_json().get("warnings", []))
    return session.data()


def word_script() -> str:
    """Per case, from the bottom up: Enter at the end of the list's last item, each new item
    typed (one a level down list-indented first)."""
    lines = ["set track revisions of d to true"]
    for case in reversed(list(CASES.values())):
        lines += [e3_probe._locate(case["after"]), "set e to end of content of text object of paragraph k of d",
                  "select (create range d start (e - 1) end (e - 1))"]
        level = 1
        for text, depth in case["new"]:
            lines.append("type paragraph selection")
            while level < depth:
                lines.append("list indent (list format of text object of selection)")
                level += 1
            lines.append(e3_probe._type(text))
    return e3_probe._script(lines)


def labels_script() -> str:
    """Accept All, then the list numbers as text."""
    return e3_probe._script(["accept all revisions d", "convert numbers to text (list format of text object of d)"])


def runs(paragraph) -> list[list[str]]:
    """A paragraph's text as ``[kind, text]`` runs: ``text``, ``ins`` or ``del``, neighbours of
    one kind joined."""
    out: list[list[str]] = []
    for node in paragraph.iter(_W + "t", _W + "delText", _W + "tab"):
        kinds = {etree.QName(a).localname for a in node.iterancestors() if isinstance(a.tag, str)}
        kind = "del" if "del" in kinds else "ins" if "ins" in kinds else "text"
        text = "\t" if node.tag == _W + "tab" else node.text or ""
        if node.tag == _W + "tab" and etree.QName(node.getparent()).localname != "r":
            continue  # a tab stop, not a tab
        if out and out[-1][0] == kind:
            out[-1][1] += text
        else:
            out.append([kind, text])
    return out


def _mark(paragraph) -> str | None:
    mark = paragraph.find(f"{_W}pPr/{_W}rPr")
    if mark is None:
        return None
    for kind in ("ins", "del"):
        if mark.find(_W + kind) is not None:
            return kind
    return None


def _settings(paragraph) -> dict[str, dict]:
    """A paragraph's own settings: each ``w:pPr`` child (revision records aside) as its
    attributes and its children's (``numPr``: ``ilvl`` and ``numId``)."""
    properties = paragraph.find(_W + "pPr")
    out: dict[str, dict] = {}
    for child in properties if properties is not None else []:
        if not isinstance(child.tag, str) or etree.QName(child).localname in _RECORDS:
            continue
        values = {etree.QName(k).localname: v for k, v in child.attrib.items()}
        for inner in child:
            if isinstance(inner.tag, str) and etree.QName(inner).localname not in ("ins", "del", "numberingChange"):
                values[etree.QName(inner).localname] = inner.get(_W + "val")
        out[etree.QName(child).localname] = values
    return out


def keyed(data: bytes) -> dict[str, dict]:
    """Each body paragraph's runs, mark revision and settings, by key (its first word)."""
    root = etree.fromstring(e3_probe.parts(data)["word/document.xml"])
    out: dict[str, dict] = {}
    for paragraph in root.find(_W + "body").iter(_W + "p"):
        text = "".join(t for _, t in runs(paragraph))
        found = re.match(r"\W*(?:[\w.]+\t)?(\w+)", text)
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
    base, ours = document(), tools_document()
    with oracle.session() as session:
        session.recover()
        made = session.run_script(word_script(), base, name="like-markdown", tag="word", timeout=200)
        if not made:
            raise SystemExit(f"Word did not add the items: {made.outcome} {made.detail}")
        theirs = made.path.read_bytes()
        out["word-added"] = {"paragraphs": keyed(theirs)}
        opened = session.resave(ours, name="like-markdown-tools", timeout=200)
        out["tools-opened"] = {"outcome": opened.outcome, "detail": opened.detail[:200]}
        for name, data in (("word", theirs), ("tools", ours)):
            done = session.run_script(labels_script(), data, name=f"like-markdown-{name}", tag="labels", timeout=200)
            if not done:
                raise SystemExit(f"Word did not accept and number {name}' items: {done.outcome} {done.detail}")
            out[f"{name}-labels"] = reviewed(done.path.read_bytes())
        done = session.run_script(e3_probe.review_script("reject"), ours, name="like-markdown-tools", tag="reject",
                                  timeout=200)
        if not done:
            raise SystemExit(f"Word did not reject the tools' items: {done.outcome} {done.detail}")
        out["tools-reject"] = reviewed(done.path.read_bytes())
    OBSERVATIONS.write_text(json.dumps({
        "_about": ("What Word 16.106 for Mac wrote for crates/ooxml-oracle/word/probes/like_markdown_probe.py "
                   "(document and cases in cases/like_markdown_cases.py): 'word-added', items added to each list "
                   "as a person adds them, tracked (Enter at the end of the last item, the text typed, Word's "
                   "list indent for an item a level down; per paragraph key its runs as [kind, text], its mark's "
                   "revision and its own pPr settings with their values); 'tools-opened', whether it opened and "
                   "saved the tools' tracked word_insert_markdown like; 'word-labels'/'tools-labels', each key's "
                   "text and settings after Word's Accept All and convert numbers to text, of Word's document "
                   "and of the tools'; 'tools-reject', the same after Word's Reject All of the tools' document. "
                   "Regenerate with python crates/ooxml-oracle/word/probes/like_markdown_probe.py."),
        "word": "16.106 (macOS), Dutch interface",
        **out}, indent=1, ensure_ascii=False) + "\n", encoding="utf-8")
    print(f"wrote {OBSERVATIONS}")


if __name__ == "__main__":
    main()
