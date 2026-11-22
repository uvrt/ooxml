"""docx-agent's side of every Word edits case (``word_edits_diff.py``'s modes): the case run
on the document as opened -- an edit by its ``op``, an accept or reject, a tool called through
docx-agent's toolbox (``word_tool_reference.py`` for the structure, object and style tools) --
and what it reports: the result, every changed part, the part order, ``touched``, the
validation delta, the problems, the next ids, the aliases.

What this computes is what the reference cache keeps (reference_cache.py), and this module is
in its key; it holds no case making and no comparison: those (word_edits_diff.py,
word_*_cases.py, word_edits_compare.py) run again on every run and the cache does not key on
them, so a new case leaves every kept result valid.  The harness (word_edits_diff.py) imports
this only after it has chosen the reference's packages and kept the Rust core out
(reference_packages.py, reference_guard.py).

Python's behaviour is not changed: this calls docx-agent's public and documented internal
functions only.
"""

from __future__ import annotations

import copy
import io
import json
import zipfile

import word_tool_reference
from docx_agent import Document
from docx_agent import validate as _validate
from docx_agent.edit import ids as _ids
from docx_agent.tools.touched import body_sections
from lxml import etree
from ooxml_edit.tools.savegate import compare_problems
from ooxml_edit.xml import serialize
from word_tool_reference import reply as _reply


def _parts(data: bytes) -> dict[str, bytes]:
    with zipfile.ZipFile(io.BytesIO(data)) as archive:
        return {name: archive.read(name) for name in archive.namelist() if not name.endswith("/")}


def _order(data: bytes) -> list[str]:
    with zipfile.ZipFile(io.BytesIO(data)) as archive:
        return [n for n in archive.namelist() if not n.endswith("/")]


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
    if op == "set_text_styled":
        return _styled(document, case)
    if op == "insert_markdown":
        return document.insert_markdown(case["markdown"], at=case["target"], html=case.get("html", "refuse"),
                                        emphasis=case.get("emphasis", "styles"), style_map=case.get("style_map"))
    if op == "delete":
        return document.delete_block(case["target"])
    if op == "set_cells":
        return document._fill_cells(case["target"], [tuple(w) for w in case["writes"]])
    if op == "replace":
        return document.replace(case["find"], case["text"], case=case.get("case", True),
                                count=case.get("count"), keep=case.get("keep", "first"),
                                regex=case.get("regex", False), whitespace=case.get("whitespace"),
                                across_paragraphs=case.get("across", False))
    return document._set_text(case["target"], case["text"])


def _styled(document, case: dict) -> dict:
    """One ``word_set_text`` item as the tool writes it (``tools/text.py``: ``_plan``,
    ``_set_one``, in the call's batch), its results as the harness compares them."""
    from docx_agent.tools.text import _plan, _set_one

    class Call:
        pass

    call = Call()
    call.document = document
    plan = _plan(call, case["target"], case["text"], case.get("style"), "")
    with document.batch():
        edits, runs, notes, written = _set_one(call, plan)
    return {"edits": [{"id": e.id, "changed": e.changed, "renamed": dict(e.renamed), "created": list(e.created),
                       "removed": list(e.removed)} for e in edits],
            "styled": runs, "notes": notes, "written": [document.get(w).id for w in written]}


def _result(case: dict, result) -> dict:
    op = case.get("op", "set_text")
    if op in ("accept", "reject", "set_text_styled"):
        return result
    if op == "set_text":
        return {"id": result.id, "changed": result.changed, "renamed": dict(result.renamed)}
    if op == "set_paragraph_style":
        return {"id": result.id, "renamed": dict(result.renamed)}
    if op == "delete":
        return {"id": result.id, "renamed": dict(result.renamed), "removed": list(result.removed),
                **({"warnings": list(result.warnings)} if result.warnings else {})}
    if op == "set_cells":
        edits, ids = result
        return {"ids": ids, "edits": [{"id": e.id, "renamed": dict(e.renamed), "created": list(e.created),
                                       "removed": list(e.removed)} for e in edits]}
    if op == "insert_markdown":
        return {"id": result.id, "changed": result.changed, "created": list(result.created),
                "renamed": dict(result.renamed), "removed": list(result.removed),
                "warnings": list(result.warnings), "blocks": list(result.blocks)}
    if op in ("set_text_paragraphs", "insert_paragraph"):
        return {"id": result.id, "changed": result.changed, "renamed": dict(result.renamed),
                "created": list(result.created)}
    return {"id": result.id, "count": result.count, "renamed": dict(result.renamed)}


_TOOLBOX = None


def _toolbox():
    global _TOOLBOX
    if _TOOLBOX is None:
        from docx_agent.tools import FORMAT, GROUPS, TOOLS
        from ooxml_edit.tools import Toolbox

        _TOOLBOX = Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS)
    return _TOOLBOX


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


def python_case(data: bytes, case: dict, baseline: list) -> dict:
    if case.get("op") == "tool":
        return python_tool_case(data, case)
    if case.get("op") in ("structure", "objects", "style"):
        return word_tool_reference.python_case(data, case, baseline, _toolbox(), _parts, _order, _touched)
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
        "aliases": dict(sorted(document.aliases.items())),
        **_replaced(document),
    }


def _replaced(document) -> dict:
    """The blocks replaced one by one (``replace:``) and what replaced each, where any was."""
    replaced = document._ledger.replaced
    return {"replaced": {f"{value:08X}": new for value, new in sorted(replaced.items())}} if replaced else {}


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
