"""Running a Rust oracle binary on a manifest and reading what its lines say beyond their
results (stdlib only): the Word harnesses' Rust side (word_reads_diff.py, word_edits_diff.py)
and the goldens' gate (golden_gate.py) share it."""

from __future__ import annotations

import collections
import json
import subprocess
import tempfile
from pathlib import Path

#: What the Rust side's lines said beyond their results, over this process's run (one harness
#: run per process): the cases that panicked (``{"panic": ...}``: the oracle's own catch, or
#: the session's ``internal`` reply), and the cases refused as not supported yet, by form
#: (:func:`not_supported_form`).  The harnesses add both to their totals: tools/gates.py fails
#: on any panic and on a form its allow-list does not name.
RUST_TALLY: dict = {"panics": 0, "not_supported_forms": collections.Counter()}


def not_supported_form(value) -> str | None:
    """The form a Rust result was refused as not supported yet (``details.not_supported`` of
    its error, or an edit oracle's ``not_supported``), if it was."""
    if not isinstance(value, dict):
        return None
    if value.get("not_supported"):
        return str(value["not_supported"])
    for error in (value.get("error"), (value.get("reply") or {}).get("error")):
        if isinstance(error, dict) and isinstance(error.get("details"), dict) \
                and error["details"].get("not_supported"):
            return str(error["details"]["not_supported"])
    return None


def refused(line: dict | None) -> str | None:
    """The form a Rust line's answer was refused in as not supported yet, if it was."""
    for key, value in (line or {}).items():
        form = not_supported_form(value) if key not in ("file", "case") else None
        if form:
            return form
    return None


def tally(line: dict) -> None:
    for key, value in line.items():
        if key in ("file", "case") or not isinstance(value, dict):
            continue
        if "panic" in value:
            RUST_TALLY["panics"] += 1
            print(f"{line.get('file')} [{line.get('case')}]: rust panicked: {str(value['panic'])[:300]}")
        form = not_supported_form(value)
        if form:
            RUST_TALLY["not_supported_forms"][form] += 1


def run_lines(binary: str, manifest: list[dict]) -> dict[tuple, dict]:
    """The binary's answer lines to ``manifest`` by (file, case), tallied (:data:`RUST_TALLY`)."""
    with tempfile.NamedTemporaryFile("w", suffix=".json", delete=False) as handle:
        json.dump(manifest, handle)
    try:
        done = subprocess.run([binary, handle.name], capture_output=True, text=True, check=True)
    finally:
        Path(handle.name).unlink()
    lines = [json.loads(x) for x in done.stdout.splitlines()]
    for line in lines:
        tally(line)
    return {(r["file"], r.get("case")): r for r in lines}
