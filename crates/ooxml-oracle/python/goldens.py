"""The gates' goldens (architecture review decision 2, R4 #2): each case of the differential
gates frozen as data, and the Rust core's answer to it blessed once, so that the gates run the
Rust oracles alone -- no docx-agent, no reference cache.

What is in git (crates/ooxml-oracle; cases/README.md has the format):

* ``cases/<tree>/<document>.jsonl``: every case of every mode on one document (``<tree>``:
  fixtures or corpus; ``<document>``: its path below the tree), one JSON object a line,
  ``{"id", "mode", "case"}``.  ``case`` is what the oracle binary is given: an edit mode's case
  as the case makers build it, a read mode's manifest item (less its ``file``).  A case run on
  another document than its own (a French copy, a reopened copy, a made-up document) names it
  in ``case["input"]``: ``$INPUT/<digest>.docx``, a file of ``cases/inputs``.  Cases on
  documents no tree holds (made-up ones) are under the document name ``_made``.  ``id`` is
  ``<mode>:<16 hex of the case's digest>``: the same whatever the run's sampling.
* ``goldens/<tree>/<document>.jsonl``: per case ``{"id", "digest", "source"}`` -- the digest of
  the Rust oracle's answer, normalised (:func:`normalised`) -- or ``{"id", "not_supported",
  "source"}`` where the Rust tools refused the form as not supported yet when it was blessed;
  docx-agent's answer to such a case is kept in ``goldens/pending/<tree>/<document>.jsonl.gz``
  (compacted, as the reference cache kept it) for the lane that serves the form to compare with.
* ``source``: where the answer comes from -- the frozen Python it equalled
  (:data:`FROZEN_PYTHON`), a known difference's cause (the golden took Rust's answer), a Word
  observation, or the reason a bless gave.

Paths in cases are portable (:func:`portable`): ``$ROOT`` the repository, ``$HOME`` the home
folder (the runners' substitute fonts), ``$INPUT`` cases/inputs.

This module is stdlib only (the gates' environment and Python 3.10, where the oracle's tests
run): the harnesses record with it (:class:`Recorder`), golden_gate.py runs the gates with it
and tools/gates_goldens.py blesses with it.
"""

from __future__ import annotations

import copy
import gzip
import hashlib
import json
import os
import shutil
from pathlib import Path

ORACLE = Path(__file__).resolve().parents[1]
ROOT = ORACLE.parents[1]
CASES = ORACLE / "cases"
GOLDENS = ORACLE / "goldens"
INPUTS = CASES / "inputs"
PENDING = GOLDENS / "pending"
#: The document name of the cases on documents no tree holds (made-up documents).
MADE = "_made"
#: A gate's tree: the folder below cases/ and goldens/ that holds its documents' files.
TREE_OF_GATE = {"fixtures": "fixtures", "reads": "fixtures", "corpus": "corpus"}
#: The source of a golden whose Rust answer equalled the frozen reference's.
FROZEN_PYTHON = "frozen Python {sha}, unmeasured"
#: The source of a golden that took Rust's answer where a known difference covered the case.
KNOWN = "known difference: {cause}"
DIGEST = "sha256:"
#: The tokens of portable paths, longest first where one holds another.
_TOKENS = ("$INPUT", "$ROOT", "$HOME")


def canonical(value) -> str:
    """``value`` as one JSON text: keys sorted, no spaces, Unicode kept."""
    return json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=False)


def _sha(data: str | bytes) -> str:
    return hashlib.sha256(data.encode() if isinstance(data, str) else data).hexdigest()


def case_id(mode: str, case: dict) -> str:
    """A case's id: its mode and its digest (the same in every run whatever its sampling)."""
    return f"{mode}:{_sha(canonical(case))[:16]}"


def _roots() -> list[tuple[str, str]]:
    home = os.path.realpath(os.path.expanduser("~"))
    pairs = [("$INPUT", str(INPUTS)), ("$ROOT", str(ROOT)), ("$HOME", home), ("$HOME", os.path.expanduser("~"))]
    return [(token, folder) for token, folder in pairs if folder not in ("", "/")]


def portable(value):
    """``value`` with each string that is a path in the repository, in cases/inputs or in the
    home folder written with its token (``$ROOT/...``, ``$INPUT/...``, ``$HOME/...``)."""
    if isinstance(value, dict):
        return {k: portable(v) for k, v in value.items()}
    if isinstance(value, list):
        return [portable(v) for v in value]
    if isinstance(value, str) and value.startswith("/"):
        for token, folder in _roots():
            if value == folder or value.startswith(folder + "/"):
                return token + value[len(folder):]
    return value


