"""The style tools' cases for ``word_edits_diff.py --style`` (lane P4d): the tools called
through docx-agent's toolbox on a document as opened, compared as the structure tools'
cases are (:mod:`word_structure_cases`), with the Rust core's (``word-edits``,
``op: "style"``).

Python's behaviour is not changed: this calls docx-agent's public and documented internal
functions only.
"""

from __future__ import annotations

import os

from word_structure_cases import TRACKING, _words, fonts_of


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


def _named(document, kind: str) -> list[str]:
    return [s.name for s in document.styles if s.kind == kind and s.name]


def _custom(document) -> list[str]:
    """Custom styles nothing uses (purge's candidates), the first few."""
    out = []
    for style in document.styles:
        if not style.name or style.default:
            continue
        node = document.styles._node(style.id)
        if node is None or (node.get(f"{{{_W}}}customStyle") or "0") not in ("1", "true", "on"):
            continue
        if not document.styles.usage(style.name, style.kind)["content"]:
            out.append(style.name)
        if len(out) == 2:
            break
    return out


_W = "http://schemas.openxmlformats.org/wordprocessingml/2006/main"


def styles_cases(document) -> list[dict]:
    """``word_styles``: listed and described; paragraph, character and table styles added
    (formatting, shading, tab stops, regions); modified (a list level linked and unlinked,
    regions, no change); removed (replaced, refused), purged; the default table style; the
    language; the fields of other actions refused -- untracked and tracked."""
    def styles(**arguments) -> dict:
        return call("word_styles", **arguments)

    paragraphs, characters, tables = (_named(document, k) for k in ("paragraph", "character", "table"))
    out = [styles(action="list"), styles(action="list", kind="table"), styles(action="list", in_use_only=True)]
    for name in paragraphs[:3] + characters[:1] + tables[:2] + ["Heading 1", "No Such Style"]:
        out.append(styles(action="describe", name=name))
    out += [
        styles(action="add", name="Lane Body", based_on="Normal", next_style="Normal", bold=True, size=11,
               color="accent1", alignment="justify", space_after=6, indent_first=-12, keep_with_next=True,
               shading="accent1 lighter 80%", tabs=[{"position": 72, "alignment": "right", "leader": "dot"},
                                                   {"alignment": "right"}]),
        styles(action="add", name="Lane Mark", kind="character", italic=True, color="C00000", shading="FFFF00"),
        styles(action="add", name="Lane Quote", based_on="Intense Quote", line_spacing=1.15, indent_left=18),
        styles(action="add", name="Lane Tabs", kind="character", tabs=[{"position": 10}]),
        styles(action="add", name=(paragraphs or ["Normal"])[0]),
        styles(action="add", name="Lane Bad", based_on="No Such Style"),
        styles(action="add", name="Lane Grid", kind="table", size=9, shading="D9E2F3", regions=[
            {"region": "header_row", "shading": "1F3864", "color": "FFFFFF", "bold": True,
             "borders": {"style": "single", "sides": ["bottom"], "color": "1F3864", "width": 1}},
            {"region": "banded_rows", "shading": "F2F2F2"}, {"region": "first_column", "bold": True},
            {"region": "top_left_cell", "borders": {"style": "none"}}]),
        styles(action="add", name="Lane Bands", regions=[{"region": "banded_columns", "shading": "accent2 lighter 60%",
                                                          "band_size": 2}, {"region": "total_row", "band_size": 3}]),
    ]
    for name in paragraphs[:2]:
        out += [styles(action="modify", name=name, bold=True, size=12, shading="none"),
                styles(action="modify", name=name, tabs=[]),
                styles(action="modify", name=name, tabs=[{"position": 36, "alignment": "clear"}, {"position": 90}]),
                styles(action="modify", name=name, numbering={"list": "none"}),
                styles(action="modify", name=name, numbering={"list": "1"}),
                styles(action="modify", name=name, numbering={"list": "1", "level": 1}),
                styles(action="modify", name=name, numbering={"list": "999"}),
                styles(action="modify", name=name, numbering={"list": "No Such Style"}),
                styles(action="modify", name=name, numbering={"list": "none", "level": 0})]
    listed = [s.name for s in document.styles if s.kind == "paragraph" and s.name
              and document.styles.find(s.name, "paragraph") is not None
              and __import__("docx_agent.edit.numbering", fromlist=["list_style"]).list_style(document, s.id)]
    for name in listed[:2]:
        if paragraphs:
            out.append(styles(action="modify", name=paragraphs[0], numbering={"list": name, "level": 1}))
        out.append(styles(action="modify", name=name, numbering={"list": "none"}))
    for name in tables[:2]:
        out += [styles(action="modify", name=name, regions=[{"region": "header_row", "bold": True, "italic": False}]),
                styles(action="modify", name=name, regions=[{"region": "bottom_right_cell", "clear": True}]),
                styles(action="modify", name=name, regions=[{"region": "header_row", "clear": True},
                                                            {"region": "even_banded_rows", "shading": "none"}]),
                styles(action="modify", name=name, shading="EDEDED", size=10)]
    for name in characters[:1]:
        out += [styles(action="modify", name=name, space_after=6), styles(action="modify", name=name, italic=True)]
    out += [styles(action="modify", name=(paragraphs or ["Normal"])[0]),
            styles(action="modify", name="No Such Style", bold=True)]
    for name in _custom(document):
        out.append(styles(action="remove", name=name))
    for name in paragraphs[1:3]:
        out += [styles(action="remove", name=name), styles(action="remove", name=name, replacement="Normal"),
                styles(action="remove", name=name, replacement=name),
                styles(action="remove", name=name, replacement="Quote")]
    out += [styles(action="remove", name="Normal", replacement="Heading 1"), styles(action="remove", name="No Such"),
            styles(action="purge_unused")]
    for name in tables[:1] + ["Table Grid", "Grid Table 4 Accent 1", "No Such Table"]:
        out.append(styles(action="set_default", name=name))
    out.append(styles(action="set_default", name="Normal", kind="paragraph"))
    out += [styles(action="set_language", language="fr-BE"), styles(action="set_language", language="nl"),
            styles(action="set_language", language="xx-YY"), styles(action="set_language", language="en_us"),
            styles(action="set_language", language="fr-Latn-BE")]
    if len(paragraphs) > 1:
        out.append(styles(action="set_language", language="de-DE", name=paragraphs[1]))
    out += [styles(action="add", name="Lane X", language="fr-BE"), styles(action="add", name="Lane X",
            numbering={"list": "1"}), styles(action="remove", name="Lane X", tabs=[]),
            styles(action="describe", name="Normal", regions=[{"region": "header_row", "bold": True}])]
    return out + [tracked(case) for case in out]


def template_cases(document) -> list[dict]:
    """``word_template`` (lane P4d5): Convert with its reflow, both sides laid out with the
    same fonts (a document already in mode 15 is a no-op) -- untracked and tracked (it is
    never tracked)."""
    case = {**call("word_template", action="upgrade_to_modern"), "fonts": fonts_of(document)}
    return [case, tracked(case)]


def cases(document) -> list[dict]:
    """Every style case for ``document``."""
    found = list_cases(document) + format_cases(document) + styles_cases(document) + template_cases(document)
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
