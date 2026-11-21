#!/usr/bin/env python3
"""The Word reads' differential harness (test support; phase P1, lane P1a).

Runs docx-agent and the Rust core on the same documents and compares, field by field:

* ``ids``: every story part's paragraph, table and row ids; the ``docx-ids.1.`` record of
  the document as opened (decoded: digest, seeds, deleted ids, aliases...); the next three
  values of each part's sequence; and the same after a reopen -- the document edited by
  docx-agent (a paragraph deleted, one added), saved with its id state, and opened again
  with it on both sides (``word-ids``).
* ``describe`` and ``read``: the tools' replies (``word-reads``, the Rust session's
  ``describe`` and ``word_read``) against docx-agent's toolbox, ``word_read`` in every view
  and for ``stories=all``.
* ``ranges``, ``inspect``, ``find``, ``batch`` and ``check``: the addressed reads, the batch,
  and ``check``'s facts read from the document alone (lane P5a; ``word_reads_tools.py``).
* ``facts``: ``check``'s layout facts read from the document alone, as library functions
  (lane P5a2; ``word-reads``' fact cases, ``word_facts_cases.py``).

Every difference is printed with its JSON path; the exit status is 1 when there is any.

    python crates/ooxml-oracle/python/word_reads_diff.py ids --rust target/release/word-ids a.docx --tree DIR
    python crates/ooxml-oracle/python/word_reads_diff.py read --rust target/release/word-reads --tree DIR

Python's behaviour is not changed: this only calls docx-agent's public and documented
internal reading functions.
"""

from __future__ import annotations

import argparse
import base64
import functools
import json
import subprocess
import sys
import tempfile
import zlib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
for package in ("docx-agent", "docx2svg", "ooxml-edit", "ooxml-common"):
    source = ROOT / "python" / package / "src"
    if str(source) not in sys.path:
        sys.path.insert(0, str(source))

# The reference is docx-agent in Python: the Rust core kept out of this process and its workers
# before docx-agent is imported (reference_guard.py).
from reference_guard import check as _core_check  # noqa: E402
from reference_guard import guard as _core_guard  # noqa: E402

_core_guard()

from docx_agent import Document  # noqa: E402
from docx_agent.edit import ids as _ids  # noqa: E402


def decode_state(state: str) -> dict:
    body = state[len(_ids.STATE_PREFIX):]
    return json.loads(zlib.decompress(base64.urlsafe_b64decode(body + "=" * (-len(body) % 4))))


def python_ids(document) -> dict:
    parts = []
    for part in document._parts():
        index = document._index(part)
        parts.append({"part": part, "story": index.story,
                      "paragraphs": [[e.id, e.state] for e in index.paragraphs],
                      "tables": [[t.id, [r for _, r in t.rows]] for t in index.tables]})
    record = decode_state(document.id_state())
    pool = document._id_pool()
    generated = {part: _ids.generate(part, pool, 3) for part in document._parts()}
    return {"parts": parts, "record": record, "generated": generated}


def edited(data: bytes) -> tuple[bytes, str, list[str]]:
    """The document with a paragraph added at the end (which stamps every id), then its
    second paragraph deleted: its bytes and id state; what could not be done, in words."""
    document = Document.open(data)
    notes = []
    try:
        document.insert_markdown("A paragraph added.", at="end")
    except Exception as error:  # noqa: BLE001 -- a test instrument: say what failed
        notes.append(f"insert: {error}")
    document.id_state()                  # the stamped state read, so the deletion is recorded
    paragraphs = document.paragraphs()
    if len(paragraphs) > 1:
        try:
            paragraphs[1].delete()
        except Exception as error:  # noqa: BLE001
            notes.append(f"delete: {error}")
    return document.to_bytes(), document.id_state(), notes


def differences(a, b, path: str = "", out: list | None = None, limit: int = 200) -> list:
    out = [] if out is None else out
    if len(out) >= limit:
        return out
    if isinstance(a, dict) and isinstance(b, dict):
        for key in list(a) + [k for k in b if k not in a]:
            if key not in a or key not in b:
                out.append(f"{path}/{key}: only in {'python' if key in a else 'rust'}")
            else:
                differences(a[key], b[key], f"{path}/{key}", out, limit)
        if list(a) != list(b) and set(a) == set(b):
            out.append(f"{path}: keys in another order")
    elif isinstance(a, list) and isinstance(b, list):
        if len(a) != len(b):
            out.append(f"{path}: {len(a)} items in python, {len(b)} in rust")
        for k, (x, y) in enumerate(zip(a, b)):
            differences(x, y, f"{path}[{k}]", out, limit)
    elif a != b or type(a) is not type(b):
        out.append(f"{path}: python {a!r} rust {b!r}")
    return out


