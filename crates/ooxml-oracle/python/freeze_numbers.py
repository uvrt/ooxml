"""Frozen answers of the Python host for ``check``'s numbers and scale (Phase 3 G1): the calls of
ooxml-edit's and docx-agent's tests of numbers and scales, each with the state it needs.  The
host goldens hold two such calls only: the tests' values are a made-up grid's, which the
recorder leaves out.

A pytest plugin (``-p freeze_numbers``) writes one line of ``frozen/numbers.jsonl`` per call of
``check`` that asks for ``numbers`` or a ``scale``:

* **host** cases, a check of a Word document (docx-agent's tests): as ``freeze_values.py``
  freezes a call -- the document's bytes and id state before it, the session's datasets, the
  other open documents (a made-up grid as what its resolver answered the call), the clock and
  locale -- with the session's rules for numbers (``numbers``), and the Python host's answer
  normalised as the host goldens normalise one.  The Rust host's replay runs the call.
* **service** cases, a check of a made-up format's document (ooxml-edit's tests): what the
  fact was computed from -- for ``numbers`` the texts the format gave, the values indexed (each
  source's address, as written, and rows) and what could not be read, the rules, the locale,
  the document's name and every reference the fix read (``facts``); for a ``scale`` the scale
  given, the values the format read and every reference read again -- with the fact the Python
  host gave.  The Rust replay runs the service (the made-up formats' tools are Python's own).

    FROZEN_OUT=<file> python -m pytest -p freeze_numbers <tests>
"""

from __future__ import annotations

import json
import os
import threading
from typing import Any

import freeze_values as values
import host_goldens as goldens

_LOCK = threading.Lock()
_LOCAL = threading.local()


def _asks(arguments: Any) -> bool:
    if not isinstance(arguments, dict):
        return False
    include = arguments.get("include") or ()
    include = [include] if isinstance(include, str) else include
    return "numbers" in include or arguments.get("scale") is not None


def _rules(session: Any) -> dict:
    rules = getattr(session, "numbers", None)
    if rules is None:
        return {}
    return {"ignore": goldens.plain(list(rules.ignore)), "whole_numbers": bool(rules.whole_numbers),
            "texts": rules.texts, "per_text": rules.per_text}


def _dataset(data: Any) -> dict:
    """A dataset's rows as the Rust host reads them (values as JSON, dates ISO)."""
    from docx_agent.rust.served import _value

    return {"columns": [str(c) for c in data.columns],
            "rows": [[_value(v) for v in row] for row in data.rows],
            "sources": [[list(s) for s in row] for row in data.sources] if data.sources is not None else None,
            "keys": data.keys, "types": list(data.types) if data.types is not None else None,
            "formats": list(data.formats) if getattr(data, "formats", None) is not None else None}


def _locale(locale: Any) -> dict | None:
    if locale is None:
        return None
    return {"decimal_separator": locale.decimal_separator, "thousands_separator": locale.thousands_separator,
            "date_order": locale.date_order, "language": locale.language, "list_separator": locale.list_separator}


def _write(case: dict) -> None:
    with _LOCK, open(os.environ["FROZEN_OUT"], "a", encoding="utf-8") as out:
        out.write(json.dumps(case, sort_keys=True, ensure_ascii=False) + "\n")


