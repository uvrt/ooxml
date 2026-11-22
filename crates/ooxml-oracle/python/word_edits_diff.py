#!/usr/bin/env python3
"""The Word edits' differential harness (test support; phase P2, lanes P2a and P2b).

Runs ``set_text`` -- with ``--ops``, ``insert_text``, ``delete_text`` and ``replace``; with
``--paragraphs``, ``set_text`` in lines and ``insert_paragraph`` (lane P2b) -- in docx-agent and in the Rust core (``word-edits``) on the same documents,
untracked and tracked, each case on the document as opened, and compares:

* ``result``: the paragraph's id after the edit, whether it changed, the renames its
  stamping made (in order);
* ``parts``: every part whose bytes changed, byte for byte as each side writes it, and the
  archive's part order;
* ``touched``: the parts and the body's heading sections the change touched;
* ``validation``: the problems after the edit against those the document opened with
  (``new``, ``fixed``, ``baseline``), as the save gate compares them;
* ``next``: the value the main part's sequence and its text-id sequence give next;
* ``aliases``: the old ids the document still follows to new ones (renames, less those of
  blocks removed); ``replaced``, where a block was replaced by one, the block that did.

Every difference is printed with its JSON path; the exit status is 1 when there is any.

    python crates/ooxml-oracle/python/word_edits_diff.py --rust target/release/word-edits --tree DIR

Python's behaviour is not changed: this calls docx-agent's public and documented internal
functions only.
"""

from __future__ import annotations

import argparse
import copy
import io
import json
import sys
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(Path(__file__).resolve().parent))
# The reference's packages: installed from the frozen commit, or the checkout's (reference_packages.py).
from reference_packages import use_sources  # noqa: E402

use_sources("docx-agent", "docx2svg", "ooxml-edit", "ooxml-common")

# The reference is docx-agent in Python: the Rust core kept out of this process and its workers
# (spawn imports this module there too) before docx-agent is imported (reference_guard.py).
from reference_guard import guard  # noqa: E402

guard()

from docx_agent import Document  # noqa: E402
from docx_agent import validate as _validate  # noqa: E402
from reference_cache import Store, compact  # noqa: E402
from word_edits_compare import settle as _compare  # noqa: E402
from word_edits_reference import python_case  # noqa: E402
from word_reads_diff import differences, files_of, report, run_rust  # noqa: E402
import word_object_cases  # noqa: E402
import word_structure_cases  # noqa: E402
import word_text_cases  # noqa: E402
import word_style_cases  # noqa: E402

TRACKING = {"author": "Writer A", "date": "2026-11-17T10:00:00Z"}
#: Marks the styled reading of word_set_text takes as emphasis; the plain path only here.
_STYLED = ("*", "\\", "]{")


def _texts(text: str) -> list[str]:
    """The new texts tried on a paragraph holding ``text``."""
    words = text.split()
    out = []
    if words:
        out.append(text.replace(words[0], "Changed", 1) + " more")
        out.append(" ".join(reversed(words)))
    out += ["A new sentence, written whole.", "", "Tab\tthen\vbreak", text]
    return [t for t in out if not any(m in t for m in _STYLED)]


