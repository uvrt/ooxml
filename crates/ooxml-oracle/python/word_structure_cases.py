"""The structure tools' cases for ``word_edits_diff.py --structure`` (lane P4b): the tools
called through docx-agent's toolbox on a document as opened, and what each side reports
-- the reply, every changed part, the part order, ``touched``, the validation delta, the
problems and the next ids -- for the harness to compare with the Rust core's
(``word-edits``, ``op: "structure"``).

Python's behaviour is not changed: this calls docx-agent's public and documented internal
functions only.
"""

from __future__ import annotations

import copy
import os
import datetime
import json

from docx_agent import validate as _validate
from docx_agent.edit import ids as _ids
from lxml import etree
from ooxml_edit.tools.savegate import compare_problems
from ooxml_edit.xml import serialize


TRACKING = {"author": "Writer A", "date": "2026-11-17T10:00:00Z"}


class ToolCases(list):
    """Each tool's case maker (a document's cases), in definition order: a tool's maker
    registers itself with ``@CASES``, so a new tool adds its own function and edits no
    shared list."""

    def __call__(self, make):
        self.append(make)
        return make

    def of(self, document) -> list[dict]:
        """Every registered tool's cases for ``document``."""
        return [case for make in self for case in make(document)]


CASES = ToolCases()
_NOW = datetime.datetime(2026, 11, 17, 10, 0, 0, tzinfo=datetime.timezone.utc)


def call(tool: str, **arguments) -> dict:
    return {"op": "structure", "tool": tool, "arguments": {"doc": "d1", **arguments}, "tracked": None}


def tracked(case: dict) -> dict:
    """``case`` with tracking on, by the harness's author, dated by the harness's clock."""
    return {**case, "tracked": TRACKING}


def _words(document) -> list[tuple[str, str]]:
    """Body paragraphs with text: (id, text), the first dozen."""
    out = []
    for entry in document._index(document.package.document_part()).paragraphs:
        text = document.paragraph(entry.id).text
        if text.strip() and "￼" not in text:
            out.append((entry.id, text))
        if len(out) == 12:
            break
    return out


@CASES
def link_cases(document) -> list[dict]:
    """``word_links``: the list; bookmarks over a paragraph, a range, text found, a table, a
    span; hyperlinks to an address and to a bookmark; a bookmark renamed and removed; a
    bookmark asked again (no change); refusals."""
    out = [call("word_links", action="list")]
    paragraphs = _words(document)
    for k, (pid, text) in enumerate(paragraphs[:4]):
        out.append(call("word_links", action="add_bookmark", target=pid, bookmark=f"Mark{k}"))
        if len(text) > 6:
            out.append(call("word_links", action="add_bookmark", target=f"{pid}@2:5", bookmark="Inner"))
            out.append(call("word_links", action="add_hyperlink", target=f"{pid}@1:4", url="https://example.com/a"))
            out.append(call("word_links", action="add_hyperlink", target=f"{pid}@0:{len(text)}",
                            url="mailto:someone@example.com"))
            out.append(call("word_links", action="add_bookmark", target=f"{pid}@3", bookmark="Place"))
        word = text.split()[0] if text.split() else text
        out.append(call("word_links", action="add_bookmark", find=word, bookmark="Found"))
        out.append(call("word_links", action="add_hyperlink", find=text[:12], url="https://example.com/"))
    if paragraphs:
        pid = paragraphs[0][0]
        out += [call("word_links", action="add_bookmark", target=pid, bookmark="1bad"),
                call("word_links", action="add_hyperlink", target=pid, url="example.com"),
                call("word_links", action="add_hyperlink", target=pid, url="https://x.org", bookmark="B"),
                call("word_links", action="add_bookmark", target=pid, find="x", bookmark="B"),
                call("word_links", action="add_bookmark", target=pid),
                call("word_links", action="add_hyperlink", target=f"{pid}@0:0", url="https://x.org")]
        if len(paragraphs) > 2:
            out.append(call("word_links", action="add_bookmark", target=f"{pid}..{paragraphs[2][0]}",
                            bookmark="Span"))
            out.append(call("word_links", action="add_bookmark", target=f"{pid}@1..{paragraphs[2][0]}@2",
                            bookmark="Across"))
    tables = [b["id"] for b in document.blocks() if b["id"] and b["id"].startswith("t:")]
    for table in tables[:2]:
        out.append(call("word_links", action="add_bookmark", target=table, bookmark="Table"))
        if paragraphs:
            out.append(call("word_links", action="add_bookmark", target=f"{paragraphs[0][0]}..{table}",
                            bookmark="ToTable"))
    marks = document.bookmarks(hidden=True)
    for mark in marks[:3]:
        name = mark.name
        out += [call("word_links", action="rename_bookmark", bookmark=name, new_name="Renamed1"),
                call("word_links", action="remove_bookmark", bookmark=name),
                call("word_links", action="add_bookmark", target=f"bm:{name}", bookmark=name),
                call("word_links", action="add_bookmark", target=f"bm:{name}", bookmark=name.lower())]
        if paragraphs:
            out.append(call("word_links", action="add_hyperlink", target=f"{paragraphs[0][0]}@0:2", bookmark=name))
    out += [call("word_links", action="rename_bookmark", bookmark="NoSuch", new_name="X"),
            call("word_links", action="remove_bookmark", bookmark="NoSuch")]
    return out


