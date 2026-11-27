#!/usr/bin/env python3
"""The stamp-on-open change (user decision 2026-11-26, phase3-stamp-on-open-design.md): a
session names every block of a document by a durable id from the first read, so the frozen
gate cases and the host goldens that named blocks by place re-targeted to those ids.

    python crates/ooxml-oracle/python/stamp_on_open.py maps --rust target/release/word-ids --out maps.jsonl
    python crates/ooxml-oracle/python/stamp_on_open.py cases --maps maps.jsonl --renamed case-ids.jsonl
    python crates/ooxml-oracle/python/stamp_on_open.py host --maps maps.jsonl

* ``maps``: each document of the trees, ``cases/inputs`` and ``host/inputs`` read by
  ``word-ids --renamed``: the ids an unstamped reading gave (``p@body/3``, ``t@body/0``, a row
  ``t@body/0/r1`` or ``t:X/r1``, a repeat ``p:X#1``) that a session now names otherwise.
* ``cases``: every frozen case's strings with its document's old ids written as the new ones;
  a changed case takes its new id (``goldens.case_id``), the old id's golden and pending answer
  go (the case is no more: its new id is blessed from a gate run), and the old -> new case id
  map is written.
* ``host``: every docx stream's events re-targeted the same way -- a call's arguments and its
  recorded result through the map of the document the call names (``doc``), the documents
  each case opened known from its ``open`` events -- and the frozen answers of ``frozen/``
  through their document's.  Re-runnable: a second run changes nothing.

Stdlib only.
"""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import goldens  # noqa: E402

ORACLE = goldens.ORACLE
HOST = ORACLE / "host"
TREES = {"fixtures": ORACLE / "fixtures", "corpus": ORACLE / "corpus"}

#: A block id, as a reading gives it: by place (with its story outside the body), a row of a
#: table by place, a paraId (a repeat's with its number); only those a map names are written
#: again (an unstamped reading's by place, repeats, out-of-range paraIds).
TOKEN = re.compile(
    r"(?<![A-Za-z0-9_@/])"
    r"((?:[A-Za-z][A-Za-z0-9]*/)?(?:p|t|s)@[A-Za-z][A-Za-z0-9]*/\d+(?:/r\d+)?"
    r"|(?:[A-Za-z][A-Za-z0-9]*/)?(?:p|t|tr|s):[0-9A-Fa-f]{1,8}(?:#\d+|/r\d+)?)"
    r"(?![0-9A-Fa-f])")


def renamed_text(text: str, ids: dict[str, str]) -> str:
    """``text`` with each old id of ``ids`` as it is named now; a row named by place in a
    table renamed keeps its place (``t@body/0/r1`` -> ``t:X/r1``)."""
    def one(m: re.Match) -> str:
        token = m.group(1)
        if token in ids:
            return ids[token]
        head, sep, row = token.rpartition("/r")
        if sep and head in ids and row.isdigit():
            return ids[head] + "/r" + row
        return token
    return TOKEN.sub(one, text)


def renamed(value, ids: dict[str, str]):
    """``value`` (JSON) with every string, keys too, re-targeted by ``ids``."""
    if not ids:
        return value
    if isinstance(value, str):
        return renamed_text(value, ids)
    if isinstance(value, list):
        return [renamed(v, ids) for v in value]
    if isinstance(value, dict):
        return {renamed_text(k, ids): renamed(v, ids) for k, v in value.items()}
    return value


def resolved(path: str) -> str:
    """A portable path as this checkout's (``$ROOT``, ``$INPUT``, ``$HOST``)."""
    if path.startswith("$HOST/"):
        return str((HOST / path[len("$HOST/"):]).resolve())
    return str(Path(goldens.local(path)).resolve())


def portable(path: str) -> str:
    """``path`` as the maps name it: ``$HOST/...`` below host/, else goldens' portable form."""
    host = str(HOST.resolve()) + "/"
    real = str(Path(path).resolve())
    return "$HOST/" + real[len(host):] if real.startswith(host) else goldens.portable(real)


def read_maps(path: Path) -> dict[str, dict[str, str]]:
    maps = {}
    for line in path.read_text().splitlines():
        row = json.loads(line)
        if "error" in (row.get("renamed") or {}):
            raise SystemExit(f"{row['file']}: {row['renamed']['error']}")
        maps[resolved(row["file"])] = row.get("renamed") or {}
    return maps


def cmd_maps(args) -> int:
    files = sorted({str(p.resolve()) for folder in [*TREES.values(), goldens.INPUTS, HOST / "inputs"]
                    for p in folder.rglob("*.docx")})
    out = []
    for at in range(0, len(files), 200):
        done = subprocess.run([args.rust, "--renamed", *files[at:at + 200]], capture_output=True, text=True,
                              check=True)
        # each file as a portable path: the maps hold in any checkout
        out += [goldens.canonical({**row, "file": portable(row["file"])})
                for row in map(json.loads, done.stdout.splitlines())]
    Path(args.out).write_text("".join(line + "\n" for line in out))
    print(f"{len(out)} documents mapped")
    return 0


