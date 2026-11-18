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
* ``next``: the value the main part's sequence and its text-id sequence give next.

Every difference is printed with its JSON path; the exit status is 1 when there is any.

    python crates/ooxml-oracle/python/word_edits_diff.py --rust target/release/word-edits --tree DIR

Python's behaviour is not changed: this calls docx-agent's public and documented internal
functions only.
"""

from __future__ import annotations

import argparse
import json
import copy
import io
import sys
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
for package in ("docx-agent", "docx2svg", "ooxml-edit", "ooxml-common"):
    source = ROOT / "python" / package / "src"
    if str(source) not in sys.path:
        sys.path.insert(0, str(source))
sys.path.insert(0, str(Path(__file__).resolve().parent))

from docx_agent import Document  # noqa: E402
from docx_agent import validate as _validate  # noqa: E402
from docx_agent.edit import ids as _ids  # noqa: E402
from docx_agent.tools.touched import body_sections  # noqa: E402
from lxml import etree  # noqa: E402
from ooxml_edit.tools.savegate import compare_problems  # noqa: E402
from ooxml_edit.xml import serialize  # noqa: E402
from word_reads_diff import differences, files_of, report, run_rust  # noqa: E402
import word_structure_cases  # noqa: E402

TRACKING = {"author": "Writer A", "date": "2026-11-17T10:00:00Z"}
#: Marks the styled reading of word_set_text takes as emphasis; the plain path only here.
_STYLED = ("*", "\\", "]{")


def _parts(data: bytes) -> dict[str, bytes]:
    with zipfile.ZipFile(io.BytesIO(data)) as archive:
        return {name: archive.read(name) for name in archive.namelist() if not name.endswith("/")}


def _order(data: bytes) -> list[str]:
    with zipfile.ZipFile(io.BytesIO(data)) as archive:
        return [n for n in archive.namelist() if not n.endswith("/")]


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


class _Call:
    """What docx-agent's review selection reads of a tool call: refs (none here) and the
    document's id in a refusal's way out."""

    class entry:  # noqa: N801 -- the attribute docx-agent reads
        doc_id = "d1"

    def resolve_ref(self, name):
        raise KeyError(name)


def _review(document, case: dict):
    """An accept or reject as ``word_changes`` makes it: its selection and its settling
    (``tools/review.py``), the results merged as the tool merges them."""
    from docx_agent.tools.review import _selection, _settle

    action = case["op"]
    call = _Call()
    how, what = _selection(call, document, case.get("ids"), case.get("author"), case.get("kind"),
                           case.get("within"), bool(case.get("all")))
    before = len(document.revisions())
    edits = _settle(document, action, how, what, case.get("author"), case.get("kind"))
    removed, renamed, warnings = [], {}, []
    for edit in edits:
        if edit is None:
            continue
        removed += [r for r in edit.removed if r not in removed]
        renamed.update(edit.renamed)
        warnings += [w for w in edit.warnings if w not in warnings]
    return {"records": before - len(document.revisions()), "removed": removed, "renamed": renamed,
            "warnings": warnings}


def _run(document, case: dict):
    """The case's edit, by its ``op``."""
    op = case.get("op", "set_text")
    if op in ("accept", "reject"):
        return _review(document, case)
    if op == "insert_text":
        return document.insert_text(case["target"], case["text"])
    if op == "delete_text":
        return document.delete_text(case["target"])
    if op == "set_text_paragraphs":
        return document.set_text_paragraphs(case["target"], case["text"])
    if op == "insert_paragraph":
        where = {"after": case["target"]} if case.get("after", True) else {"before": case["target"]}
        return document.insert_paragraph(case["text"], style=case.get("style"), **where)
    if op == "set_paragraph_style":
        return document.set_paragraph_style(case["target"], case.get("style"))
    if op == "delete":
        return document.delete_block(case["target"])
    if op == "replace":
        return document.replace(case["find"], case["text"], case=case.get("case", True),
                                count=case.get("count"), keep=case.get("keep", "first"),
                                regex=case.get("regex", False), whitespace=case.get("whitespace"),
                                across_paragraphs=case.get("across", False))
    return document._set_text(case["target"], case["text"])


