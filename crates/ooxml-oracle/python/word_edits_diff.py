#!/usr/bin/env python3
"""The Word edits' differential harness (test support; phase P2, lane P2a).

Runs ``set_text`` in docx-agent and in the Rust core (``word-edits``) on the same documents,
untracked and tracked, each case on the document as opened, and compares:

* ``result``: the paragraph's id after the edit, whether it changed, the renames its
  stamping made (in order);
* ``parts``: every part whose bytes changed, byte for byte as each side writes it, and the
  archive's part order;
* ``touched``: the parts and the body's heading sections the change touched;
* ``validation``: the problems after the edit against those the document opened with
  (``new``, ``fixed``, ``baseline``), as the save gate compares them;
* ``next``: the value the main part's sequence and its text-id sequence give next.

Every difference is printed with its JSON path; the exit status is 1 when there is any.

    python crates/ooxml-oracle/python/word_edits_diff.py --rust target/release/word-edits --tree DIR

Python's behaviour is not changed: this calls docx-agent's public and documented internal
functions only.
"""

from __future__ import annotations

import argparse
import copy
import io
import sys
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
for package in ("docx-agent", "docx2svg", "ooxml-edit", "ooxml-common"):
    source = ROOT / "python" / package / "src"
    if str(source) not in sys.path:
        sys.path.insert(0, str(source))
sys.path.insert(0, str(Path(__file__).resolve().parent))

from docx_agent import Document  # noqa: E402
from docx_agent import validate as _validate  # noqa: E402
from docx_agent.edit import ids as _ids  # noqa: E402
from docx_agent.tools.touched import body_sections  # noqa: E402
from lxml import etree  # noqa: E402
from ooxml_edit.tools.savegate import compare_problems  # noqa: E402
from ooxml_edit.xml import serialize  # noqa: E402
from word_reads_diff import differences, files_of, report, run_rust  # noqa: E402

TRACKING = {"author": "Writer A", "date": "2026-11-17T10:00:00Z"}
#: Marks the styled reading of word_set_text takes as emphasis; the plain path only here.
_STYLED = ("*", "\\", "]{")


def _parts(data: bytes) -> dict[str, bytes]:
    with zipfile.ZipFile(io.BytesIO(data)) as archive:
        return {name: archive.read(name) for name in archive.namelist() if not name.endswith("/")}


def _order(data: bytes) -> list[str]:
    with zipfile.ZipFile(io.BytesIO(data)) as archive:
        return [n for n in archive.namelist() if not n.endswith("/")]


def _texts(text: str) -> list[str]:
    """The new texts tried on a paragraph holding ``text``."""
    words = text.split()
    out = []
    if words:
        out.append(text.replace(words[0], "Changed", 1) + " more")
        out.append(" ".join(reversed(words)))
    out += ["A new sentence, written whole.", "", "Tab\tthen\vbreak", text]
    return [t for t in out if not any(m in t for m in _STYLED)]


def _targets(document) -> list[str]:
    """Paragraphs to edit: the first body paragraph with text, one in a table, one in a
    header or note, one with a field or hyperlink, the last body paragraph."""
    found: list[str] = []
    body = document._index(document.package.document_part())
    paragraphs = [e for e in body.paragraphs if _text_of(e)]
    if paragraphs:
        found.append(paragraphs[0].id)
        found.append(paragraphs[-1].id)
        mid = paragraphs[len(paragraphs) // 2]
        found.append(mid.id)
    for entry in body.paragraphs:
        if entry.element.getparent() is not None and entry.element.getparent().tag.endswith("}tc") and _text_of(entry):
            found.append(entry.id)
            break
    for entry in body.paragraphs:
        xml = entry.element
        if any(n.tag.endswith(("}hyperlink", "}fldSimple", "}fldChar")) for n in xml.iter()):
            found.append(entry.id)
            break
    for part in document._parts()[1:]:
        index = document._index(part)
        texts = [e for e in index.paragraphs if _text_of(e)]
        if texts:
            found.append(texts[0].id)
            break
    return list(dict.fromkeys(found))


def _text_of(entry) -> str:
    from docx_agent.edit import text as _text

    return _text.paragraph_text(entry.element)


def python_case(data: bytes, case: dict, baseline: list) -> dict:
    document = Document.open(data)
    before = _parts(data)
    try:
        if case["tracked"]:
            with document.tracking(**case["tracked"]):
                result = document._set_text(case["target"], case["text"])
        else:
            result = document._set_text(case["target"], case["text"])
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
    problems = _validate.check(document.package)
    validation = compare_problems(baseline, problems, key=lambda p: (p.code, p.part, str(p)))
    pool = copy.deepcopy(document._id_pool())
    next_ids = {seed: _ids.generate(seed, pool, 1) for seed in (main, main + "\0text")}
    return {
        "result": {"id": result.id, "changed": result.changed, "renamed": dict(result.renamed)},
        "parts": parts,
        "order": _order(out),
        "touched": touched,
        "touched_as_written": as_written,
        "validation": validation,
        "next": next_ids,
    }


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


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--rust", required=True, help="the word-edits binary")
    parser.add_argument("--tree", action="append", help="every .docx below this folder")
    parser.add_argument("--limit", type=int, default=0, help="at most this many cases per document")
    parser.add_argument("files", nargs="*")
    args = parser.parse_args(argv)
    expected: dict = {}
    manifest = []
    for path in files_of(args):
        data = path.read_bytes()
        try:
            document = Document.open(data)
            baseline = _validate.check(document.package)
            cases = []
            for target in _targets(document):
                text = document.paragraph(target).text
                for new in _texts(text):
                    for tracked in (None, TRACKING):
                        cases.append({"target": target, "text": new, "tracked": tracked})
            if args.limit:
                cases = cases[:args.limit]
        except Exception as error:  # noqa: BLE001
            print(f"{path}: python failed: {error}")
            continue
        for k, case in enumerate(cases):
            expected[(str(path), k)] = python_case(data, case, baseline)
        manifest.append({"file": str(path), "cases": cases})
    rust = run_rust(args.rust, manifest)
    got = {key: value.get("rust") for key, value in rust.items()}
    rewritten = 0
    for key, want in expected.items():
        have = got.get(key)
        written = want.pop("touched_as_written", None)
        if isinstance(have, dict) and "refused" in have and "refused" in want:
            got[key] = want      # both refused: the messages are each side's own words
        elif isinstance(have, dict) and "parts" in have and "parts" in want:
            _shorten(want["parts"], have["parts"])
            if have.get("touched") != want["touched"] and have.get("touched") == written:
                rewritten += 1   # equal once the body before is as lxml writes it
                want["touched"] = written
    if rewritten:
        print(f"{rewritten} cases where docx-agent's touched counts blocks only re-serialized (equal "
              "once the body before is read as lxml writes it)")
    return report(expected, got)


def _shorten(want: dict, have: dict, context: int = 160) -> None:
    """Two parts that differ, cut to the stretch around their first difference."""
    for name in set(want) & set(have):
        a, b = want[name], have[name]
        if a == b:
            continue
        at = next((k for k, (x, y) in enumerate(zip(a, b)) if x != y), min(len(a), len(b)))
        start = max(0, at - context)
        want[name] = f"@{at}: {a[start:at + context]}"
        have[name] = f"@{at}: {b[start:at + context]}"


if __name__ == "__main__":
    sys.exit(main())