@CASES
def note_cases(document) -> list[dict]:
    """``word_notes``: the list; footnotes and endnotes inserted at positions, at a range's
    end and after text found; each note edited, deleted and moved; refusals -- untracked
    and tracked."""
    out = [call("word_notes", action="list")]
    paragraphs = _words(document)
    for pid, text in paragraphs[:3]:
        out += [call("word_notes", action="insert", at=f"{pid}@{min(3, len(text))}", text="A note."),
                call("word_notes", action="insert", kind="endnote", at=pid, text="First\nSecond"),
                call("word_notes", action="insert", find=text.split()[0], text="Found.")]
    if paragraphs:
        pid, text = paragraphs[-1]
        out += [call("word_notes", action="insert", at=f"{pid}@{len(text) + 5}", text="x"),
                call("word_notes", action="insert", at=pid, find="x", text="x"),
                call("word_notes", action="edit", note="fn:99", text="x"),
                call("word_notes", action="insert", at=f"{pid}@0", text="bad\u000c")]
    notes = document.notes()
    for note in notes[:3]:
        out += [call("word_notes", action="edit", note=note.id, text="Rewritten"),
                call("word_notes", action="edit", note=note.id, text="One\nTwo\nThree"),
                call("word_notes", action="delete", note=note.id)]
        if paragraphs:
            out.append(call("word_notes", action="move", note=note.id, at=f"{paragraphs[0][0]}@1"))
    return out + [tracked(case) for case in out if case["arguments"]["action"] != "list"]


@CASES
def section_cases(document) -> list[dict]:
    """``word_sections``: the list; breaks inserted after and before blocks, at the end and
    by a side written in ``after``; each break removed; page setup set; refusals --
    untracked and tracked."""
    out = [call("word_sections", action="list")]
    blocks = [b["id"] for b in document.blocks() if b["id"]]
    for k, block in enumerate(blocks[:3]):
        out += [call("word_sections", action="insert_break", after=block),
                call("word_sections", action="insert_break", before=block, start="continuous")]
    if blocks:
        out += [call("word_sections", action="insert_break", after="end", start="oddPage"),
                call("word_sections", action="insert_break", after=f"before:{blocks[-1]}"),
                call("word_sections", action="insert_break", after=blocks[0], before=blocks[0]),
                call("word_sections", action="set", section=blocks[0], columns=2)]
    sections = [s.id for s in document.sections()]
    for section in sections:
        if section != "s:body":
            out.append(call("word_sections", action="remove_break", section=section))
        out += [call("word_sections", action="set", section=section, orientation="landscape"),
                call("word_sections", action="set", section=section, margins={"top": 36, "left": 50.5}),
                call("word_sections", action="set", section=section, page_width=500, page_height=700,
                     start="evenPage"),
                call("word_sections", action="set", section=section, columns=1),
                call("word_sections", action="set", section=section, orientation="portrait")]
    out += [call("word_sections", action="remove_break", section="s:body"),
            call("word_sections", action="set", section="s:1"),
            call("word_sections", action="set", section="s:99", columns=2),
            call("word_sections", action="set", section="s:2", columns=3)]
    return out + [tracked(case) for case in out if case["arguments"]["action"] != "list"]


