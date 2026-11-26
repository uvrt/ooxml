"""Frozen answers of the Python host for the values a Word call reads from its sources (Phase 3
D3b): ``word_insert_markdown``'s ``from`` naming one text field (round 45, R65),
``word_edit_table``'s ``from`` naming rows, and each cell's one-value ``from`` with its
refusals (docx-agent's ``rust/served`` adapters: ``field_from``, ``sources_json``,
``_cell_froms``).  The host goldens do not hold them: their datasets are made by a test's own
tool, which the recorder leaves out.

A pytest plugin (``-p freeze_values``) run over docx-agent's tests writes, for each such call, one
line of ``frozen/values.jsonl``: the document as it was before the call (its bytes kept in
``host/inputs``, its id state), the session's datasets then (each with its values as the
adapters hand them over: ``served._value``), the session's clock and locale, the call, and the
Python host's answer normalised as the host goldens normalise one, with the document's parts'
digests after it.  The Rust host's replay of them is ``tests/integration/frozen_values.rs``.

The test's own formats beside Word's (a made-up grid whose ``t:`` names its tables) are frozen
too, as what the call asked of them: each address its resolver was given and what it answered
(the rows, or its refusal), and each run of addresses merged. Every other open document is
frozen as its bytes (a Word document), or as its handle, name, kind, ``value_sources`` and its
format's answers.

A call this cannot freeze is written as ``{"skipped": why}``: a dataset tied to a document.

    FROZEN_OUT=<file> python -m pytest -p freeze_values python/docx-agent/tests/test_x.py
"""

from __future__ import annotations

import json
import os
import threading
from pathlib import Path
from typing import Any

import host_goldens as goldens

#: The tools and arguments frozen.
_TOOLS = ("word_insert_markdown", "word_edit_table")
_LOCK = threading.Lock()
#: Each session's documents as first opened, and the latest tracking call on each, by the
#: session's table of documents (shared by its views).
_OPENED: dict[int, dict] = {}
_TRACKING: dict[int, dict] = {}


def _reads_values(name: str, arguments: Any) -> bool:
    tool = name.rsplit(".", 1)[-1]
    if tool not in _TOOLS or not isinstance(arguments, dict):
        return False
    if arguments.get("from"):
        return True
    cells = arguments.get("cells")
    return tool == "word_edit_table" and isinstance(cells, list) and any(
        isinstance(c, dict) and c.get("from") for c in cells)


def _dataset(entry: Any) -> dict:
    from docx_agent.rust.served import _value

    data = entry.dataset
    return {"id": entry.id, "columns": [str(c) for c in data.columns],
            "rows": [[_value(v) for v in row] for row in data.rows],
            "sources": [[list(s) for s in row] for row in data.sources] if data.sources is not None else None,
            "keys": data.keys, "types": list(data.types) if data.types is not None else None,
            "markdown": [[r, c, md] for (r, c), md in sorted((data.markdown or {}).items())],
            "cache": dict(data.cache) if data.cache else None,
            "doc": entry.doc, "stale": bool(entry.stale)}


def _skip(session: Any) -> str | None:
    for entry in session.dataset_entries():
        if entry.doc is not None:
            return f"d:{entry.id} is tied to document {entry.doc}"
    return None


def _rows(found: Any) -> dict:
    """A resolver's answer: the rows (as :func:`_dataset` writes a dataset), or its refusal."""
    if isinstance(found, BaseException):
        return {"refused": {"code": getattr(found, "code", type(found).__name__),
                            "message": getattr(found, "message", str(found)),
                            "options": goldens.plain(list(getattr(found, "valid_options", None) or []))}}

    class _Entry:
        id = ""
        dataset = found
        doc = None
        stale = False

    rows = _dataset(_Entry())
    del rows["id"], rows["doc"], rows["stale"]
    return {"rows": rows}


class _Hooks:
    """The test's own formats' hooks a call uses, wrapped to freeze what they answer."""

    def __init__(self, session: Any) -> None:
        self.answers: dict[str, list] = {}
        self.merges: list = []
        self.undo: list = []
        for fmt in {id(f): f for f in session.formats.values()}.values():
            if fmt.kind != "docx":
                self._wrap(fmt)

    def _wrap(self, fmt: Any) -> None:
        kind = fmt.kind
        answers = self.answers.setdefault(kind, [])
        for prefix, resolver in list((fmt.datasets or {}).items()):
            def wrapped(document, address, resolver=resolver):
                try:
                    found = resolver(document, address)
                except Exception as exc:
                    answers.append([id(document), address, _rows(exc)])
                    raise
                answers.append([id(document), address, _rows(found)])
                return found
            fmt.datasets[prefix] = wrapped
            self.undo.append(lambda fmt=fmt, prefix=prefix, resolver=resolver:
                             fmt.datasets.__setitem__(prefix, resolver))
        merge = fmt.merge_sources
        if merge is not None:
            def merged(addresses, merge=merge):
                found = list(merge(addresses))
                self.merges.append([kind, list(addresses), found])
                return found
            object.__setattr__(fmt, "merge_sources", merged)
            self.undo.append(lambda fmt=fmt, merge=merge: object.__setattr__(fmt, "merge_sources", merge))

    def restore(self) -> None:
        for undo in reversed(self.undo):
            undo()


