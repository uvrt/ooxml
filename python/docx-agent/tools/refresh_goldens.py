"""Re-record the golden transcripts' expectations (``tests/goldens/transcripts``), safely.

    python tools/refresh_goldens.py                 # every transcript; writes what changed
    python tools/refresh_goldens.py w2 w4           # those whose name starts so
    python tools/refresh_goldens.py --dry-run       # say what would change, write nothing
    python tools/refresh_goldens.py --new-keys coverage.status,coverage.approximations
    python tools/refresh_goldens.py --rerecord w2   # record what the session now returns, saying each change

Each transcript is replayed (``tests/goldens_replay.py``) and every call's expectation is
written as it now comes out: ``data_sha``, ``data_keys`` and ``data_sha_core`` with the rest.
It runs only where Office's faces are installed (the transcripts' layout facts are those of a
Mac with Office), and it refuses -- writing nothing -- unless, for every call,

* ``ok``, ``summary`` and every other recorded field (ids, warnings, new problems, the
  error) are what was recorded;
* ``data_sha`` is the one recorded, or -- with ``--new-keys`` naming keys a change added to
  the results (``size``, or nested one level, ``coverage.status``) -- the data less those
  keys hashes to it;
and every saved output's parts (names and uncompressed bytes: ``goldens_replay.package_digest``)
are byte for byte the ones recorded.  So a refresh records what the code
now returns *about* the same results; a change that alters a result or a saved document is a
different change, to be reviewed as one, and is not something this tool waves through.

``--rerecord`` is that change (P4e: the transcripts recorded on docx-agent's Python, re-recorded
on the Rust session): every call's expectation and the output's digest are written as they
now come out, and every field that changed is listed, call by call, for review.  A transcript
whose calls no longer all succeed is not written.
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "tests"))

import goldens_replay  # noqa: E402

TRANSCRIPTS = ROOT / "tests" / "goldens" / "transcripts"
#: The fields a refresh may write; everything else must replay as recorded.
DATA_FIELDS = ("data_sha", "data_keys", "data_sha_core")


def without(data, keys: list[str]):
    """``data`` less ``keys`` (``key`` or ``key.sub``, ``sub`` of a dict or of each dict in a
    list)."""
    if not isinstance(data, dict):
        return data
    out = {}
    for key, value in data.items():
        if key in keys:
            continue
        if isinstance(value, dict):
            value = {sub: v for sub, v in value.items() if f"{key}.{sub}" not in keys}
        elif isinstance(value, list):
            # a list's items, each a dict: ``blocks.new_text`` is each block's new key
            value = [{sub: v for sub, v in item.items() if f"{key}.{sub}" not in keys}
                     if isinstance(item, dict) else item for item in value]
        out[key] = value
    return out


def refresh(path: Path, new_keys: list[str]) -> tuple[dict, list[str], int]:
    """The transcript with its expectations re-recorded, the problems that forbid writing
    it, and how many expectations changed."""
    from test_tools_goldens import inputs_of

    transcript = json.loads(path.read_text(encoding="utf-8"))
    data: dict[int, object] = {}
    results, outputs = goldens_replay.replay(
        transcript, inputs_of(transcript),
        on_result=lambda index, result: data.__setitem__(index, result.to_json().get("data")))
    problems, changed = [], 0
    for index, (call, got) in enumerate(zip(transcript["calls"], results)):
        old = call["expect"]
        where = f"{path.stem} call {index} ({call['tool']})"
        for field in sorted((set(old) | set(got)) - set(DATA_FIELDS)):
            if old.get(field) != got.get(field):
                problems.append(f"{where}: {field} was {old.get(field)!r}, is {got.get(field)!r}")
        if old.get("data_sha") != got.get("data_sha"):
            if old.get("data_sha") is None or goldens_replay.digest(without(data[index], new_keys)) != old["data_sha"]:
                problems.append(f"{where}: data changed beyond --new-keys {new_keys} "
                                f"(data_sha {old.get('data_sha')} -> {got.get('data_sha')})")
        if got != old:
            changed += 1
            call["expect"] = got
    if len(results) != len(transcript["calls"]):
        problems.append(f"{path.stem}: replayed {len(results)} of {len(transcript['calls'])} calls")
    if not outputs:
        problems.append(f"{path.stem}: saved nothing")
    else:
        output = outputs[-1]
        recorded = transcript.get("output", {})
        if (output.name != recorded.get("name")
                or goldens_replay.package_digest(output.data) != recorded.get("parts_sha256")):
            problems.append(f"{path.stem}: the saved output is not the one recorded")
    return transcript, problems, changed


def rerecord(path: Path) -> tuple[dict, list[str]]:
    """The transcript with every expectation as the session now returns it, and the list of
    what changed (``<call> <tool> <field>: old -> new``)."""
    from test_tools_goldens import inputs_of

    transcript = json.loads(path.read_text(encoding="utf-8"))
    results, outputs = goldens_replay.replay(transcript, inputs_of(transcript))
    changes = []
    for index, (call, got) in enumerate(zip(transcript["calls"], results)):
        old = call["expect"]
        for field in sorted(set(old) | set(got)):
            if old.get(field) != got.get(field):
                changes.append(f"{path.stem} call {index} ({call['tool']}) {field}: "
                               f"{json.dumps(old.get(field), ensure_ascii=False)[:200]} -> "
                               f"{json.dumps(got.get(field), ensure_ascii=False)[:200]}")
        call["expect"] = got
    if len(results) != len(transcript["calls"]):
        changes.append(f"{path.stem}: replayed {len(results)} of {len(transcript['calls'])} calls")
    if outputs:
        last = outputs[-1]
        digest = goldens_replay.package_digest(last.data)
        if transcript.get("output") != {"name": last.name, "parts_sha256": digest}:
            changes.append(f"{path.stem} output: {transcript.get('output')} -> "
                           f"{{'name': {last.name!r}, 'parts_sha256': {digest!r}}}")
        transcript["output"] = {"name": last.name, "parts_sha256": digest}
    return transcript, changes


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("names", nargs="*", help="transcripts whose name starts with one of these (default: all)")
    parser.add_argument("--new-keys", default="", help="comma-separated keys a change added to results")
    parser.add_argument("--dry-run", action="store_true", help="write nothing")
    parser.add_argument("--rerecord", action="store_true",
                        help="record every expectation as it now comes out, listing each change")
    args = parser.parse_args(argv)

    from conftest import OFFICE_FACES_ABSENT

    if OFFICE_FACES_ABSENT:
        print("Office's faces (Calibri, Cambria, Georgia, Aptos) are not all installed here; the "
              "transcripts are recorded where they are.", file=sys.stderr)
        return 2
    new_keys = [key.strip() for key in args.new_keys.split(",") if key.strip()]
    paths = [p for p in sorted(TRANSCRIPTS.glob("*.json"))
             if not args.names or any(p.stem.startswith(name) for name in args.names)]
    if args.rerecord:
        for path in paths:
            transcript, changes = rerecord(path)
            failing = [c["tool"] for c in transcript["calls"] if not c["expect"]["ok"]]
            print(f"{path.stem}: {len(changes)} change(s)" + (f"; not written, a call fails: {failing}"
                                                               if failing else ""))
            for change in changes:
                print("  " + change)
            if not failing and changes and not args.dry_run:
                path.write_text(json.dumps(transcript, indent=1, ensure_ascii=False) + "\n", encoding="utf-8")
        return 0
    refreshed, problems = [], []
    for path in paths:
        transcript, found, changed = refresh(path, new_keys)
        problems += found
        print(f"{path.stem}: {changed} expectation(s) {'differ' if found else 'to write' if changed else 'as recorded'}")
        if changed and not found:
            refreshed.append((path, transcript))
    if problems:
        print("\nNot written -- these do not replay as recorded:", file=sys.stderr)
        for problem in problems:
            print("  " + problem, file=sys.stderr)
        return 1
    if args.dry_run:
        return 0
    for path, transcript in refreshed:
        path.write_text(json.dumps(transcript, indent=1, ensure_ascii=False) + "\n", encoding="utf-8")
    print(f"wrote {len(refreshed)} transcript(s)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
