"""rust.yml's fuzz job's seed corpora from this checkout's documents.

    python3 tools/fuzz_seeds.py DIR

DIR/package (each document under python/ with a FUZZ_SEED_SUFFIXES suffix, for zip_read and
opc_open), DIR/xml (each XML part of them, for xml_roundtrip, chart_layout, drawing_render and
vml_paint), DIR/metafile (the metafiles in ooxml-oracle's recorded case bundle, for
metafile_draw), DIR/font (the .ttf and .otf faces under python/, for glyph_ink) and
DIR/markdown (the Markdown under python/, for markdown_parse), every distinct file once, none
over 1 MiB.  Prints the count per kind as JSON.

Standard library only, Python 3.10+.
"""

from __future__ import annotations

import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
PY = ROOT / "python"

#: The documents this script takes, and the parts of them it takes for the XML target (as
#: tools/rust_oracles.py does), up to libFuzzer's -max_len in rust.yml.
FUZZ_SEED_SUFFIXES = (".docx", ".docm", ".dotx", ".dotm", ".pptx", ".pptm", ".potx", ".xlsx", ".xlsm", ".xltx")
FUZZ_XML_SUFFIXES = (".xml", ".rels", ".vml")
FUZZ_FONT_SUFFIXES = (".ttf", ".otf")
FUZZ_MAX_LEN = 1 << 20
METAFILE_CASES = ROOT / "crates" / "ooxml-oracle" / "tests" / "data" / "metafile-cases.gz"


def fuzz_seeds(out: Path) -> dict[str, int]:
    import hashlib
    import zipfile

    counts = {"package": 0, "xml": 0, "metafile": 0, "font": 0, "markdown": 0}
    seen: set[str] = set()

    def keep(kind: str, data: bytes) -> None:
        digest = hashlib.sha256(data).hexdigest()
        if len(data) <= FUZZ_MAX_LEN and digest not in seen:
            seen.add(digest)
            (out / kind / digest[:20]).write_bytes(data)
            counts[kind] += 1

    for kind in counts:
        (out / kind).mkdir(parents=True, exist_ok=True)
    for path in sorted(PY.rglob("*")):
        if not path.is_file() or ".venv" in path.parts:
            continue
        if path.suffix.lower() in FUZZ_FONT_SUFFIXES:
            keep("font", path.read_bytes())  # the faces the repository ships, for glyph_ink
            continue
        if path.suffix.lower() == ".md":
            keep("markdown", path.read_bytes())  # the documentation's Markdown, for markdown_parse
            continue
        if path.suffix.lower() not in FUZZ_SEED_SUFFIXES:
            continue
        data = path.read_bytes()
        keep("package", data)
        try:
            with zipfile.ZipFile(path) as archive:
                for info in archive.infolist():
                    if info.filename.lower().endswith(FUZZ_XML_SUFFIXES) and info.file_size <= FUZZ_MAX_LEN:
                        keep("xml", archive.read(info))
        except (zipfile.BadZipFile, OSError, RuntimeError, NotImplementedError):
            continue  # a deliberately damaged document: a package seed only
    for data in metafile_cases():
        keep("metafile", data)
    return counts


def metafile_cases(path: Path = METAFILE_CASES) -> list[bytes]:
    """The metafiles in ooxml-oracle's recorded case bundle (the repository has no .emf or
    .wmf files; its tests build them): a gzip stream of length-prefixed cases, each a list
    of length-prefixed fields, field 0 the kind and field 1 a metafile's bytes."""
    import gzip

    def fields(data: bytes) -> list[bytes]:
        out, at = [], 0
        while at + 4 <= len(data):
            n = int.from_bytes(data[at:at + 4], "little")
            out.append(data[at + 4:at + 4 + n])
            at += 4 + n
        return out

    if not path.is_file():
        return []
    blob = gzip.decompress(path.read_bytes())
    found, at = [], 0
    while at + 4 <= len(blob):
        n = int.from_bytes(blob[at:at + 4], "little")
        case = fields(blob[at + 4:at + 4 + n])
        if len(case) > 1 and case[0] == b"metafile":
            found.append(case[1])
        at += 4 + n
    return found


if __name__ == "__main__":
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    print(json.dumps(fuzz_seeds(Path(sys.argv[1]))))