def local(value, inputs: Path = INPUTS):
    """``value`` with each token of :func:`portable` written as this machine's path
    (``inputs``: where ``$INPUT`` is)."""
    if isinstance(value, dict):
        return {k: local(v, inputs) for k, v in value.items()}
    if isinstance(value, list):
        return [local(v, inputs) for v in value]
    if isinstance(value, str) and value.startswith(_TOKENS):
        token, _, rest = value.partition("/")
        base = {"$INPUT": str(inputs), "$ROOT": str(ROOT), "$HOME": os.path.expanduser("~")}[token]
        return base + ("/" + rest if rest else "")
    return value


def _compact_parts(value):
    """``value`` with every ``parts`` map's texts replaced by their digests (the reference
    cache's ``compact``, at any depth: an edit's parts and a tool reply's)."""
    if isinstance(value, dict):
        out = {}
        for key, item in value.items():
            if key == "parts" and isinstance(item, dict):
                out[key] = {name: DIGEST + _sha(text) if isinstance(text, str) else _compact_parts(text)
                            for name, text in item.items()}
            else:
                out[key] = _compact_parts(item)
        return out
    if isinstance(value, list):
        return [_compact_parts(v) for v in value]
    return value


#: The keys whose values are a run's own (wall-clock times): a timing map keeps its keys only.
VOLATILE = ("timing",)


def _steady(value):
    if isinstance(value, dict):
        return {k: (sorted(v) if k in VOLATILE and isinstance(v, dict) else
                    None if k in VOLATILE and isinstance(v, (int, float)) else _steady(v))
                for k, v in value.items()}
    if isinstance(value, list):
        return [_steady(v) for v in value]
    return value


def normalised(line: dict) -> dict:
    """A Rust oracle's answer to one case as the golden keeps it: the line less its ``file``
    and ``case`` (the run's names for the case), each changed part as its digest, wall-clock
    timings as their names only, paths portable."""
    answer = {k: v for k, v in line.items() if k not in ("file", "case")}
    return portable(_steady(_compact_parts(answer)))


def digest(line: dict) -> str:
    """The digest of :func:`normalised` ``line``: what a golden holds."""
    return digest_of(normalised(line))


#: A golden's digest: the first 32 hex digits (128 bits) of the SHA-256, as goldens are
#: compared within one case; the 170,000 goldens' digests are most of their files' size.
DIGEST_HEX = 32


def digest_of(answer: dict) -> str:
    """The digest of an answer already normalised."""
    return DIGEST + _sha(canonical(answer))[:DIGEST_HEX]


def compact(result):
    """docx-agent's result as the pending goldens keep it: parts as digests."""
    return _compact_parts(copy.deepcopy(result))


# --- files ------------------------------------------------------------------------------------

def document_file(base: Path, tree: str, document: str, suffix: str = ".jsonl") -> Path:
    """The file of ``document``'s cases or goldens below ``base`` (CASES, GOLDENS, PENDING)."""
    return base / tree / (document + suffix)


def read_lines(path: Path) -> list[dict]:
    if not path.is_file():
        return []
    opener = gzip.open if path.suffix == ".gz" else open
    with opener(path, "rt", encoding="utf-8") as handle:
        return [json.loads(line) for line in handle if line.strip()]


def write_lines(path: Path, lines: list[dict]) -> None:
    """``lines`` canonical, one a line (gzip where the name ends in .gz, with no timestamp so
    the same lines give the same bytes); the file removed when there are none."""
    if not lines:
        path.unlink(missing_ok=True)
        return
    path.parent.mkdir(parents=True, exist_ok=True)
    text = "".join(canonical(line) + "\n" for line in lines).encode("utf-8")
    if path.suffix == ".gz":
        with open(path, "wb") as raw, gzip.GzipFile(fileobj=raw, mode="wb", mtime=0) as handle:
            handle.write(text)
    else:
        path.write_bytes(text)


def documents(tree: str, base: Path | None = None) -> list[str]:
    """The documents ``tree`` has files for below ``base`` (CASES when None; their names below
    the tree), sorted."""
    folder = (CASES if base is None else base) / tree
    return sorted(str(p.relative_to(folder))[:-len(".jsonl")] for p in folder.rglob("*.jsonl")) \
        if folder.is_dir() else []


def cases_of(tree: str, document: str, mode: str) -> list[dict]:
    """``document``'s frozen cases of ``mode``, in their order."""
    return [line for line in read_lines(document_file(CASES, tree, document)) if line["mode"] == mode]


