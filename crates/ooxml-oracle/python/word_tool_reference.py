"""docx-agent's side of a Word edits case that calls a tool (``word_edits_diff.py``'s
``--structure``, ``--objects`` and ``--style``): the case run through docx-agent's toolbox on
the document as opened, and what it reports -- the reply as both sides must say it alike,
every changed part, the part order, ``touched``, the validation delta, the problems and the
next ids.  What this computes is what the reference cache keeps (reference_cache.py), so it
holds no case making and no comparison: those (word_*_cases.py, word_edits_compare.py) run
again on every run and the cache does not key on them.

Python's behaviour is not changed: this calls docx-agent's public and documented internal
functions only.
"""

from __future__ import annotations

import copy
import datetime
import json

from docx_agent import validate as _validate
from docx_agent.edit import ids as _ids
from lxml import etree
from ooxml_edit.tools.savegate import compare_problems
from ooxml_edit.xml import serialize

#: The harness's clock (the cases' tracked changes are dated by it).
_NOW = datetime.datetime(2026, 11, 17, 10, 0, 0, tzinfo=datetime.timezone.utc)


def reply(reply: dict) -> dict:
    """What a reply says that both sides must say alike: an error's code, message and
    field (the way out is Rust's rule-2 addition); else the summary, data, ids and warnings."""
    if reply.get("error"):
        error = reply["error"]
        return {"error": {k: error.get(k) for k in ("code", "message", "field")}}
    return {k: reply.get(k) for k in ("summary", "data", "changed", "created", "removed", "renamed", "warnings",
                                      "total", "next_cursor") if reply.get(k) not in (None, [], {})}


def python_case(data: bytes, case: dict, baseline: list, toolbox, parts_of, order_of, touched_of) -> dict:
    """One case through docx-agent's toolbox: the reply and what the document became."""
    session = toolbox.session(clock=lambda: _NOW)
    doc = session.open(data, name="case.docx")
    if case.get("reference_copy") or case.get("reference_file"):     # d2, which the call names
        other = open(case["reference_file"], "rb").read() if case.get("reference_file") else data
        session.open(other, name="copy.docx")
    if case.get("tracked"):
        session.entry(doc).tracking = {"on": True, "author": case["tracked"]["author"]}
    for blob in case.get("blobs") or []:                # b1, b2, ... as the session numbers them
        session.add_blob(bytes.fromhex(blob["hex"]), blob["name"], blob["mime"])
    arguments = {**case["arguments"], "doc": doc}
    answer = json.loads(json.dumps(toolbox.dispatch(session, case["tool"], arguments).to_json()))
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
        "reply": reply(answer),
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