def run_rust(binary: str, manifest: list[dict]) -> dict[str, dict]:
    with tempfile.NamedTemporaryFile("w", suffix=".json", delete=False) as handle:
        json.dump(manifest, handle)
    done = subprocess.run([binary, handle.name], capture_output=True, text=True, check=True)
    Path(handle.name).unlink()
    return {(r["file"], r.get("case")): r for r in map(json.loads, done.stdout.splitlines())}


def files_of(args) -> list[Path]:
    files = [Path(f) for f in args.files]
    for tree in args.tree or ():
        files += sorted(Path(tree).rglob("*.docx"))
    return [f for f in files if not f.name.startswith("~$")]


def check_ids(args) -> int:
    files = files_of(args)
    scratch = Path(tempfile.mkdtemp(prefix="word-ids-"))
    expected: dict = {}
    manifest = []
    for n, path in enumerate(files):
        data = path.read_bytes()
        try:
            expected[(str(path), None)] = python_ids(Document.open(data))
            manifest.append({"file": str(path), "state": None})
            new, state, notes = edited(data)
            again = scratch / f"{n}-{path.name}"
            again.write_bytes(new)
            expected[(str(again), None)] = python_ids(Document.open(new, ids=state))
            expected[(str(again), None)]["notes"] = notes
            manifest.append({"file": str(again), "state": state})
        except Exception as error:  # noqa: BLE001
            print(f"{path}: python failed: {error}")
    rust = run_rust(args.rust, manifest)
    return report(expected, {k: v.get("ids") for k, v in rust.items()}, ignore=("notes",), results=args.results)


def refusal(result) -> dict | str | None:
    """What a side's result says went wrong, if anything: a tool reply's error (code,
    message, field) or a refused edit's message; kept in the results so a difference that is
    one side's error says which error."""
    if not isinstance(result, dict):
        return None
    reply = result.get("reply")
    if isinstance(reply, dict) and reply.get("error"):
        return reply["error"]
    return result.get("refused") or result.get("error") or None


def report(expected: dict, rust: dict, ignore: tuple = (), results: str | None = None,
           inputs: dict | None = None, counts: dict | None = None) -> int:
    """Prints each case with differences and the totals; exits 1 when there is any.
    ``results``: also a JSON-lines file of them for tools/gates.py -- one line per case with
    differences (its file, case key, input from ``inputs``, differences and each side's error,
    :func:`refusal`), then the totals
    with ``counts`` (e.g. the cases left to the host)."""
    _core_check()      # a reference that loaded the Rust core compared Rust with Rust
    failed = 0
    lines = []
    for key, want in expected.items():
        got = rust.get(key)
        want = {k: v for k, v in want.items() if k not in ignore}
        found = differences(want, got) if got is not None else ["no rust result"]
        name = key[0] if key[1] is None else f"{key[0]} [{key[1]}]"
        if found:
            failed += 1
            print(f"{name}: {len(found)} differences")
            for line in found[:20]:
                print("   ", line)
            errors = {side: refusal(r) for side, r in (("python", want), ("rust", got)) if refusal(r)}
            for side, error in errors.items():
                print(f"    {side} error: {json.dumps(error)[:400]}")
            lines.append({"file": key[0], "case": key[1], "input": (inputs or {}).get(key), "differences": found,
                          **({"errors": errors} if errors else {})})
    print(f"{len(expected)} cases, {failed} with differences")
    if results:
        totals = {"cases": len(expected), "failed": failed, **(counts or {})}
        Path(results).write_text("".join(json.dumps(x) + "\n" for x in [*lines, {"totals": totals}]))
    return 1 if failed else 0


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("what", choices=["ids", "describe", "read", "ranges", "inspect", "find", "batch", "check",
                                           "facts"])
    parser.add_argument("--rust", required=True, help="the word-ids or word-reads binary")
    parser.add_argument("--tree", action="append", help="every .docx below this folder")
    parser.add_argument("--results", help="also write the cases with differences here (JSON lines)")
    parser.add_argument("files", nargs="*")
    args = parser.parse_args(argv)
    if args.what == "ids":
        return check_ids(args)
    if args.what == "facts":
        from word_facts_cases import check_facts

        return check_facts(args, run_rust, functools.partial(report, results=args.results), files_of)
    from word_reads_tools import check_tools

    return check_tools(args, run_rust, functools.partial(report, results=args.results), files_of)


if __name__ == "__main__":
    sys.exit(main())