def _targets(document) -> list[str]:
    """Paragraphs to edit: the first body paragraph with text, one in a table, one in a
    header or note, one with a field or hyperlink, the last body paragraph."""
    found: list[str] = []
    body = document._index(document.package.document_part())
    paragraphs = [e for e in body.paragraphs if _text_of(e)]
    if paragraphs:
        found.append(paragraphs[0].id)
        found.append(paragraphs[-1].id)
        mid = paragraphs[len(paragraphs) // 2]
        found.append(mid.id)
    for entry in body.paragraphs:
        if entry.element.getparent() is not None and entry.element.getparent().tag.endswith("}tc") and _text_of(entry):
            found.append(entry.id)
            break
    for entry in body.paragraphs:
        xml = entry.element
        if any(n.tag.endswith(("}hyperlink", "}fldSimple", "}fldChar")) for n in xml.iter()):
            found.append(entry.id)
            break
    for part in document._parts()[1:]:
        index = document._index(part)
        texts = [e for e in index.paragraphs if _text_of(e)]
        if texts:
            found.append(texts[0].id)
            break
    return list(dict.fromkeys(found))


def _text_of(entry) -> str:
    from docx_agent.edit import text as _text

    return _text.paragraph_text(entry.element)


def _range_cases(document, targets: list[str]) -> list[dict]:
    """Insertions, deletions and replacements on the targets' text (``--ops``)."""
    cases: list[dict] = []
    for target in targets:
        text = document.paragraph(target).text
        words = text.split()
        for at in sorted({0, len(text) // 2, len(text)}):
            cases.append({"op": "insert_text", "target": f"{target}@{at}", "text": "inserted words "})
        if words:
            start = text.index(words[0])
            cases.append({"op": "delete_text", "target": f"{target}@{start}:{start + len(words[0])}"})
        if text:
            cases.append({"op": "delete_text", "target": target})
    # ranges across paragraphs: from a target's middle into the next paragraphs
    body = [e for e in document._index(document.package.document_part()).paragraphs]
    for target in targets[:3]:
        entry = next((e for e in body if e.id == target), None)
        if entry is None:
            continue
        for span in (1, 2):
            if entry.index + span >= len(body):
                continue
            last = body[entry.index + span]
            first_text = document.paragraph(entry.id).text
            last_text = document.paragraph(last.id).text
            start, end = len(first_text) // 2, len(last_text) // 2
            cases.append({"op": "delete_text", "target": f"{entry.id}@{start}..{last.id}@{end}"})
    for target in targets[:1]:
        words = document.paragraph(target).text.split()
        if len(words) > 1:
            cases.append({"op": "replace", "find": f"{words[0]}   {words[1]}", "text": "collapsed",
                          "whitespace": "collapse"})
            cases.append({"op": "replace", "find": r"(\w+)\s+(\w+)", "text": r"\2 \1", "regex": True, "count": 2})
            cases.append({"op": "replace", "find": r"(?P<w>\w+)\.", "text": r"[\g<w>]", "regex": True, "count": 3})
            cases.append({"op": "replace", "find": r"\w+\n\w+", "text": "joined", "regex": True,
                          "across": True, "count": 2})
        if words:
            cases.append({"op": "replace", "find": words[0], "text": "Replaced", "case": True})
            cases.append({"op": "replace", "find": words[0].upper(), "text": "replaced again", "case": False,
                          "count": 1})
            cases.append({"op": "replace", "find": words[0], "text": words[0] + "s", "keep": "characters"})
    return [c for c in cases if not any(m in c.get("text", "") for m in _STYLED)]


def _review_cases(document) -> list[dict]:
    """Accepts and rejects (``--review``): everything; each of the first changes by its id;
    a later record of a change by its own id; by the first author; by the first change's
    kind; within the first change's paragraph."""
    changes = document.changes()
    if not changes:
        return []
    cases: list[dict] = []
    for op in ("accept", "reject"):
        cases.append({"op": op, "all": True})
        for change in changes[:3]:
            cases.append({"op": op, "ids": [change.id]})
        multi = next((c for c in changes if len(c.revisions) > 1), None)
        if multi is not None:
            cases.append({"op": op, "ids": [multi.revisions[-1]]})
        if changes[0].author:
            cases.append({"op": op, "author": changes[0].author})
        cases.append({"op": op, "kind": changes[0].kind})
        if changes[0].paragraph_ids:
            cases.append({"op": op, "within": changes[0].paragraph_ids[0]})
    cases.append({"op": "accept", "ids": ["rev:999999"]})
    return cases


def _tool_cases(document) -> list[dict]:
    """``word_changes``, ``word_comments`` and ``word_set_tracking`` called as tools
    (``--tools``): lists, settling, a preview, refusals, the tracking mode and Word's switch,
    and comments resolved, reopened and deleted."""
    def call(tool, **arguments):
        return {"op": "tool", "tool": tool, "arguments": {"doc": "d1", **arguments}}

    out = [call("word_changes", action="list"), call("word_changes", action="list", detail="records"),
           call("word_changes", action="accept"), call("word_changes", action="list", all=True),
           call("word_changes", action="reject", ids=["rev:999999"]),
           call("word_set_tracking", on=True, author="Writer A", word_switch=True),
           call("word_set_tracking", on=False, word_switch=False), call("word_set_tracking", on=True),
           call("word_comments", action="list"), call("word_comments", action="list", open_only=True),
           call("word_comments", action="resolve", items=[]), call("word_comments", action="delete"),
           call("word_comments", action="resolve", items=[{"comment": "c:00000000"}])]
    changes = document.changes()
    if changes:
        first = changes[0]
        out += [call("word_changes", action="accept", all=True), call("word_changes", action="reject", all=True),
                call("word_changes", action="accept", ids=[first.id]),
                call("word_changes", action="reject", ids=[first.id], author="x"),
                call("word_changes", action="preview", all=True), call("word_changes", action="preview", ids=[first.id]),
                call("word_changes", action="list", kind=first.kind), call("word_changes", action="list", detail="records",
                                                                           kind=first.kind)]
        if first.author:
            out += [call("word_changes", action="reject", author=first.author),
                    call("word_changes", action="list", author=first.author)]
        if first.paragraph_ids:
            out += [call("word_changes", action="list", within=first.paragraph_ids[0]),
                    call("word_changes", action="accept", within=first.paragraph_ids[0])]
    threads = document.comments(replies=False)
    if threads:
        first = threads[0].id
        out += [call("word_comments", action="resolve", items=[{"comment": first}]),
                call("word_comments", action="reopen", items=[{"comment": first}]),
                call("word_comments", action="delete", items=[{"comment": first}])]
        replies = threads[0].replies
        if replies:
            out.append(call("word_comments", action="resolve", items=[{"comment": replies[0].id}]))
    return out


def _paragraph_cases(document, targets: list[str]) -> list[dict]:
    """New paragraphs and styles (``--paragraphs``): a text in lines on each target, a
    paragraph after and before it (and after and before the first table), styles set (the
    document's, built-in ones it lacks, a list style, ones refused) and paragraphs inserted
    in a style."""
    cases: list[dict] = []
    for target in targets:
        text = document.paragraph(target).text
        cases.append({"op": "set_text_paragraphs", "target": target, "text": "First line\nSecond line"})
        cases.append({"op": "set_text_paragraphs", "target": target, "text": f"{text}\r\n\nA third\vbroken"})
        cases.append({"op": "insert_paragraph", "target": target, "text": "A new paragraph.", "after": True})
        cases.append({"op": "insert_paragraph", "target": target, "text": "", "after": False})
    for target in targets[:2]:
        for style in ("Heading 1", "Normal", None, "Quote", "List Number 3", "Intense Quote", "No Such Style",
                      "Default Paragraph Font"):
            cases.append({"op": "set_paragraph_style", "target": target, "style": style})
        cases.append({"op": "insert_paragraph", "target": target, "text": "Styled", "after": True,
                      "style": "Heading 2"})
        cases.append({"op": "insert_paragraph", "target": target, "text": "Listed", "after": False,
                      "style": "List Bullet 4"})
    tables = document._index(document.package.document_part()).tables
    if tables:
        for after in (True, False):
            cases.append({"op": "insert_paragraph", "target": tables[0].id, "text": "Beside a table", "after": after})
    return [c for c in cases if not any(m in c.get("text", "") for m in _STYLED)]


def _styled_cases(document, targets: list[str]) -> list[dict]:
    """``word_set_text`` with inline emphasis (``--styled``): strong and emphasis, a named
    span (a style the document has, a built-in one it may lack, one refused), lines, a
    paragraph style, a cell's text in lines."""
    cases: list[dict] = []
    for target in targets:
        cases.append({"op": "set_text_styled", "target": target, "text": "Plain **strong** and *emphasis* end"})
        cases.append({"op": "set_text_styled", "target": target, "text": "[Titled words]{Book Title}, then ***both***"})
        cases.append({"op": "set_text_styled", "target": target, "text": "**First** line\nSecond *line*"})
    for target in targets[:2]:
        cases.append({"op": "set_text_styled", "target": target, "text": "A *styled* heading", "style": "Heading 2"})
        cases.append({"op": "set_text_styled", "target": target, "text": "[x]{No Such Style} y"})
        cases.append({"op": "set_text_styled", "target": target, "text": "[Quoted]{style=\"Intense Emphasis\"} and \\*not\\*"})
    tables = document._index(document.package.document_part()).tables
    if tables:
        cases.append({"op": "set_text_styled", "target": f"{tables[0].id}/c0,0", "text": "**Cell** one\nand *two*"})
        cases.append({"op": "set_text_styled", "target": f"{tables[0].id}/c0,0", "text": "Only **one**"})
    return cases


#: Text in French, written by every text edit (``--spacing``): marks with and without a
#: space before them, guillemets, marks between words, code.
_FRENCH = ["Bonjour : le monde ! Il a dit «oui» ; fin?", "Quoi?! « déjà» et 12:30, x?y.", "Tab\tpuis : fin !"]


def _spacing_cases(document, targets: list[str]) -> list[dict]:
    """The spacing written text takes in French (``--spacing``, on documents ``_french``
    made French): set_text, in lines, insert_text, replace, insert_paragraph, styled text,
    Markdown (code keeps its spacing)."""
    cases: list[dict] = []
    for target in targets:
        text = document.paragraph(target).text
        for french in _FRENCH:
            cases.append({"target": target, "text": french})
        cases.append({"op": "set_text_paragraphs", "target": target, "text": "Premier : un\nSecond !"})
        cases.append({"op": "insert_text", "target": f"{target}@{len(text)}", "text": " : ajouté !"})
        cases.append({"op": "insert_text", "target": f"{target}@0", "text": "«Début» ?"})
        cases.append({"op": "insert_paragraph", "target": target, "text": "Nouveau : «x»!", "after": True})
        cases.append({"op": "set_text_styled", "target": target, "text": "Un **mot** : *fin* !"})
        words = [w for w in text.split() if w.isalpha()]
        if words:
            cases.append({"op": "replace", "find": words[0], "text": words[0] + " :", "keep": "characters"})
            cases.append({"op": "replace", "find": words[0], "text": "«" + words[0] + "»"})
    target = targets[0] if targets else None
    if target is not None:
        cases.append({"op": "insert_markdown", "markdown": "# Titre : un\n\nTexte « cité » ; `code : x`!\n\n"
                      "```\nbloc : code !\n```\n\n| A | B |\n|---|---|\n| x : y | «z» |\n",
                      "target": f"after:{target}"})
    tables = document._index(document.package.document_part()).tables
    if tables:
        cases.append({"op": "set_cells", "target": tables[0].id, "writes": [[0, 0, "Cellule : un !"]]})
    return cases


def _french(data: bytes) -> bytes:
    """``data`` in French: every ``w:lang`` of its Word parts ``fr-FR``, and the styles'
    defaults too (added where they state none)."""
    from lxml import etree as _etree

    w = "http://schemas.openxmlformats.org/wordprocessingml/2006/main"
    out = io.BytesIO()
    with zipfile.ZipFile(io.BytesIO(data)) as source, zipfile.ZipFile(out, "w", zipfile.ZIP_DEFLATED) as target:
        for info in source.infolist():
            body = source.read(info.filename)
            if info.filename.startswith("word/") and info.filename.endswith(".xml"):
                root = _etree.fromstring(body, _etree.XMLParser(huge_tree=True))
                for lang in root.iter(f"{{{w}}}lang"):
                    lang.set(f"{{{w}}}val", "fr-FR")
                if root.tag == f"{{{w}}}styles":
                    defaults = root.find(f"{{{w}}}docDefaults")
                    if defaults is None:
                        defaults = _etree.Element(f"{{{w}}}docDefaults")
                        root.insert(0, defaults)
                    holder = defaults.find(f"{{{w}}}rPrDefault")
                    if holder is None:
                        holder = _etree.SubElement(defaults, f"{{{w}}}rPrDefault")
                        defaults.insert(0, holder)
                    rpr = holder.find(f"{{{w}}}rPr")
                    if rpr is None:
                        rpr = _etree.SubElement(holder, f"{{{w}}}rPr")
                    if rpr.find(f"{{{w}}}lang") is None:
                        _etree.SubElement(rpr, f"{{{w}}}lang").set(f"{{{w}}}val", "fr-FR")
                body = _etree.tostring(root, xml_declaration=True, encoding="UTF-8", standalone=True)
            target.writestr(info, body)
    return out.getvalue()


#: insert_markdown's cases (``--markdown``): (Markdown, where, options); ``{t}`` is a
#: paragraph's id. Pictures and footnotes are not ported (the host writes them).
_MARKDOWN = [
    ("# Title\n\nSome *em*, **strong**, `code`, [a link](https://example.com/a) and [an anchor](#top%20x).\n\n"
     "- one\n- two\n  - nested\n\n1. first\n2. second\n", "end", {}),
    ("> quoted\n>\n> - in a quote\n\n```\ncode line\n\nmore\n```\n\n---\n\n: A caption\n", "after:{t}", {}),
    ("| A | B |\n|:-:|--:|\n| 1 | 2 |\n| x |\n\nTable: numbers\n", "before:{t}", {}),
    ("| a |\n|---|\n| 1 |\n\n| b |\n|---|\n| 2 |\n", "end", {}),
    ("| | |\n|-|-|\n| no | header |\n", "{t}", {}),
    ("<div>x</div>\n\ntext <span>y</span>\n", "end", {}),
    ("<div>x</div>\n\ntext <span>y</span> <!-- c -->\n", "after:{t}", {"html": "text"}),
    ("3. three\n4. four\n\n- b\n\n1. again\n", "after:{t}", {}),
    ("# Head\n\nbody\n\n## Sub\n", "end", {"style_map": {"h1": "Title", "paragraph": "Body Text"}}),
    ("***both*** ~~strike~~ line<br>break\nsoft\n", "after:{t}", {"emphasis": "direct"}),
    ("<!-- only a comment -->\n", "end", {}),
    ("Plain line\n", "end:nowhere", {}),
]


def _markdown_cases(document, targets: list[str]) -> list[dict]:
    """``insert_markdown`` (``--markdown``): headings, emphasis, links, lists, quotes, code,
    rules, captions, tables, raw HTML refused or kept, a style map, direct emphasis."""
    target = targets[min(1, len(targets) - 1)] if targets else None
    cases = []
    for markdown, at, options in _MARKDOWN:
        if "{t}" in at and target is None:
            continue
        cases.append({"op": "insert_markdown", "markdown": markdown, "target": at.replace("{t}", target or ""),
                      **options})
    return cases


def _delete_cases(document, targets: list[str]) -> list[dict]:
    """Block deletions (``--delete``): each target paragraph, the paragraph before the last
    in the body, the first and last table, and a paragraph holding a bookmark or comment."""
    body = document._index(document.package.document_part())
    cases = [{"op": "delete", "target": t} for t in targets]
    if len(body.paragraphs) > 1:
        cases.append({"op": "delete", "target": body.paragraphs[-2].id})
    for table in body.tables[:1] + body.tables[-1:]:
        cases.append({"op": "delete", "target": table.id})
    for entry in body.paragraphs:
        if any(n.tag.endswith(("}bookmarkStart", "}commentRangeStart", "}commentReference"))
               for n in entry.element.iter()):
            cases.append({"op": "delete", "target": entry.id})
            break
    # content controls: the first three of the block level, the first of another (refused)
    from docx_agent.edit.annotations import control_level

    levels = [(i, control_level(e) == "block") for _, i, e in document._content_controls()]
    cases += [{"op": "delete", "target": i} for i, block in levels if block][:3]
    cases += [{"op": "delete", "target": i} for i, block in levels if not block][:1]
    return list({c["target"]: c for c in cases}.values())


def _cells_cases(document) -> list[dict]:
    """Cells written in one pass (``--cells``), on each table of the body (first three):
    the first cell's text, the last row's first cell in two lines, the first cell made
    one empty paragraph, and a place the table does not have."""
    cases = []
    for table in document._index(document.package.document_part()).tables[:3]:
        last = len(table.rows) - 1
        cases.append({"op": "set_cells", "target": table.id, "writes": [[0, 0, "Written"], [last, 0, "First\nSecond"]]})
        cases.append({"op": "set_cells", "target": table.id, "writes": [[0, 0, ""]]})
        cases.append({"op": "set_cells", "target": table.id, "writes": [[last + 5, 0, "nowhere"]]})
    return cases


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--rust", required=True, help="the word-edits binary")
    parser.add_argument("--tree", action="append", help="every .docx below this folder")
    parser.add_argument("--limit", type=int, default=0, help="at most this many cases per document")
    parser.add_argument("--ops", action="store_true", help="insert_text, delete_text and replace too")
    parser.add_argument("--only-ops", action="store_true", help="insert_text, delete_text and replace only")
    parser.add_argument("--review", action="store_true", help="accept and reject only (word_changes)")
    parser.add_argument("--tools", action="store_true", help="the review tools called as tools only")
    parser.add_argument("--text", action="store_true", help="the text tools called as tools only (word_text_cases.py)")
    parser.add_argument("--delete", action="store_true", help="block deletions only")
    parser.add_argument("--structure", action="store_true", help="the structure tools called as tools only")
    parser.add_argument("--objects", action="store_true", help="the object tools called as tools only")
    parser.add_argument("--style", action="store_true", help="the style tools called as tools only")
    parser.add_argument("--cells", action="store_true", help="set_cells only")
    parser.add_argument("--styled", action="store_true", help="word_set_text's styled text only")
    parser.add_argument("--markdown", action="store_true", help="insert_markdown only")
    parser.add_argument("--spacing", metavar="DIR",
                        help="the text edits on French copies of the documents, written into DIR")
    parser.add_argument("--paragraphs", action="store_true",
                        help="set_text in lines and insert_paragraph (only these, with --only-ops none)")
    parser.add_argument("--results", help="also write the cases with differences here (JSON lines)")
    parser.add_argument("--cache", metavar="DIR", help="docx-agent's results kept by earlier runs (reference_cache.py)")
    parser.add_argument("--cache-new", metavar="FILE", help="write the results this run computes here, to keep")
    parser.add_argument("--cache-used", metavar="FILE",
                        help="write each document's case keys here (JSON: the store's coverage, tools/gates_store.py)")
    parser.add_argument("files", nargs="*")
    args = parser.parse_args(argv)
    store = Store(args.cache, args.cache_new, lambda a, b: not differences(a, b)) \
        if args.cache or args.cache_new else None
    kept: dict = {}         # the cases whose result is a kept one: what docx-agent needs to run them again
    expected: dict = {}
    inputs: dict = {}
    manifest = []
    used: dict[str, list[str]] = {}     # each document's case keys (--cache-used)
    made: list[tuple] = []              # each document's cases, as made: (path, source, data, baseline, cases)
    for path in files_of(args):
        data = path.read_bytes()
        source = str(path)
        used[source] = []
        if args.spacing:
            data = _french(data)
            path = Path(args.spacing) / f"{len(manifest):04d}-{path.name}"
            path.write_bytes(data)
        try:
            document = Document.open(data)
            baseline = _validate.check(document.package)
            cases = []
            targets = _targets(document)
            if args.text:
                args.tools = True
            if args.structure:
                cases, targets = word_structure_cases.cases(document), []
            elif args.objects:
                cases, targets = word_object_cases.cases(document), []
            elif args.style:
                cases, targets = word_style_cases.cases(document), []
            elif args.review or args.tools:
                found = (word_text_cases.cases(document, targets) if args.text else _tool_cases(document)) \
                    if args.tools else _review_cases(document)
                cases = [{**case, "tracked": None} for case in found]
                targets = []
            if not args.only_ops and not args.paragraphs and not args.delete and not args.cells and not args.styled \
                    and not args.markdown and not args.spacing \
                    and not (args.review or args.tools or args.structure or args.objects or args.style):
                for target in targets:
                    text = document.paragraph(target).text
                    for new in _texts(text):
                        for tracked in (None, TRACKING):
                            cases.append({"target": target, "text": new, "tracked": tracked})
            if args.spacing:
                cases = [{**case, "tracked": tracked} for case in _spacing_cases(document, targets)
                         for tracked in (None, TRACKING)]
            elif args.markdown:
                cases = [{**case, "tracked": tracked} for case in _markdown_cases(document, targets)
                         for tracked in (None, TRACKING)]
            elif args.styled:
                cases = [{**case, "tracked": tracked} for case in _styled_cases(document, targets)
                         for tracked in (None, TRACKING)]
            elif args.cells:
                cases = [{**case, "tracked": tracked} for case in _cells_cases(document)
                         for tracked in (None, TRACKING)]
            elif args.delete:
                cases = [{**case, "tracked": tracked} for case in _delete_cases(document, targets)
                         for tracked in (None, TRACKING)]
            elif args.paragraphs:
                for case in _paragraph_cases(document, targets):
                    for tracked in (None, TRACKING):
                        cases.append({**case, "tracked": tracked})
            if (args.ops or args.only_ops) and not args.paragraphs and not args.delete and not args.cells \
                    and not args.styled and not args.markdown and not args.spacing and not (args.review or args.tools):
                for case in _range_cases(document, targets):
                    for tracked in (None, TRACKING):
                        cases.append({**case, "tracked": tracked})
            if args.limit:
                cases = cases[:args.limit]
        except Exception as error:  # noqa: BLE001
            print(f"{path}: python failed: {error}")
            continue
        if store:
            used[source] = [store.key(data, case) for case in cases]
        made.append((path, source, data, baseline, cases))
        manifest.append({"file": str(path), "cases": cases})
    if store:
        # every case is made before the store is read: it keeps their results only
        store.read({key for keys in used.values() for key in keys})
    for path, source, data, baseline, cases in made:
        for k, case in enumerate(cases):
            found = store.get(used[source][k]) if store else None
            if found is None and store:
                found = store.compute(python_case, data, case, baseline)
            elif found is None:
                found = python_case(data, case, baseline)
            else:
                kept[(str(path), k)] = (data, case, baseline)
            expected[(str(path), k)] = found
            inputs[(str(path), k)] = {**case, "source": source}
    rust = run_rust(args.rust, manifest)
    got = {key: value.get("rust") for key, value in rust.items()}
    counts = {"unported": 0, "rewritten": 0, None: 0, "again": 0}
    for key, want in expected.items():
        have = got.get(key)
        if key in kept:     # compared with the parts as digests; on a difference docx-agent runs it again
            w = copy.deepcopy(want)
            h, tag = _compare(w, compact(copy.deepcopy(have)))
            if h is not None and not differences(w, h):
                expected[key], got[key] = w, h
                counts[tag] += 1
                continue
            # computed again, and kept again: the fresh result replaces the kept one in the store
            # (a kept result that is wrong now, e.g. an error of the run that made it, is not reused)
            want = expected[key] = store.compute(python_case, *kept[key])
            counts["again"] += 1
        got[key], tag = _compare(want, have)
        counts[tag] += 1
    unported, rewritten = counts["unported"], counts["rewritten"]
    cache = {"kept": len(kept) - counts["again"], "computed": len(expected) - len(kept) + counts["again"]} \
        if store else {}
    if store:
        store.close()
    if args.cache_used:
        Path(args.cache_used).write_text(json.dumps(used))
    if unported:
        print(f"{unported} cases the Rust tools left to the host (unported)")
    if rewritten:
        print(f"{rewritten} cases where docx-agent's touched counts blocks only re-serialized (equal "
              "once the body before is read as lxml writes it)")
    return report(expected, got, results=args.results, inputs=inputs,
                  counts={"unported": unported, "rewritten": rewritten, **cache})


if __name__ == "__main__":
    sys.exit(main())