def _install_service() -> None:
    """The service's inputs recorded as the made-up formats' checks compute them."""
    from ooxml_edit.tools import dispatch, numbers, scales

    index_of, fact = numbers.index_of, dispatch.Call.fact
    numbers_report, scales_report = numbers.report, scales.report
    values_of = scales.values_of

    def recording_index_of(call: Any) -> Any:
        index, notes = index_of(call)
        record = getattr(_LOCAL, "record", None)
        if record is not None:
            record["sources"] = [{"address": s.address, "shown": s.shown, "dataset": _dataset(s.dataset)}
                                 for s in index.sources]
            record["notes"] = list(notes)
        return index, notes

    def recording_fact(call: Any, address: str, *args: Any, **kwargs: Any) -> Any:
        record = getattr(_LOCAL, "record", None)
        try:
            found = fact(call, address, *args, **kwargs)
        except Exception as exc:
            if record is not None:
                record.setdefault("facts", {})[address] = {"message": getattr(exc, "message", str(exc))}
            raise
        if record is not None:
            record.setdefault("facts", {})[address] = found[0]
        return found

    def recording_values_of(call: Any, scale: Any, scope: Any) -> Any:
        found = values_of(call, scale, scope)
        record = getattr(_LOCAL, "record", None)
        if record is not None:
            record["values"] = [{"target": v.target, "text": v.text, "given": v.given, "now": v.now,
                                 "exact": v.exact, "differs": v.differs, "skip": v.skip,
                                 "where": goldens.plain(dict(v.where)), "kept": goldens.plain(v.kept),
                                 "source": v.source} for v in found]
        return found

    def numbers_case(call: Any, items: Any, rules: Any, narrower: Any = ()) -> Any:
        record = {"service": "numbers", "items": [
            {"at": i.at, "text": i.text, "written": [list(w) for w in i.written],
             "scope": goldens.plain(dict(i.scope)), "where": goldens.plain(dict(i.where))} for i in items],
            "rules": {"ignore": goldens.plain(list(rules.ignore)), "whole_numbers": bool(rules.whole_numbers),
                      "texts": rules.texts, "per_text": rules.per_text},
            "narrower": list(narrower)}
        return _service(call, record, lambda: numbers_report(call, items, rules, narrower))

    def scales_case(call: Any, given: Any, scope: Any) -> Any:
        record = {"service": "scale", "given": goldens.plain(given), "scope": goldens.plain(dict(scope))}
        return _service(call, record, lambda: scales_report(call, given, scope))

    def _service(call: Any, record: dict, run: Any) -> Any:
        entry = call.entry
        if entry is not None and entry.kind == "docx":
            return run()
        record.update(test=os.environ.get("PYTEST_CURRENT_TEST", "").split(" ")[0],
                      locale=_locale(getattr(call, "locale", None)),
                      doc=entry.doc_id if entry is not None else None,
                      kind=entry.kind if entry is not None else None,
                      reading=sorted(k for k, f in call.session.formats.items()
                                     if getattr(f, "scale_values", None) is not None))
        _LOCAL.record = record
        try:
            data = run()
        except Exception as exc:  # noqa: BLE001 -- a refusal is an answer too
            _LOCAL.record = None
            record["raised"] = {"code": getattr(exc, "code", type(exc).__name__),
                                "message": getattr(exc, "message", str(exc))}
            _write(record)
            raise
        _LOCAL.record = None
        record["data"] = goldens.plain(data)
        _write(record)
        return data

    numbers.index_of = recording_index_of
    dispatch.Call.fact = recording_fact
    scales.values_of = recording_values_of
    numbers.report = numbers_case
    scales.report = scales_case


def pytest_configure(config: Any) -> None:
    from ooxml_edit.tools import Toolbox
    from ooxml_edit.tools import session as sessions

    sessions.utc_now = lambda: goldens.FIXED_CLOCK
    goldens.pin_faces()
    _install_service()
    rows = values._dataset

    def with_formats(entry: Any) -> dict:
        # (a resolver's answer with its columns' number formats: what a check of numbers reads)
        found = rows(entry)
        formats = getattr(entry.dataset, "formats", None)
        found["formats"] = list(formats) if formats is not None else None
        return found

    values._dataset = with_formats
    original = Toolbox.dispatch

    def dispatch(box: Any, session: Any, name: str, arguments: Any, *args: Any, **kwargs: Any) -> Any:
        parsed = json.loads(arguments) if isinstance(arguments, str) else arguments
        tool = str(name).rsplit(".", 1)[-1]
        documents = getattr(session, "documents", None)
        doc = parsed.get("doc") if isinstance(parsed, dict) else None
        if tool != "check" or not _asks(parsed) or documents is None or \
                getattr(documents.get(doc), "kind", None) != "docx":
            return original(box, session, name, arguments, *args, **kwargs)
        test = os.environ.get("PYTEST_CURRENT_TEST", "").split(" ")[0]
        try:
            case = values._case(session, box, str(name), dict(parsed), test)
        except Exception as exc:  # noqa: BLE001 -- a call that cannot be frozen is said so
            case = {"test": test, "tool": tool, "arguments": goldens.plain(parsed),
                    "skipped": f"not frozen: {type(exc).__name__}: {exc}"}
        case["numbers"] = _rules(session)
        # (the index read again in this call, so the grid's answers to it are frozen with it:
        # a kept index changes no answer)
        kept = getattr(session, "session", session)
        if hasattr(kept, "_number_index"):
            kept._number_index = None
        hooks = values._Hooks(session)
        others = values._others(session, doc, None)
        try:
            result = original(box, session, name, arguments, *args, **kwargs)
        finally:
            hooks.restore()
        if "skipped" not in case:
            for other in others:
                subject = other.pop("subject", None)
                if subject is not None:
                    other["answers"] = [[address, answer] for held, address, answer
                                        in hooks.answers.get(other["kind"], []) if held == subject]
            case["others"] = others
            case["merges"] = hooks.merges
            case["result"] = goldens.normalised_result(result)
            case["after"] = goldens.part_digests(session.save(doc).data) if doc in session.documents else None
        _write(case)
        return result

    Toolbox.dispatch = dispatch