def cmd_cases(args) -> int:
    maps = read_maps(Path(args.maps))
    pairs, total = [], 0
    for tree, folder in TREES.items():
        for document in goldens.documents(tree):
            cases_file = goldens.document_file(goldens.CASES, tree, document)
            lines, changed, gone = [], False, set()
            for line in goldens.read_lines(cases_file):
                total += 1
                case = line["case"]
                source = resolved(case["input"]) if case.get("input") else str((folder / document).resolve())
                new_case = renamed(case, maps.get(source, {}))
                if new_case == case:
                    lines.append(line)
                    continue
                new_id = goldens.case_id(line["mode"], new_case)
                pairs.append({"tree": tree, "document": document, "old": line["id"], "new": new_id})
                lines.append({"id": new_id, "mode": line["mode"], "case": new_case})
                gone.add(line["id"])
                changed = True
            if not changed:
                continue
            goldens.write_lines(cases_file, lines)
            for base, suffix in ((goldens.GOLDENS, ".jsonl"), (goldens.PENDING, ".jsonl.gz")):
                path = goldens.document_file(base, tree, document, suffix)
                kept = [g for g in goldens.read_lines(path) if g["id"] not in gone]
                goldens.write_lines(path, kept)
    previous = []
    if Path(args.renamed).is_file():
        previous = [json.loads(x) for x in Path(args.renamed).read_text().splitlines()]
    Path(args.renamed).write_text("".join(json.dumps(p, sort_keys=True) + "\n" for p in [*previous, *pairs]))
    print(f"{total} cases, {len(pairs)} re-targeted")
    return 0


def host_event(event: dict, documents: dict[str, dict[str, str]]) -> dict:
    """``event`` with its calls' arguments and results re-targeted by their documents' maps."""
    kind = event.get("event")
    if kind == "open" and event.get("returned") and event.get("input"):
        documents[event["returned"]] = documents.get("#maps", {}).get(resolved(event["input"]["path"]), {})
        return event
    def of(arguments) -> dict[str, str]:
        doc = arguments.get("doc") if isinstance(arguments, dict) else None
        return documents.get(doc, {}) if isinstance(doc, str) else {}
    out = dict(event)
    if kind == "call":
        ids = of(event.get("arguments"))
        for key in ("arguments", "result"):
            if key in out:
                out[key] = renamed(out[key], ids)
    elif kind == "calls":
        calls = event.get("calls") or []
        maps = [of(c[1] if isinstance(c, list) and len(c) > 1 else {}) for c in calls]
        out["calls"] = [renamed(c, ids) for c, ids in zip(calls, maps)]
        if isinstance(event.get("results"), list):
            out["results"] = [renamed(r, ids) for r, ids in zip(event["results"], maps)]
    return out


def cmd_host(args) -> int:
    maps = read_maps(Path(args.maps))
    rewritten = events = 0
    for stream in sorted((HOST / "docx").rglob("*.jsonl")):
        lines = stream.read_text().splitlines()
        cases: dict[str, dict] = {}
        out = []
        for line in lines:
            event = json.loads(line)
            documents = cases.setdefault(event.get("case"), {"#maps": maps})
            new = host_event(event, documents)
            events += 1
            if new != event:
                rewritten += 1
                out.append(goldens.canonical(new))
            else:
                out.append(line)
        stream.write_text("".join(x + "\n" for x in out))
    # the Python host's frozen answers (frozen/*.jsonl): each a call on one document
    for frozen in sorted((ORACLE / "frozen").glob("*.jsonl")):
        out = []
        for line in frozen.read_text().splitlines():
            if not line.strip():
                out.append(line)
                continue
            answer = json.loads(line)
            source = ((answer.get("document") or {}).get("input") or {}).get("path")
            ids = maps.get(resolved(source), {}) if source else {}
            new = {**answer, **{k: renamed(answer[k], ids) for k in ("arguments", "result") if k in answer}}
            events += 1
            if new != answer:
                rewritten += 1
                out.append(json.dumps(new, ensure_ascii=False))
            else:
                out.append(line)
        frozen.write_text("".join(x + "\n" for x in out))
    print(f"{events} events and frozen answers, {rewritten} re-targeted")
    return 0


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    sub = parser.add_subparsers(dest="command", required=True)
    m = sub.add_parser("maps")
    m.add_argument("--rust", required=True, help="the word-ids oracle binary")
    m.add_argument("--out", required=True)
    c = sub.add_parser("cases")
    c.add_argument("--maps", required=True)
    c.add_argument("--renamed", required=True, help="the old -> new case id map (appended to)")
    h = sub.add_parser("host")
    h.add_argument("--maps", required=True)
    args = parser.parse_args(argv)
    return {"maps": cmd_maps, "cases": cmd_cases, "host": cmd_host}[args.command](args)


if __name__ == "__main__":
    sys.exit(main())
