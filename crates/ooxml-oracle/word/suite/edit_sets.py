"""The suite's cases: tool calls as JSON, as a caller sends them, each run on a fresh copy of
its document to give one tool output that Word is asked about.  Three sources:

* **sets** (``suite/sets/<set>.json``): the e1-e6 edit sets, written as tool calls.  A case is
  ``{"name", "document", "tracked", "second", "blobs", "calls": [{"tool", "arguments"}]}``:
  ``document`` is a fixture's path under ``crates/ooxml-oracle/fixtures`` or ``"new"`` (a new
  blank document); ``tracked`` an author to track the edits under; ``second`` a fixture
  opened as ``d2``; ``blobs`` ``[[name, "png:<w>x<h>"]]`` made at run time.  In a call's
  arguments, ``$pN`` is the N-th body paragraph's id at the time of the call (``$p-1`` the
  last), ``$tN`` the N-th table's and ``$bN`` the N-th blob's handle; ``"doc"`` defaults to
  ``d1``.
* **trial**: the golden transcripts (``python/docx-agent/tests/goldens/transcripts``), the
  trial's tasks as the tools were called, with the text Word must show in each output.
* **gates**: the gates' frozen cases (``crates/ooxml-oracle/cases/**.jsonl``, one call each),
  sampled (``WORD_SUITE_GATES``): by default ``sample``, one case per tool and per mode (its
  ``action``) over every fixture, the first by fixture path and case id, about an hour of Word
  cold; ``full``, ``WORD_SUITE_GATES_PER_TOOL`` (default 1) per fixture and tool, tracked or
  not, some 1,600 cases and ten hours and more of Word.  A case with a second document or a
  host locale is never taken.

Must not: import pytest or docx-agent's library (only its tool host).
"""

from __future__ import annotations

import datetime as dt
import hashlib
import json
import os
import re
import struct
import zlib
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

SUITE = Path(__file__).resolve().parent
ORACLE = SUITE.parents[1]
ROOT = ORACLE.parents[1]
FIXTURES = ORACLE / "fixtures"
SETS = SUITE / "sets"
GATE_CASES = ORACLE / "cases"
TRANSCRIPTS = ROOT / "python" / "docx-agent" / "tests" / "goldens"
#: The suite's clock: every tracked edit and comment is dated by it.
CLOCK = dt.datetime(2026, 11, 17, 10, 0, 0, tzinfo=dt.timezone.utc)
#: Which of the gates' cases the suite takes (``WORD_SUITE_GATES``): ``sample`` or ``full``.
GATES = os.environ.get("WORD_SUITE_GATES", "sample")
#: The full set's cases per fixture and tool (``WORD_SUITE_GATES_PER_TOOL``; 0: none).
GATES_PER_TOOL = int(os.environ.get("WORD_SUITE_GATES_PER_TOOL", "1"))

#: Text Word must show in a trial output's final view (the trial's own checks).
TRIAL_TEXT = {
    "w1-template-report": ["Annual Water Quality Report 2025", "Executive summary", "Nitrate (mg/l)"],
    "w2-review-comments": ["initial term is 36 months", "99.9%", "Final"],
    "w3-figures-update": ["EUR 57.1 million", "Q4 was the strongest quarter"],
    "w4-restructure": ["4 Data retention", "6 Incident response"],
    "w5-selective-review": ["Review summary", "Team leads must agree"],
    "w6-merge-section": ["Working at height", "stepladders on uneven ground"],
    "batch-30-edits": ["(Reviewed: point 1.)", "Note 1:"],
}

_PLACEHOLDER = re.compile(r"\$([ptb])(-?\d+)")


@dataclass
class Case:
    """One tool output to ask Word about: its id (``<source>/<name>``) and how it is made."""

    id: str
    source: str
    spec: dict[str, Any]
    #: Text Word's PDF must show.
    shows: list[str] = field(default_factory=list)


def png(width: int, height: int, rgb: tuple[int, int, int] = (21, 96, 130)) -> bytes:
    """A plain PNG, made here: no raster is kept in the repository."""
    def chunk(kind: bytes, body: bytes) -> bytes:
        return struct.pack(">I", len(body)) + kind + body + struct.pack(">I", zlib.crc32(kind + body) & 0xFFFFFFFF)

    header = struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0)
    rows = (b"\x00" + bytes(rgb) * width) * height
    return (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", header) + chunk(b"IDAT", zlib.compress(rows, 9))
            + chunk(b"IEND", b""))


def _blob(spec: str) -> bytes:
    kind, _, size = spec.partition(":")
    if kind != "png":
        raise ValueError(f"a blob is png:<w>x<h>, not {spec!r}")
    width, height = (int(n) for n in size.split("x"))
    return png(width, height)


# -- the sources --------------------------------------------------------------------------------


def set_cases() -> list[Case]:
    out = []
    for path in sorted(SETS.glob("*.json")):
        edit_set = json.loads(path.read_text(encoding="utf-8"))
        for spec in edit_set["cases"]:
            out.append(Case(f"{edit_set['set']}/{spec['name']}", "sets", spec, spec.get("shows", [])))
    return out


def trial_cases() -> list[Case]:
    return [Case(f"trial/{task}", "trial", {"transcript": task}, text) for task, text in sorted(TRIAL_TEXT.items())]


