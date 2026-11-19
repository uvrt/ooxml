"""The style tools' cases for ``word_edits_diff.py --style`` (lane P4d): the tools called
through docx-agent's toolbox on a document as opened, compared as the structure tools'
cases are (:mod:`word_structure_cases`), with the Rust core's (``word-edits``,
``op: "style"``).

Python's behaviour is not changed: this calls docx-agent's public and documented internal
functions only.
"""

from __future__ import annotations

import os

from word_structure_cases import TRACKING, _words


def call(tool: str, **arguments) -> dict:
    return {"op": "style", "tool": tool, "arguments": {"doc": "d1", **arguments}, "tracked": None}


def tracked(case: dict) -> dict:
    return {**case, "tracked": TRACKING}


def _listed(document) -> list[str]:
    """Body paragraphs in a list, the first dozen."""
    out = []
    for entry in document._index(document.package.document_part()).paragraphs:
        if document.list_of(entry.id) is not None:
            out.append(entry.id)
        if len(out) == 12:
            break
    return out


def list_cases(document) -> list[dict]:
    """``word_lists``: paragraphs added to bullet and numbered lists (one, a span, a table,
    at a level), taken out (in no list, numbered by their style, forced), moved to a
    level, restarted, continued (joining a paragraph), a level formatted; refusals --
    untracked and tracked."""
    out = []
    words = [pid for pid, _ in _words(document)]
    listed = _listed(document)
    for k, pid in enumerate(words[:4]):
        out.append(call("word_lists", targets=[pid], action="add", kind="number" if k % 2 else "bullet"))
    if len(words) > 2:
        out += [call("word_lists", targets=[f"{words[1]}..{words[2]}"], action="add", kind="number", level=1),
                call("word_lists", targets=[words[0], words[1]], action="add"),
                call("word_lists", targets=[words[0]], action="remove"),
                call("word_lists", targets=[words[0]], action="level", level=2),
                call("word_lists", targets=[words[0]], action="restart"),
                call("word_lists", targets=[words[0]], action="add", join=words[1])]
    tables = [b["id"] for b in document.blocks() if b["id"] and b["id"].startswith("t:")]
    for table in tables[:1]:
        out.append(call("word_lists", targets=[table], action="add", kind="number"))
    for pid in listed[:4]:
        out += [call("word_lists", targets=[pid], action="remove"),
                call("word_lists", targets=[pid], action="remove", force=True),
                call("word_lists", targets=[pid], action="level", level=1),
                call("word_lists", targets=[pid], action="level"),
                call("word_lists", targets=[pid], action="restart", start=3),
                call("word_lists", targets=[pid], action="continue"),
                call("word_lists", targets=[pid], action="format", number_format="lowerRoman", label="%1)",
                     start=2),
                call("word_lists", targets=[pid], action="format", level=1, number_at=18, text_at=36,
                     follow="space", tab_at=40),
                call("word_lists", targets=[pid], action="format", follow="tab", number_format="xyz")]
        if words:
            out.append(call("word_lists", targets=[pid], action="continue", join=words[0]))
    if len(listed) > 1:
        out += [call("word_lists", targets=[listed[-1]], action="continue", join=listed[0]),
                call("word_lists", targets=[f"{listed[0]}..{listed[-1]}"], action="level", level=1),
                call("word_lists", targets=[listed[0], listed[-1]], action="remove", force=True)]
    out.append(call("word_lists", targets=["p:00000000"], action="add"))
    return out + [tracked(case) for case in out]


def format_cases(document) -> list[dict]:
    """``word_format``: run and paragraph formatting on paragraphs, ranges, spans and tables;
    clearing; styles; text found; a sample's look; keeps; refusals -- untracked and tracked."""
    out = []
    words = _words(document)
    pids = [pid for pid, _ in words]
    listed = _listed(document)
    for k, (pid, text) in enumerate(words[:4]):
        cut = min(len(text), 4)
        out += [call("word_format", targets=[pid], bold=True, size=13, color="accent1" if k % 2 else "C00000"),
                call("word_format", targets=pid, alignment="center", space_before=6, bottom=3, line_spacing=1.15,
                     indent_left=18, indent_first=-9 if k % 2 else 12, keep_with_next=True),
                call("word_format", targets=[f"{pid}@0:{cut}"], italic=True, underline=True, highlight="yellow",
                     font="Arial"),
                call("word_format", targets=[pid], clear_direct=True),
                call("word_format", targets=[pid], clear=["bold", "indent_first", "keep_with_next", "space_after"]),
                call("word_format", targets=[pid], paragraph_style="Heading 2"),
                call("word_format", targets=[f"{pid}@0:{cut}"], character_style="Strong", strike=False),
                call("word_format", targets=[pid], character_style="Emphasis", highlight="none", keep_lines=True),
                call("word_format", targets=[pid], keep_together=True)]
        word = text.split()[0] if text.split() else ""
        if word:
            out += [call("word_format", find={"text": word}, bold=True),
                    call("word_format", find=[{"text": word, "expect": "all", "within": pid}], italic=True),
                    call("word_format", find=[{"text": word.upper() + "x", "within": pid}], italic=True),
                    call("word_format", find=[{"text": word, "occurrence": 2}], italic=True)]
    if len(pids) > 2:
        out += [call("word_format", targets=[f"{pids[0]}..{pids[2]}"], keep_together=True),
                call("word_format", targets=[f"{pids[0]}..{pids[2]}"], keep_together=False),
                call("word_format", targets=[f"{pids[1]}@1..{pids[2]}@2"], bold=True),
                call("word_format", targets=[pids[1], pids[2]], like=pids[0]),
                call("word_format", targets=[pids[1]], like=pids[0], paragraph_style="Title", keep_with_next=False),
                call("word_format", targets=[pids[0]], like=f"{pids[0]}..{pids[2]}")]
    for pid in listed[:3]:
        if pids:
            out += [call("word_format", targets=[pids[0]], like=pid),
                    call("word_format", targets=[pid], like=pids[0])]
    tables = [b["id"] for b in document.blocks() if b["id"] and b["id"].startswith("t:")]
    for table in tables[:2]:
        out += [call("word_format", targets=[table], keep_together=True),
                call("word_format", targets=[table], keep_together=False),
                call("word_format", targets=[table], italic=True, space_after=0)]
    out += [call("word_format", targets=pids[:1], paragraph_style="No Such Style"),
            call("word_format", targets=pids[:1], character_style="Heading 1"),
            call("word_format", targets=pids[:1], color="nope"),
            call("word_format", targets=pids[:1]),
            call("word_format", targets=pids[:1], bold=True, clear=["bold"]),
            call("word_format", bold=True),
            call("word_format", targets=[], bold=True),
            call("word_format", targets=["p:00000000"], bold=True),
            call("word_format", targets=["s:1"], keep_together=True),
            call("word_format", targets=["s:1"], italic=True)]
    return out + [tracked(case) for case in out]


def cases(document) -> list[dict]:
    """Every style case for ``document``."""
    found = list_cases(document) + format_cases(document)
    wanted = os.environ.get("STYLE_TOOLS")       # a comma list, to run some tools only
    found = [c for c in found if not wanted or c["tool"] in wanted.split(",")]
    per_tool = int(os.environ.get("STYLE_PER_TOOL") or 0)   # the first n of each tool's
    if per_tool:
        taken: dict[str, int] = {}
        kept = []
        for case in found:
            key = case["tool"] + ("+tracked" if case.get("tracked") else "")
            if taken.get(key, 0) < per_tool:
                taken[key] = taken.get(key, 0) + 1
                kept.append(case)
        found = kept
    return found