@CASES
def story_cases(document) -> list[dict]:
    """``word_headers_footers``: headers and footers set (one line, several, first-page,
    even), removed, linked and unlinked, on every section -- untracked and tracked."""
    out = []
    for section in [s.id for s in document.sections()][:3] + ["s:1"]:
        for which in ("header", "footer"):
            out += [call("word_headers_footers", section=section, which=which, action="set", text="Title"),
                    call("word_headers_footers", section=section, which=which, action="set", text="A\nB\nC"),
                    call("word_headers_footers", section=section, which=which, action="set", type="first",
                         text="First"),
                    call("word_headers_footers", section=section, which=which, action="set", type="even", text=""),
                    call("word_headers_footers", section=section, which=which, action="remove"),
                    call("word_headers_footers", section=section, which=which, action="link", type="first"),
                    call("word_headers_footers", section=section, which=which, action="unlink"),
                    call("word_headers_footers", section=section, which=which, action="unlink", type="even")]
    out.append(call("word_headers_footers", section="s:42", which="header", action="set", text="x"))
    fonts = fonts_of(document)
    for section in [s.id for s in document.sections()][:2]:
        out += [{**call("word_headers_footers", section=section, which="footer", action="set", text="Title",
                        page_number="page_x_of_y"), "fonts": fonts},
                {**call("word_headers_footers", section=section, which="header", action="set", text="",
                        page_number="after_text"), "fonts": fonts},
                {**call("word_headers_footers", section=section, which="footer", action="set", type="first",
                        text="A ", page_number="page_x_of_y"), "fonts": fonts}]
    return out + [tracked(case) for case in out if case["arguments"]["action"] == "set"]


@CASES
def move_cases(document) -> list[dict]:
    """``word_move``: blocks and spans moved after and before blocks, to the end, a
    heading's section moved after another's; no-ops and refusals -- untracked and tracked."""
    out = []
    blocks = [b["id"] for b in document.blocks() if b["id"]]
    headings = [b["id"] for b in document.blocks() if b.get("level")]
    if len(blocks) > 3:
        out += [call("word_move", blocks=blocks[0], after=blocks[2]),
                call("word_move", blocks=blocks[2], before=blocks[0]),
                call("word_move", blocks=f"{blocks[0]}..{blocks[1]}", after=blocks[-1]),
                call("word_move", blocks=blocks[1], after="end"),
                call("word_move", blocks=blocks[1], after=f"before:{blocks[3]}"),
                call("word_move", blocks=blocks[1], after=blocks[0]),
                call("word_move", blocks=blocks[0], after=blocks[0]),
                call("word_move", blocks=f"{blocks[0]}..{blocks[2]}", after=blocks[1]),
                call("word_move", blocks=blocks[0], after=blocks[1], before=blocks[2]),
                call("word_move", blocks=blocks[-1], before=blocks[0])]
    for heading in headings[:3]:
        out.append(call("word_move", section_heading=heading, after=blocks[-1]))
        for other in headings[:3]:
            if other != heading:
                out.append(call("word_move", section_heading=heading, after_section=other))
    if blocks:
        out.append(call("word_move", section_heading=blocks[0], after=blocks[-1]))
    return out + [tracked(case) for case in out]


@CASES
def comment_cases(document) -> list[dict]:
    """``word_comments`` edit (lane P4b, P4a's deferred): each thread's comments and their
    replies rewritten, in one line and several; a comment that does not exist."""
    out = [call("word_comments", action="edit", items=[{"comment": "c:00000000", "text": "x"}])]
    for thread in document.comments(replies=False)[:3]:
        out += [call("word_comments", action="edit", items=[{"comment": thread.id, "text": "Rewritten"}]),
                call("word_comments", action="edit", items=[{"comment": thread.id, "text": "One\nTwo"}])]
        for reply in thread.replies[:1]:
            out.append(call("word_comments", action="edit", items=[{"comment": reply.id, "text": ""},
                                                                    {"comment": thread.id, "text": "Both"}]))
    return out