def goldens_of(tree: str, document: str) -> dict[str, dict]:
    """``document``'s goldens by case id."""
    return {line["id"]: line for line in read_lines(document_file(GOLDENS, tree, document))}


def sample(cases: list[dict], per_tool: int, by_tool: bool) -> list[dict]:
    """The first ``per_tool`` cases of each tool (``by_tool``: a tool mode's cases) or of the
    document (0: all), in their order."""
    if not per_tool:
        return cases
    if not by_tool:
        return cases[:per_tool]
    seen: dict[str, int] = {}
    out = []
    for line in cases:
        tool = str(line["case"].get("tool"))
        seen[tool] = seen.get(tool, 0) + 1
        if seen[tool] <= per_tool:
            out.append(line)
    return out


# --- recording ----------------------------------------------------------------------------

class Recorder:
    """What a harness run answers, per case, for goldens: the case as the Rust oracle was given
    it, its normalised answer and digest, whether docx-agent's answer equalled it, the form it
    was refused in as not supported yet (with docx-agent's answer, compacted).

    ``trees`` are the folders whose documents have case files (the run's tree).  ``owners``
    (:meth:`own`) maps a document the Rust side ran that is a copy of a tree's document (a
    French copy, a reopened copy) to that document.  A document the run read that is not a
    tree's (a copy, a made-up document, a document named by a case) is frozen into
    ``inputs`` by its digest and named ``$INPUT/<digest>.docx``.
    """

    def __init__(self, path: str, trees: list[str], inputs: str | None = None):
        self.path = Path(path)
        self.trees = [Path(t).resolve() for t in trees]
        self.inputs = Path(inputs) if inputs else self.path.parent / "inputs"
        self.owners: dict[str, str] = {}
        self.cases: dict[tuple, tuple[str | None, dict]] = {}
        #: each answer normalised as it came (the harness's comparison changes the lines it
        #: settles), and the form it was refused in, by the harness's key
        self.answers: dict[tuple, tuple[dict, str | None]] = {}
        self.records: list[dict] = []

    def own(self, copy_path: str, document: str) -> None:
        self.owners[str(copy_path)] = str(document)

    def freeze(self, path: str) -> str:
        data = Path(path).read_bytes()
        name = f"{_sha(data)[:32]}.docx"
        self.inputs.mkdir(parents=True, exist_ok=True)
        target = self.inputs / name
        if not target.exists():
            shutil.copyfile(path, target)
        return f"$INPUT/{name}"

    def _case(self, file: str, case: dict) -> tuple[str | None, dict]:
        """The case's document (a tree's, or None: made-up) and the case as frozen."""
        case = copy.deepcopy(case)
        if case.get("opened"):
            case["opened"] = self.freeze(case["opened"])
        owner = self.owners.get(file)
        inside = any(Path(file).resolve().is_relative_to(tree) for tree in self.trees)
        document = owner or (file if inside else None)
        if document != file:
            case["input"] = self.freeze(file)
        return document, portable(case)

    def manifest(self, manifest: list[dict], lines: dict[tuple, dict], refused) -> None:
        """The cases a run gave the Rust oracle (an edit mode's ``{"file", "cases"}`` entries,
        a read mode's items) and its answers, by the harness's keys, read before the harness
        compares them (``refused(line)``: the form a line was refused in, oracle_run.py's)."""
        for item in manifest:
            file = item["file"]
            if "cases" in item:
                for k, case in enumerate(item["cases"]):
                    self.cases[(file, k)] = self._case(file, case)
            else:
                self.cases[(file, item.get("case"))] = self._case(
                    file, {k: v for k, v in item.items() if k != "file"})
        for key, line in lines.items():
            self.answers[key] = (normalised(line), refused(line))

    def refused(self, key: tuple) -> str | None:
        """The form the Rust side refused the case under ``key`` in, if it did."""
        return self.answers.get(key, (None, None))[1]

    def verdict(self, key: tuple, differs: bool, refused: str | None, python=None) -> None:
        """One case's comparison: ``differs`` (docx-agent's answer and Rust's), ``refused`` (the
        form Rust refused as not supported yet) with docx-agent's answer ``python``."""
        if key not in self.cases:
            return
        document, case = self.cases[key]
        answer = self.answers.get(key)
        record = {"file": document, "case": case}
        if refused:
            record.update(not_supported=refused, python=compact(python))
        elif answer is None:
            record.update(differs=True, missing=True)
        else:
            record.update(digest=digest_of(answer[0]), output=answer[0], differs=bool(differs))
        self.records.append(record)

    def close(self) -> None:
        write_lines(self.path, self.records)