def gate_cases() -> tuple[list[Case], str | None]:
    """The gates' frozen tool-call cases :data:`GATES` takes, and why none are when none are."""
    if GATES not in ("sample", "full"):
        raise ValueError(f"WORD_SUITE_GATES is sample or full, not {GATES!r}")
    files = sorted(GATE_CASES.glob("fixtures/**/*.jsonl"))
    if not files:
        return [], "the gates' cases are not frozen as data yet (crates/ooxml-oracle/cases)"
    entries = _gate_entries(files)
    if GATES == "full":
        return [_gate_case(fixture, entry) for fixture, entry in entries], None
    out = []
    sampled: set[tuple[str, str | None]] = set()
    for fixture, entry in sorted(entries, key=lambda item: (item[0], item[1]["id"])):
        stratum = (entry["case"]["tool"], entry["case"]["arguments"].get("action"))
        if stratum not in sampled:
            sampled.add(stratum)
            out.append(_gate_case(fixture, entry))
    return out, None


def _gate_entries(files: list[Path]) -> list[tuple[str, dict[str, Any]]]:
    """The tool-call cases of these files, with their fixture: ``full`` takes
    :data:`GATES_PER_TOOL` per fixture and tool (tracked or not), ``sample`` every one."""
    out = []
    for path in files:
        fixture = path.relative_to(GATE_CASES / "fixtures").as_posix().removesuffix(".jsonl")
        taken: dict[str, int] = {}
        for line in path.read_text(encoding="utf-8").splitlines():
            entry = json.loads(line)
            case = entry["case"]
            if "tool" not in case or case.get("reference_file") or case.get("reference_copy") or case.get("locale"):
                continue                     # an op case, or one with a second document or a host locale
            if GATES == "full":
                key = case["tool"] + ("+tracked" if case.get("tracked") else "")
                if taken.get(key, 0) >= GATES_PER_TOOL:
                    continue
                taken[key] = taken.get(key, 0) + 1
            out.append((fixture, entry))
    return out


def _gate_case(fixture: str, entry: dict[str, Any]) -> Case:
    case = entry["case"]
    spec = {"document": fixture, "tracked": (case.get("tracked") or {}).get("author"),
            "calls": [{"tool": case["tool"], "arguments": case["arguments"]}],
            "may_refuse": True}
    return Case(f"gates/{fixture}/{entry['id']}", "gates", spec)


def all_cases() -> list[Case]:
    return set_cases() + trial_cases() + gate_cases()[0]


# -- running a case -----------------------------------------------------------------------------


def _resolve(value: Any, ids: dict[str, list[str]]) -> Any:
    if isinstance(value, str):
        def one(match: re.Match) -> str:
            listed = ids[match.group(1)]
            n = int(match.group(2))
            return listed[n - 1 if n > 0 else n]
        return _PLACEHOLDER.sub(one, value)
    if isinstance(value, list):
        return [_resolve(v, ids) for v in value]
    if isinstance(value, dict):
        return {k: _resolve(v, ids) for k, v in value.items()}
    return value


def _document(name: str) -> bytes:
    if name == "new":
        import ooxml_core

        return bytes(ooxml_core.new_document(page=("A4", False), language="en-US", separators=(".", ","),
                                             properties=(None, None, "2026-10-04T00:00:00Z")))
    return (FIXTURES / name).read_bytes()


class Refused(RuntimeError):
    """A call the tools refused: the case gives no output to ask Word about."""


def run(box: Any, case: Case) -> bytes:
    """The case's tool output: its document after its calls."""
    if case.source == "trial":
        return _run_transcript(case.spec["transcript"])
    import word_docs

    spec = case.spec
    session = box.session(clock=lambda: CLOCK)
    data = _document(spec["document"])
    session.open(data, Path(spec["document"]).name if spec["document"] != "new" else "new.docx")
    if spec.get("second"):
        session.open(_document(spec["second"]), Path(spec["second"]).name)
    handles = [session.add_blob(_blob(blob), name) for name, blob in spec.get("blobs") or []]
    if spec.get("tracked"):
        _call(box, session, "word_set_tracking", {"doc": "d1", "on": True, "author": spec["tracked"]})
    for call in spec["calls"]:
        arguments = dict(call["arguments"])
        if any(_PLACEHOLDER.search(json.dumps(v)) for v in arguments.values()):
            listed = [b for b, _ in word_docs.blocks(box, session, "d1", view="markup")]
            ids = {"p": [b for b in listed if b.startswith(("p:", "p@"))],
                   "t": [b for b in listed if b.startswith("t")], "b": handles}
            arguments = _resolve(arguments, ids)
        arguments.setdefault("doc", "d1")
        _call(box, session, call["tool"], arguments, may_refuse=spec.get("may_refuse", False))
    return session.entry("d1").snapshot()


def _call(box: Any, session: Any, tool: str, arguments: dict, *, may_refuse: bool = False) -> Any:
    result = box.dispatch(session, tool, arguments)
    if not result.ok:
        error = result.to_json().get("error")
        if may_refuse:
            raise Refused(f"{tool}: {error}")
        raise RuntimeError(f"{tool} {arguments}: {error}")
    return result


def _run_transcript(task: str) -> bytes:
    import goldens_replay

    transcript = json.loads((TRANSCRIPTS / "transcripts" / f"{task}.json").read_text(encoding="utf-8"))
    if "fixture" in transcript:
        inputs = (FIXTURES / transcript["fixture"]).parent
    else:
        inputs = TRANSCRIPTS / "trial" / "_inputs" / transcript["task"] / "input"
    _, outputs = goldens_replay.replay(transcript, inputs)
    return outputs[-1].data


def digest(data: bytes) -> str:
    """A tool output's digest, recorded with Word's answers: the answers are about these bytes."""
    import io
    import zipfile

    with zipfile.ZipFile(io.BytesIO(data)) as archive:
        h = hashlib.sha256()
        for name in sorted(archive.namelist()):
            h.update(name.encode() + b"\0" + archive.read(name) + b"\0")
    return h.hexdigest()[:16]
