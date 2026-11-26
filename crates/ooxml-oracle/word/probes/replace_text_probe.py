#!/usr/bin/env python3
"""Measure what Word records for a tracked replace in a range, and what it makes of the tools'
(round 60, R139: ``replace_text`` with ``range`` and ``regex``).

The document and the cases are ``cases/replace_text_cases.py``'s: a replace of ``2026`` in a span
of two paragraphs, one in a table cell, and one by a regular expression in one paragraph.  Word --
through ``crates/ooxml-oracle/word/oracle.py``'s machine-wide lock --

* ``word-replace``: opens the document, turns tracking on and makes each case with its own Find and
  Replace (``execute find`` on the range, ``replace all``, stopping at the range's end; the
  pattern as Word's wildcards), and saves;
* ``word-replace-untracked``: the same with tracking off, each key's text;
* ``tools-opened``: opens the document the tools made (``replace_text`` of each case, tracked)
  and saves it again: whether it opens without a prompt;
* ``tools-accept``, ``tools-reject``: Word's Accept All and Reject All of the tools' document.

Recorded: each block of Word's, normalised (e3's: authors, dates, rsids), and per paragraph or
cell (by its key, the first word) its text as runs: ``[kind, text]`` with kind ``text``, ``ins`` or
``del``, neighbours of one kind joined; for Accept All and Reject All, each key's text.

    python crates/ooxml-oracle/word/probes/replace_text_probe.py   # writes observations/replace-text-word.json

Nothing Word writes is committed: only the facts read from it.
"""

from __future__ import annotations

import io
import json
import re
import sys
import zipfile
from pathlib import Path

from lxml import etree

sys.path.insert(0, str(Path(__file__).resolve().parent))
import probe_paths  # noqa: E402  (the driver, the fixture makers and the cases on the path)

import e3_probe  # noqa: E402
import oracle  # noqa: E402
import probe_tools  # noqa: E402

from replace_text_cases import AUTHOR, CASES, document  # noqa: E402

OBSERVATIONS = probe_paths.OBSERVATIONS / "replace-text-word.json"
W = "http://schemas.openxmlformats.org/wordprocessingml/2006/main"
_W = "{%s}" % W


# -- Word's side -------------------------------------------------------------------------------


def _range(name: str) -> list[str]:
    """AppleScript: ``r`` is the case's range of ``d``."""
    if name == "span":
        return [e3_probe._locate("Alpha"), "set a to s", e3_probe._locate("Bravo"),
                "set r to create range d start a end (end of content of text object of paragraph k of d) - 1"]
    if name == "cell":
        return ["set c to text object of cell 2 of row 1 of table 1 of d",
                "set r to create range d start (start of content of c) end (end of content of c) - 1"]
    return [e3_probe._locate(name),
            "set r to create range d start s end (end of content of text object of paragraph k of d) - 1"]


def _quoted(text: str) -> str:
    return '"' + text.replace("\\", "\\\\").replace('"', '\\"') + '"'


def word_script(tracked: bool = True) -> str:
    lines = ["set track revisions of d to true"] if tracked else []
    for case in CASES.values():
        word = case["word"]
        lines += _range(word["range"])
        lines += ["set f to find object of r", "clear formatting f", "clear formatting (replacement of f)",
                  f"execute find f find text {_quoted(word['find'])} replace with {_quoted(word['replace'])} "
                  f"replace replace all wrap find find stop match case true "
                  f"match wildcards {'true' if word['wildcards'] else 'false'}"]
    return e3_probe._script(lines)


# -- the tools' side ---------------------------------------------------------------------------


def ranges(session: probe_tools.Session) -> dict[str, str]:
    """Each case's range as the tools name it, by its placeholder."""
    blocks = session.blocks()
    by_key = {re.match(r"\W*(\w+)", text).group(1): block for block, text in blocks if re.match(r"\W*\w+", text)}
    table = next(block for block, _ in blocks if block.startswith("t:"))
    return {"{span}": f"{by_key['Alpha']}..{by_key['Bravo']}", "{cell}": f"{table}/c0,1",
            "{Charlie}": by_key["Charlie"]}