def _result(case: dict, result) -> dict:
    op = case.get("op", "set_text")
    if op in ("accept", "reject"):
        return result
    if op == "set_text":
        return {"id": result.id, "changed": result.changed, "renamed": dict(result.renamed)}
    if op == "set_paragraph_style":
        return {"id": result.id, "renamed": dict(result.renamed)}
    if op == "delete":
        return {"id": result.id, "renamed": dict(result.renamed), "removed": list(result.removed)}
    if op in ("set_text_paragraphs", "insert_paragraph"):
        return {"id": result.id, "changed": result.changed, "renamed": dict(result.renamed),
                "created": list(result.created)}
    return {"id": result.id, "count": result.count, "renamed": dict(result.renamed)}


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


_TOOLBOX = None


def _toolbox():
    global _TOOLBOX
    if _TOOLBOX is None:
        from docx_agent.tools import FORMAT, GROUPS, TOOLS
        from ooxml_edit.tools import Toolbox

        _TOOLBOX = Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS)
    return _TOOLBOX


def _reply(reply: dict) -> dict:
    """What a reply says that both sides must say alike: an error's code, message and
    field (the way out is Rust's rule-2 addition); else the summary, data, ids and warnings."""
    if reply.get("error"):
        error = reply["error"]
        return {"error": {k: error.get(k) for k in ("code", "message", "field")}}
    return {k: reply.get(k) for k in ("summary", "data", "changed", "created", "removed", "renamed", "warnings",
                                      "total", "next_cursor") if reply.get(k) not in (None, [], {})}


def python_tool_case(data: bytes, case: dict) -> dict:
    toolbox = _toolbox()
    session = toolbox.session()
    doc = session.open(data, name="case.docx")
    arguments = {**case["arguments"], "doc": doc}
    reply = json.loads(json.dumps(toolbox.dispatch(session, case["tool"], arguments).to_json()))
    out = session.save(doc).data
    session.close(doc)
    before, after = _parts(data), _parts(out)
    parts = {name: body.decode("utf-8", "replace") for name, body in after.items() if before.get(name) != body}
    return {"reply": _reply(reply), "parts": parts, "order": _order(out)}


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
    return list({c["target"]: c for c in cases}.values())


def python_case(data: bytes, case: dict, baseline: list) -> dict:
    if case.get("op") == "tool":
        return python_tool_case(data, case)
    if case.get("op") == "structure":
        return word_structure_cases.python_case(data, case, baseline, _toolbox(), _reply, _parts, _order, _touched)
    document = Document.open(data)
    before = _parts(data)
    try:
        if case["tracked"]:
            with document.tracking(**case["tracked"]):
                result = _run(document, case)
        else:
            result = _run(document, case)
    except Exception as error:  # noqa: BLE001 -- a test instrument: a refusal is compared as one
        return {"refused": f"{type(error).__name__}: {error}"}
    out = document.to_bytes()
    after = _parts(out)
    parts = {name: body.decode("utf-8", "replace") for name, body in after.items() if before.get(name) != body}
    main = document.package.document_part()
    touched = _touched(document, before, after, main)
    # The body as lxml writes it before the edit: docx-agent compares bytes, so a part not
    # written in that form before reports blocks only re-serialized (namespace
    # declarations moved before attributes) as touched.
    rewritten = dict(before)
    if main in before:
        rewritten[main] = serialize(etree.fromstring(before[main], etree.XMLParser(huge_tree=True)))
    as_written = _touched(document, rewritten, after, main)
    # A part docx-agent wrote again unchanged but for lxml's serialization: an edit that
    # changed nothing writes it, the Rust core does not.
    reserialized = [main] if main in after and rewritten.get(main) == after[main] and before[main] != after[main] \
        else []
    problems = _validate.check(document.package)
    validation = compare_problems(baseline, problems, key=lambda p: (p.code, p.part, str(p)))
    pool = copy.deepcopy(document._id_pool())
    next_ids = {seed: _ids.generate(seed, pool, 1) for seed in (main, main + "\0text")}
    return {
        "result": _result(case, result),
        "parts": parts,
        "order": _order(out),
        "touched": touched,
        "touched_as_written": as_written,
        "reserialized": reserialized,
        "validation": validation,
        "problems": [str(p) for p in problems],
        "next": next_ids,
    }