def _others(session: Any, main: Any, out: Path) -> list:
    """The other open documents: a Word document's bytes and state; another format's
    handle, name and kind, and its ``value_sources``."""
    found = []
    for handle, entry in session.documents.items():
        if handle == main:
            continue
        if entry.kind == "docx":
            saved = session.save(handle)
            found.append({"doc": handle, "kind": "docx", "name": saved.name,
                          "input": goldens.keep_input(saved.data, saved.name, goldens.HOST / "inputs"),
                          "state": goldens.plain(saved.state)})
            continue
        fmt = session.formats[entry.kind]
        values = fmt.value_sources(entry.subject) if fmt.value_sources is not None else None
        found.append({"doc": handle, "kind": entry.kind, "name": entry.name, "subject": id(entry.subject),
                      "prefixes": sorted(fmt.datasets or {}),
                      "value_sources": None if values is None else [str(v) for v in values]})
    return found


def _case(session: Any, box: Any, name: str, arguments: dict, test: str) -> dict:
    case: dict = {"test": test, "tool": name.rsplit(".", 1)[-1], "arguments": goldens.plain(arguments)}
    why = _skip(session)
    if why is not None:
        case["skipped"] = why
        return case
    doc = arguments.get("doc")
    saved = session.save(doc)
    out = Path(os.environ["FROZEN_OUT"])
    case["document"] = {"name": saved.name,
                        "input": goldens.keep_input(saved.data, saved.name, goldens.HOST / "inputs"),
                        "state": goldens.plain(saved.state), "version": saved.version}
    case["datasets"] = [_dataset(e) for e in session.dataset_entries()]
    opened = _OPENED.get(id(session.documents), {}).get(doc)
    if opened is not None:
        case["opened"] = {"name": opened["name"], "state": opened["state"],
                          "input": goldens.keep_input(opened["data"], opened["name"], goldens.HOST / "inputs")}
    tracking = _TRACKING.get(id(session.documents), {}).get(doc)
    if tracking is not None:
        case["tracking"] = tracking
    case["formats"] = sorted(({"kind": f.kind, "prefixes": sorted(f.datasets or {})}
                              for f in {id(f): f for f in session.formats.values()}.values()
                              if f.kind != "docx"), key=lambda f: f["kind"])
    now = session.now()
    case["clock"] = now.isoformat() if hasattr(now, "isoformat") else None
    locale = getattr(session, "locale", None)
    case["locale"] = None if locale is None else {
        "decimal_separator": locale.decimal_separator, "thousands_separator": locale.thousands_separator,
        "date_order": locale.date_order, "language": locale.language, "list_separator": locale.list_separator}
    return case


def pytest_configure(config: Any) -> None:
    from ooxml_edit.tools import Toolbox
    from ooxml_edit.tools import session as sessions

    # every date a call writes the same, and every layout of the repository's faces, as the
    # host goldens record them
    sessions.utc_now = lambda: goldens.FIXED_CLOCK
    goldens.pin_faces()
    original = Toolbox.dispatch
    opening = sessions.Session.open

    def open_(session, data, name, *args, **kwargs):
        # what each document was opened as: the session's first version of it (what a
        # call compares with "the original")
        doc = opening(session, data, name, *args, **kwargs)
        _OPENED.setdefault(id(session.documents), {})[doc] = {
            "data": bytes(data), "name": name, "state": goldens.plain(kwargs.get("state"))}
        return doc

    sessions.Session.open = open_

    def dispatch(box, session, name, arguments, *args, **kwargs):
        parsed = json.loads(arguments) if isinstance(arguments, str) else arguments
        if str(name).rsplit(".", 1)[-1] == "word_set_tracking" and isinstance(parsed, dict):
            result = original(box, session, name, arguments, *args, **kwargs)
            if result.ok and getattr(session, "documents", None) is not None:
                _TRACKING.setdefault(id(session.documents), {})[parsed.get("doc")] = goldens.plain(parsed)
            return result
        if not _reads_values(str(name), parsed) or getattr(session, "documents", None) is None:
            return original(box, session, name, arguments, *args, **kwargs)
        test = os.environ.get("PYTEST_CURRENT_TEST", "").split(" ")[0]
        try:
            case = _case(session, box, str(name), dict(parsed), test)
        except Exception as exc:  # noqa: BLE001 -- a call that cannot be frozen is said so
            case = {"test": test, "tool": str(name), "arguments": goldens.plain(parsed),
                    "skipped": f"not frozen: {type(exc).__name__}: {exc}"}
        hooks = _Hooks(session)
        others = _others(session, parsed.get("doc"), Path(os.environ["FROZEN_OUT"]))
        try:
            result = original(box, session, name, arguments, *args, **kwargs)
        finally:
            hooks.restore()
        if "skipped" not in case:
            for other in others:
                subject = other.pop("subject", None)
                if subject is not None:
                    other["answers"] = [[address, answer] for doc, address, answer
                                        in hooks.answers.get(other["kind"], []) if doc == subject]
            case["others"] = others
            case["merges"] = hooks.merges
            case["result"] = goldens.normalised_result(result)
            doc = parsed.get("doc")
            case["after"] = goldens.part_digests(session.save(doc).data) if doc in session.documents else None
        with _LOCK, open(os.environ["FROZEN_OUT"], "a", encoding="utf-8") as out:
            out.write(json.dumps(case, sort_keys=True, ensure_ascii=False) + "\n")
        return result

    Toolbox.dispatch = dispatch
