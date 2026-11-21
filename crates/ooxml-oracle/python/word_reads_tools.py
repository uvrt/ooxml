"""``describe``, ``word_read``, ``word_inspect``, ``find_text``, ``read_batch`` and ``check``
for ``word_reads_diff.py``: docx-agent's toolbox against the Rust tools (``word-reads``), case
by case.  ``describe``'s fields the Rust core does not give yet
(``ooxml_tools_word::NOT_DESCRIBED``) and the application-only ``memory`` are left out of the
comparison, as are ``check``'s seconds (``timing``: its parts' names are compared) and its
``validation`` (the save gate's comparison, which the tool layer adds around either); every other
field must be equal.  ``check``'s cases are the facts the Rust session gives whole (``validate``,
``fields``, ``app``); one it leaves to the host fails as ``unported``.

The addressed cases (``ranges``, ``inspect``, ``find``, ``batch``) are made per document from
what it holds: its first, a middle and its last body paragraph, a span of five blocks, its
first table and that table's first cell and row, its first heading's range, its first
section, its first bookmark, a text range in its first paragraph with text, and a missing id."""

from __future__ import annotations

import json

#: describe's data keys the Rust core does not give (and why: ooxml_tools_word::NOT_DESCRIBED).
NOT_DESCRIBED = {"pages", "problems"}

#: word_read's cases: (name, arguments beyond doc).
READS = (("final", {}), ("markup", {"view": "markup"}), ("original", {"view": "original"}),
         ("all", {"stories": "all"}))

#: check's cases: (name, include).
CHECKS = (("validate", ["validate"]), ("fields", ["fields"]),
          ("validate fields app", ["validate", "fields", "app"]))


def _toolbox():
    from docx_agent.tools import FORMAT, GROUPS, TOOLS
    from ooxml_edit.tools import Toolbox

    return Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS)


def targets(path) -> list[str]:
    """The addresses a document's addressed cases read (module docstring)."""
    from docx_agent import Document
    from docx_agent.edit.outline import body_blocks, headings

    document = Document.open(path.read_bytes())
    out: list[str] = []
    top = [b for b in body_blocks(document) if b.id]
    paragraphs = [p.id for p in document.paragraphs()]
    if paragraphs:
        out += [paragraphs[0], paragraphs[len(paragraphs) // 2], paragraphs[-1]]
    ids = [getattr(b, "id") for b in top]
    if len(ids) >= 5:
        middle = len(ids) // 2
        out.append(f"{ids[middle - 2]}..{ids[middle + 2]}")
        out.append(f"{ids[-2]}..")
    tables = document.tables()
    if tables:
        # (and a cell below the table's last row: refused with what the table has)
        out += [tables[0].id, f"{tables[0].id}/c0,0", f"{tables[0].id}/r0", f"{tables[0].id}/c{len(tables[0].rows)},0"]
    found = headings(document)
    if found:
        out.append(f"h:{found[0].id}")
    out.append("s:1")
    marks = document.bookmarks()
    if marks:
        out.append(f"bm:{marks[0].name}")
    with_text = next((p for p in document.paragraphs() if len(p.text) >= 6), None)
    if with_text is not None:
        out.append(f"{with_text.id}@1:5")
    out.append("p:0BADF00D")
    return list(dict.fromkeys(out))


def needle(path) -> str | None:
    """A word to find: the longest word of the first paragraph with words."""
    from docx_agent import Document

    for paragraph in Document.open(path.read_bytes()).paragraphs():
        words = [w for w in paragraph.text.split() if w.isalpha()]
        if words:
            return max(words, key=len)
    return None


def cases_of(path, what: str) -> list[tuple[str, str, dict]]:
    if what == "describe":
        return [("describe", "describe", {})]
    if what == "read":
        return [(name, "word_read", arguments) for name, arguments in READS]
    if what == "check":
        return [(f"check {name}", "check", {"include": include}) for name, include in CHECKS]
    if what == "ranges":
        return [(f"read {t}", "word_read", {"range": t}) for t in targets(path)]
    if what == "inspect":
        return [(f"inspect {t}", "word_inspect", {"target": t}) for t in ["body", *targets(path)]]
    word = needle(path)
    if what == "find":
        cases = [("find e", "find_text", {"text": "e"}), ("find all e", "find_text", {"text": "e", "stories": "all"}),
                 ("find empty", "find_text", {"text": ""})]
        if word:
            cases.append((f"find {word}", "find_text", {"text": word}))
        tables = targets(path)
        table = next((t for t in tables if t.startswith("t") and "/" not in t), None)
        if table:
            cases.append((f"find e in {table}", "find_text", {"text": "e", "range": table}))
        return cases
    ops = [{"tool": "describe", "arguments": {"doc": "d1"}},
           {"tool": "word_read", "arguments": {"doc": "d1", "range": "p:0BADF00D"}},
           {"tool": "find_text", "arguments": {"doc": "d1", "text": word or "e"}},
           {"tool": "word_inspect", "arguments": {"doc": "d1", "target": (targets(path) or ["body"])[0]}}]
    return [("batch", "read_batch", {"ops": ops})]


def python_replies(toolbox, path, what: str) -> dict:
    session = toolbox.session()
    doc = session.open(path.read_bytes(), name=path.name)
    out = {}
    try:
        for name, tool, arguments in cases_of(path, what):
            reply = json.loads(json.dumps(toolbox.dispatch(session, tool, {"doc": doc, **arguments}).to_json()))
            out[name] = (tool, arguments, reply)
    finally:
        session.close(doc)
    return out


def comparable(tool: str, reply: dict) -> dict:
    if reply.get("error") is not None:
        return {"error": {"code": reply["error"].get("code"), "message": reply["error"].get("message")}}
    data = reply.get("data")
    if tool == "read_batch":
        data = {**data, "ops": [{k: v for k, v in op.items() if k not in ("doc", "version")}
                                for op in data.get("ops", [])]}
        for op in data["ops"]:
            if op.get("tool") == "describe" and isinstance(op.get("data"), dict):
                op["data"] = comparable("describe", {"data": op["data"]})["data"]
            if "error" in op:
                op["error"] = {"code": op["error"].get("code"), "message": op["error"].get("message")}
            op.pop("warnings", None)
    if tool == "describe":
        data = {k: v for k, v in data.items() if k not in NOT_DESCRIBED}
    if tool == "check" and isinstance(data, dict):
        data = {k: v for k, v in data.items() if k != "validation"}
        if isinstance(data.get("timing"), dict):
            data["timing"] = sorted(k for k in data["timing"] if k != "validation")
    out = {"summary": reply.get("summary"), "data": data}
    if tool in ("word_read", "find_text"):
        out["total"] = reply.get("total")
    if tool in ("word_read", "find_text"):
        out["next_cursor"] = reply.get("next_cursor")
    return out


def check_tools(args, run_rust, report, files_of) -> int:
    toolbox = _toolbox()
    expected, manifest = {}, []
    for path in files_of(args):
        try:
            replies = python_replies(toolbox, path, args.what)
        except Exception as error:  # noqa: BLE001 -- a test instrument: say what failed
            print(f"{path}: python failed: {error}")
            continue
        for case, (tool, arguments, reply) in replies.items():
            expected[(str(path), case)] = comparable(tool, reply)
            manifest.append({"file": str(path), "case": case, "tool": tool, "arguments": {"doc": "d1", **arguments}})
    rust = {}
    for key, line in run_rust(args.rust, manifest).items():
        reply = line["reply"]
        tool = next(m["tool"] for m in manifest if (m["file"], m["case"]) == key)
        rust[key] = comparable(tool, reply)
    return report(expected, rust)