def tools_document() -> bytes:
    """The document after the tools' ``replace_text`` of every case, tracked by :data:`AUTHOR`."""
    session = probe_tools.Session(document())
    session.call("word_set_tracking", on=True, author=AUTHOR)
    names = ranges(session)
    for case in CASES.values():
        arguments = dict(case["tools"])
        arguments["range"] = names[arguments["range"]]
        session.call("replace_text", **arguments)
    return session.data()


# -- reading -----------------------------------------------------------------------------------


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


def keyed(data: bytes) -> dict[str, list[list[str]]]:
    """Each body paragraph's and cell paragraph's runs, by its key (its text's first word)."""
    root = etree.fromstring(e3_probe.parts(data)["word/document.xml"])
    out = {}
    for paragraph in root.iter(_W + "p"):
        text = "".join(t.text or "" for t in paragraph.iter(_W + "t", _W + "delText"))
        found = re.match(r"(\w+)", text)
        if found:
            out[found.group(1)] = runs(paragraph)
    return out


def texts(data: bytes) -> dict[str, str]:
    """Each key's text as written (no revisions left)."""
    return {key: "".join(text for _, text in parts) for key, parts in keyed(data).items()}


def main() -> None:
    out: dict = {}
    ours = tools_document()
    with oracle.session() as session:
        session.recover()
        made = session.run_script(word_script(), document(), name="replace-text", tag="word", timeout=200)
        if not made:
            raise SystemExit(f"Word did not make the replaces: {made.outcome} {made.detail}")
        data = made.path.read_bytes()
        out["word-replace"] = {"blocks": e3_probe.blocks(data), "runs": keyed(data)}
        untracked = session.run_script(word_script(tracked=False), document(), name="replace-text", tag="untracked",
                                       timeout=200)
        if not untracked:
            raise SystemExit(f"Word did not make the untracked replaces: {untracked.outcome} {untracked.detail}")
        out["word-replace-untracked"] = {"texts": texts(untracked.path.read_bytes())}
        opened = session.resave(ours, name="replace-text-tools", timeout=200)
        out["tools-opened"] = {"outcome": opened.outcome, "detail": opened.detail[:200]}
        for action in ("accept", "reject"):
            done = session.run_script(e3_probe.review_script(action), ours, name="replace-text-tools",
                                      tag=action, timeout=200)
            if not done:
                raise SystemExit(f"Word did not {action} the tools' replaces: {done.outcome} {done.detail}")
            reviewed = done.path.read_bytes()
            out[f"tools-{action}"] = {"texts": texts(reviewed),
                                      "revisions": "<w:ins " in e3_probe.parts(reviewed)["word/document.xml"].decode()
                                      or "<w:del " in e3_probe.parts(reviewed)["word/document.xml"].decode()}
    OBSERVATIONS.write_text(json.dumps({
        "_about": ("What Word 16.106 for Mac wrote for crates/ooxml-oracle/word/probes/replace_text_probe.py "
                   "(documents and cases in cases/replace_text_cases.py): 'word-replace', its own tracked Find and "
                   "Replace of each case in its range (blocks normalised as e3's; 'runs' per paragraph key as "
                   "[kind, text], kind text/ins/del); 'word-replace-untracked', each key's text after the same "
                   "replaces untracked; 'tools-opened', whether it opened and saved the tools' "
                   "tracked replace_text of the same cases; 'tools-accept'/'tools-reject', each key's text after "
                   "its Accept All / Reject All of the tools' document and whether any revision was left. "
                   "Regenerate with python crates/ooxml-oracle/word/probes/replace_text_probe.py."),
        "word": "16.106 (macOS), Dutch interface",
        **out}, indent=1, ensure_ascii=False) + "\n", encoding="utf-8")
    print(f"wrote {OBSERVATIONS}")


if __name__ == "__main__":
    main()