def fonts_of(document) -> dict:
    """The fonts docx-agent lays ``document`` out with, as docx2svg hands them to the Rust
    layout (``ooxml_core.WordFonts``): the Rust side lays out with the same."""
    import docx2svg
    from docx2svg.rust import _fonts_key
    from docx_agent.layout import layout_options

    options = docx2svg.ConvertOptions(**layout_options(document, {}))
    folders, substitutes, substitute = _fonts_key(options)
    return {"folders": [list(f) for f in folders], "substitutes": [list(s) for s in substitutes],
            "substitute": substitute}


@CASES
def field_cases(document) -> list[dict]:
    """``word_fields`` (lane P4b2): the list; fields inserted at positions, after text found
    and beside blocks (page numbers, SEQ, REF, DATE); dates; cross-references to text and
    page; captions; tables of contents; every result updated, some, and a field that does
    not exist -- untracked and tracked."""
    fonts = fonts_of(document)
    out = [call("word_fields", action="list")]
    paragraphs = _words(document)
    for pid, text in paragraphs[:3]:
        out += [call("word_fields", action="insert_field", at=f"{pid}@{min(2, len(text))}", instruction="PAGE"),
                call("word_fields", action="insert_field", at=pid, instruction="NUMPAGES"),
                call("word_fields", action="insert_field", at=f"{pid}@0", instruction="SEQ Figure \\* ARABIC"),
                call("word_fields", action="insert_date", at=f"{pid}@{len(text)}"),
                call("word_fields", action="insert_field", find=text.split()[0], instruction="SECTIONPAGES")]
    if paragraphs:
        pid, text = paragraphs[0]
        out += [call("word_fields", action="insert_field", after=pid, instruction="PAGE"),
                call("word_fields", action="insert_field", before=pid, instruction="PAGE"),
                call("word_fields", action="insert_field", at=pid, instruction="TOC"),
                call("word_fields", action="insert_field", at=pid),
                call("word_fields", action="insert_field", at=f"{pid}@{len(text) + 4}", instruction="PAGE"),
                call("word_fields", action="insert_cross_reference", at=pid, bookmark="NoSuch"),
                call("word_fields", action="insert_caption", after=pid, label="Table", text=": Results"),
                call("word_fields", action="insert_caption", before=pid),
                call("word_fields", action="insert_caption", after=pid, label="1x"),
                call("word_fields", action="insert_toc", after=pid),
                call("word_fields", action="insert_toc", before=pid, levels="1-2"),
                call("word_fields", action="insert_toc", after=pid, levels="3-1")]
    for mark in document.bookmarks(hidden=True)[:2]:
        if paragraphs:
            pid = paragraphs[-1][0]
            out += [call("word_fields", action="insert_cross_reference", at=f"{pid}@1", bookmark=mark.name),
                    call("word_fields", action="insert_cross_reference", at=pid, bookmark=mark.name, kind="page"),
                    call("word_fields", action="insert_field", at=pid, instruction=f"REF {mark.name} \\h")]
    blocks = [b["id"] for b in document.blocks() if b["id"]]
    tables = [b for b in blocks if b.startswith("t:")]
    for table in tables[:1]:
        out.append(call("word_fields", action="insert_caption", before=table, label="Table"))
    if blocks:
        out.append(call("word_fields", action="insert_toc", after="end"))
    ids = [f.id for f in document.fields()]
    out += [call("word_fields", action="update"),
            call("word_fields", action="update", ids=["fld:body/999"])]
    if ids:
        out.append(call("word_fields", action="update", ids=ids[:2]))
    out += [tracked(case) for case in out if case["arguments"]["action"] not in ("list", "update")]
    return [{**case, "fonts": fonts} for case in out]


def cases(document) -> list[dict]:
    """Every structure case for ``document``."""
    found = CASES.of(document)
    wanted = os.environ.get("STRUCTURE_TOOLS")       # a comma list, to run some tools only
    found = [c for c in found if not wanted or c["tool"] in wanted.split(",")]
    per_tool = int(os.environ.get("STRUCTURE_PER_TOOL") or 0)   # the first n of each tool's
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