def _touched(document, before: dict, after: dict, main: str) -> list[dict]:
    touched: list[dict] = []
    for name in sorted(set(before) | set(after)):
        if before.get(name) == after.get(name):
            continue
        if name == main and name in before and name in after:
            found = body_sections(document, before[name], after[name])
            if found is not None:
                touched += [{"part": name, **item} for item in found]
                continue
        touched.append({"part": name})
    return touched


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--rust", required=True, help="the word-edits binary")
    parser.add_argument("--tree", action="append", help="every .docx below this folder")
    parser.add_argument("--limit", type=int, default=0, help="at most this many cases per document")
    parser.add_argument("--ops", action="store_true", help="insert_text, delete_text and replace too")
    parser.add_argument("--only-ops", action="store_true", help="insert_text, delete_text and replace only")
    parser.add_argument("--review", action="store_true", help="accept and reject only (word_changes)")
    parser.add_argument("--tools", action="store_true", help="the review tools called as tools only")
    parser.add_argument("--delete", action="store_true", help="block deletions only")
    parser.add_argument("--structure", action="store_true", help="the structure tools called as tools only")
    parser.add_argument("--paragraphs", action="store_true",
                        help="set_text in lines and insert_paragraph (only these, with --only-ops none)")
    parser.add_argument("files", nargs="*")
    args = parser.parse_args(argv)
    expected: dict = {}
    manifest = []
    for path in files_of(args):
        data = path.read_bytes()
        try:
            document = Document.open(data)
            baseline = _validate.check(document.package)
            cases = []
            targets = _targets(document)
            if args.structure:
                cases, targets = word_structure_cases.cases(document), []
            elif args.review or args.tools:
                found = _tool_cases(document) if args.tools else _review_cases(document)
                cases = [{**case, "tracked": None} for case in found]
                targets = []
            if not args.only_ops and not args.paragraphs and not args.delete and not (args.review or args.tools
                                                                                      or args.structure):
                for target in targets:
                    text = document.paragraph(target).text
                    for new in _texts(text):
                        for tracked in (None, TRACKING):
                            cases.append({"target": target, "text": new, "tracked": tracked})
            if args.delete:
                cases = [{**case, "tracked": tracked} for case in _delete_cases(document, targets)
                         for tracked in (None, TRACKING)]
            elif args.paragraphs:
                for case in _paragraph_cases(document, targets):
                    for tracked in (None, TRACKING):
                        cases.append({**case, "tracked": tracked})
            if (args.ops or args.only_ops) and not args.paragraphs and not args.delete and not (args.review or args.tools):
                for case in _range_cases(document, targets):
                    for tracked in (None, TRACKING):
                        cases.append({**case, "tracked": tracked})
            if args.limit:
                cases = cases[:args.limit]
        except Exception as error:  # noqa: BLE001
            print(f"{path}: python failed: {error}")
            continue
        for k, case in enumerate(cases):
            expected[(str(path), k)] = python_case(data, case, baseline)
        manifest.append({"file": str(path), "cases": cases})
    rust = run_rust(args.rust, manifest)
    got = {key: value.get("rust") for key, value in rust.items()}
    rewritten = 0
    for key, want in expected.items():
        have = got.get(key)
        if want.pop("structure", False) and isinstance(have, dict):
            word_structure_cases.settle(want, have)
        if isinstance(have, dict) and "reply" in have:
            if have.get("unported"):
                got[key] = want      # the host answers it its own way (Done::unported)
                continue
            have["reply"] = _reply(have["reply"])
            have.pop("unported", None)
            _shorten(want["parts"], have["parts"])
            continue
        written = want.pop("touched_as_written", None)
        reserialized = want.pop("reserialized", [])
        if isinstance(have, dict) and "parts" in have and "parts" in want:
            for name in reserialized:
                if name not in have["parts"]:
                    want["parts"].pop(name, None)   # no change, only lxml's serialization
                    if have.get("touched") == written:
                        want["touched"] = written
        if isinstance(have, dict) and "refused" in have and "refused" in want:
            got[key] = want      # both refused: the messages are each side's own words
        elif isinstance(have, dict) and "parts" in have and "parts" in want:
            _shorten(want["parts"], have["parts"])
            if have.get("touched") != want["touched"] and have.get("touched") == written:
                rewritten += 1   # equal once the body before is as lxml writes it
                want["touched"] = written
    if rewritten:
        print(f"{rewritten} cases where docx-agent's touched counts blocks only re-serialized (equal "
              "once the body before is read as lxml writes it)")
    return report(expected, got)


def _shorten(want: dict, have: dict, context: int = 160) -> None:
    """Two parts that differ, cut to the stretch around their first difference."""
    for name in set(want) & set(have):
        a, b = want[name], have[name]
        if a == b:
            continue
        at = next((k for k, (x, y) in enumerate(zip(a, b)) if x != y), min(len(a), len(b)))
        start = max(0, at - context)
        want[name] = f"@{at}: {a[start:at + context]}"
        have[name] = f"@{at}: {b[start:at + context]}"


if __name__ == "__main__":
    sys.exit(main())
