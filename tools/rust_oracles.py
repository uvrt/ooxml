"""Oracles from today's Python path for the Rust core's phase 0 contracts
(docs/architecture/README.md, section 15).

    python tools/rust_oracles.py xml-pairs OUT [PATH ...]
    python tools/rust_oracles.py touched-save OUT [PATH ...]
    python tools/rust_oracles.py compare-saves OUT

``xml-pairs``: every XML part (``.xml``, ``.rels``, ``.vml``) of every package found under the
PATHs (files or directories; default: ``python/``, all seven packages' tests and fixtures),
each distinct part once: ``N.raw`` (its bytes) and ``N.ref`` (what
``ooxml_edit.xml.serialize(parse_xml(raw))`` writes), or ``N.err`` when lxml refuses it.
``crates/ooxml-xml/examples/xml_contract.rs`` checks the Rust writer against them.

``touched-save``: for every package, ``N.in`` (its bytes) and ``N.py`` (what
``OpcPackage.to_bytes()`` writes with every part touched: each XML part's tree marked dirty,
every other part replaced by its own bytes).  ``crates/ooxml-opc/examples/save_contract.rs``
writes ``N.rs`` (the Rust package's save with every part touched) and ``N.same`` (its
unchanged save); ``compare-saves`` then checks that ``N.same`` equals ``N.in`` byte for byte
and that ``N.rs`` has ``N.py``'s part entries: names, order, metadata and uncompressed bytes.
Directory entries (``word/``) are not parts: the Rust package copies them raw, as every
untouched entry, where ``OpcPackage`` leaves them out; they are counted, not compared.

Needs ooxml-edit and ooxml-common importable (this checkout's ``python/*/src``).
"""

from __future__ import annotations

import hashlib
import json
import sys
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
PACKAGE_SUFFIXES = (".docx", ".docm", ".dotx", ".dotm", ".pptx", ".pptm", ".potx",
                    ".xlsx", ".xlsm", ".xltx")
XML_SUFFIXES = (".xml", ".rels", ".vml")
#: What ZipInfo says about an entry that the comparison checks (compress_size and
#: header_offset follow from the data and the order).
ENTRY_FIELDS = ("filename", "date_time", "compress_type", "comment", "extra", "create_system",
                "create_version", "extract_version", "flag_bits", "volume", "internal_attr",
                "external_attr", "CRC", "file_size")


def packages(paths: list[str]) -> list[Path]:
    found: list[Path] = []
    for p in [Path(x) for x in paths] or [ROOT / "python"]:
        if p.is_file():
            found.append(p)
        else:
            found.extend(f for f in sorted(p.rglob("*")) if f.suffix.lower() in PACKAGE_SUFFIXES
                         and f.is_file() and ".venv" not in f.parts)
    return found


def xml_pairs(out: Path, paths: list[str]) -> dict:
    from ooxml_edit.xml import parse_xml, serialize

    out.mkdir(parents=True, exist_ok=True)
    seen: set[str] = set()
    counts = {"packages": 0, "not_zip": 0, "parts": 0, "refused": 0}
    for package in packages(paths):
        try:
            archive = zipfile.ZipFile(package)
        except zipfile.BadZipFile:
            counts["not_zip"] += 1
            continue
        counts["packages"] += 1
        for info in archive.infolist():
            if not info.filename.lower().endswith(XML_SUFFIXES):
                continue
            raw = archive.read(info)
            digest = hashlib.sha256(raw).hexdigest()
            if digest in seen:
                continue
            seen.add(digest)
            n = len(seen) - 1
            (out / f"{n:05d}.raw").write_bytes(raw)
            try:
                (out / f"{n:05d}.ref").write_bytes(serialize(parse_xml(raw)))
                counts["parts"] += 1
            except Exception as error:  # lxml's refusal is the oracle's answer
                (out / f"{n:05d}.err").write_text(f"{type(error).__name__}: {error}\n")
                counts["refused"] += 1
    return counts


def touched_save(out: Path, paths: list[str]) -> dict:
    from ooxml_edit.opc import OpcPackage

    out.mkdir(parents=True, exist_ok=True)
    counts = {"packages": 0, "skipped": 0}
    for package in packages(paths):
        data = package.read_bytes()
        try:
            opc = OpcPackage.open(data)
            for name in opc.part_names:
                if name.lower().endswith(XML_SUFFIXES):
                    opc.tree(name)
                    opc.mark_dirty(name)
                else:
                    opc.replace_part(name, opc.read(name))
            saved = opc.to_bytes()
        except Exception as error:
            print(f"{package}: {type(error).__name__}: {error}", file=sys.stderr)
            counts["skipped"] += 1
            continue
        n = counts["packages"]
        (out / f"{n:05d}.in").write_bytes(data)
        (out / f"{n:05d}.py").write_bytes(saved)
        (out / f"{n:05d}.name").write_text(str(package.relative_to(ROOT) if package.is_relative_to(ROOT) else package.name))
        counts["packages"] += 1
    return counts


def entries(data: bytes) -> list[tuple]:
    with zipfile.ZipFile(__import__("io").BytesIO(data)) as archive:
        return [tuple(getattr(i, f) for f in ENTRY_FIELDS) + (archive.read(i),)
                for i in archive.infolist() if not i.is_dir()]


def compare_saves(out: Path) -> dict:
    counts = {"packages": 0, "unchanged_identical": 0, "touched_equal": 0,
              "with_directory_entries": 0, "different": []}
    for source in sorted(out.glob("*.in")):
        counts["packages"] += 1
        name = source.with_suffix(".name").read_text()
        same, rust, python = (source.with_suffix(s) for s in (".same", ".rs", ".py"))
        if same.exists() and same.read_bytes() == source.read_bytes():
            counts["unchanged_identical"] += 1
        else:
            counts["different"].append(f"{name}: the unchanged save is not the input")
        if not rust.exists():
            counts["different"].append(f"{name}: no Rust save")
            continue
        with zipfile.ZipFile(source) as archive:
            counts["with_directory_entries"] += any(i.is_dir() for i in archive.infolist())
        ours, theirs = entries(rust.read_bytes()), entries(python.read_bytes())
        if ours == theirs:
            counts["touched_equal"] += 1
            continue
        for a, b in zip(ours, theirs):
            if a != b:
                fields = [f for f, x, y in zip(ENTRY_FIELDS + ("bytes",), a, b) if x != y]
                counts["different"].append(f"{name}: {a[0]}: {fields}")
                break
        else:
            counts["different"].append(f"{name}: {len(ours)} entries against {len(theirs)}")
    return counts


def main(argv: list[str]) -> int:
    if len(argv) < 2 or argv[0] not in ("xml-pairs", "touched-save", "compare-saves"):
        print(__doc__, file=sys.stderr)
        return 2
    command, out, rest = argv[0], Path(argv[1]), argv[2:]
    if command == "xml-pairs":
        result = xml_pairs(out, rest)
    elif command == "touched-save":
        result = touched_save(out, rest)
    else:
        result = compare_saves(out)
    print(json.dumps(result, indent=1))
    return 1 if result.get("different") else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