def python_case(data: bytes, case: dict, baseline: list, toolbox, reply_of, parts_of, order_of, touched_of) -> dict:
    """One case through docx-agent's toolbox: the reply and what the document became."""
    session = toolbox.session(clock=lambda: _NOW)
    doc = session.open(data, name="case.docx")
    if case.get("reference_copy") or case.get("reference_file"):     # d2, which the call names
        other = open(case["reference_file"], "rb").read() if case.get("reference_file") else data
        session.open(other, name="copy.docx")
    if case.get("tracked"):
        session.entry(doc).tracking = {"on": True, "author": case["tracked"]["author"]}
    arguments = {**case["arguments"], "doc": doc}
    reply = json.loads(json.dumps(toolbox.dispatch(session, case["tool"], arguments).to_json()))
    document = session.entry(doc).document
    out = session.save(doc).data
    before, after = parts_of(data), parts_of(out)
    parts = {name: body.decode("utf-8", "replace") for name, body in after.items() if before.get(name) != body}
    main = document.package.document_part()
    problems = _validate.check(document.package)
    rewritten = dict(before)
    if main in before:
        rewritten[main] = serialize(etree.fromstring(before[main], etree.XMLParser(huge_tree=True)))
    reserialized = [main] if main in after and rewritten.get(main) == after[main] and before[main] != after[main] \
        else []
    reserialized += _reserialized(before, after, main)
    pool = copy.deepcopy(document._id_pool())
    result = {
        "structure": True,
        "reply": reply_of(reply),
        "parts": parts,
        "order": order_of(out),
        "touched": touched_of(document, before, after, main),
        "touched_as_written": touched_of(document, rewritten, after, main),
        "reserialized": reserialized,
        "validation": compare_problems(baseline, problems, key=lambda p: (p.code, p.part, str(p))),
        "problems": [str(p) for p in problems],
        "next": {seed: _ids.generate(seed, pool, 1) for seed in (main, main + "\0text")},
    }
    session.close(doc)
    return result


def _reserialized(before: dict, after: dict, main: str) -> list[str]:
    """The XML parts other than ``main`` docx-agent wrote again unchanged but for lxml's
    serialization (a part it marks dirty without changing it)."""
    out = []
    for name, body in after.items():
        if name == main or name not in before or before[name] == body:
            continue
        try:
            again = serialize(etree.fromstring(before[name], etree.XMLParser(huge_tree=True)))
        except etree.XMLSyntaxError:
            continue
        if again == body:
            out.append(name)
    return out


def settle(want: dict, have: dict) -> None:
    """``want`` as the Rust side is compared with it: a part docx-agent wrote again unchanged
    but for lxml's serialization dropped when the Rust core did not write it, and
    ``touched`` read from the body before as lxml writes it when that is what equals."""
    written = want.pop("touched_as_written", None)
    gone = [name for name in want.pop("reserialized", []) if name not in have.get("parts", {})]
    for name in gone:
        want["parts"].pop(name, None)
    # A part only re-serialized is not touched either (docx-agent's touched compares bytes).
    for key in ("touched",):
        if isinstance(want.get(key), list):
            want[key] = [t for t in want[key] if t.get("part") not in gone or "heading" in t]
    if isinstance(written, list):
        written = [t for t in written if t.get("part") not in gone or "heading" in t]
    if have.get("touched") != want.get("touched") and have.get("touched") == written:
        want["touched"] = written
    # An update's timing is the application's alone (section 9).
    for side in (want, have):
        data = (side.get("reply") or {}).get("data")
        if isinstance(data, dict):
            data.pop("timing", None)
    # A refused call changes nothing on either side, but docx-agent's id pool keeps the ids
    # its stamping drew before the refusal (the edit is rolled back, the pool is not).
    if (want.get("reply") or {}).get("error") and (have.get("reply") or {}).get("error") \
            and not want.get("parts") and not have.get("parts"):
        want["next"] = have.get("next")
