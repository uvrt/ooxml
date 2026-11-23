"""goldens.py and golden_gate.py: the frozen cases' format, the answers' normalisation and
digests, the recorder the harnesses record with, and how the gate on goldens judges a case."""

from __future__ import annotations

import gzip
import sys
from pathlib import Path

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

import golden_gate  # noqa: E402
import goldens  # noqa: E402

EDIT = {"tool": "word_fields", "arguments": {"doc": "d1", "action": "list"}, "tracked": None}


def test_a_case_id_is_its_mode_and_digest_whatever_the_key_order() -> None:
    again = {"tracked": None, "arguments": {"action": "list", "doc": "d1"}, "tool": "word_fields"}
    assert goldens.case_id("structure", EDIT) == goldens.case_id("structure", again)
    assert goldens.case_id("structure", EDIT).startswith("structure:")
    assert goldens.case_id("objects", EDIT) != goldens.case_id("structure", EDIT)


def test_paths_are_portable_and_back() -> None:
    home = str(Path("~").expanduser())
    case = {"reference_file": str(goldens.ROOT / "crates/x.docx"), "fonts": {"folders": [[home + "/fonts", 1]]},
            "input": str(goldens.INPUTS / "ab.docx"), "text": "/not/a/known/root"}
    frozen = goldens.portable(case)
    assert frozen == {"reference_file": "$ROOT/crates/x.docx", "fonts": {"folders": [["$HOME/fonts", 1]]},
                      "input": "$INPUT/ab.docx", "text": "/not/a/known/root"}
    assert goldens.local(frozen) == case


def test_an_answer_is_normalised() -> None:
    line = {"file": "/tmp/x.docx", "case": 3,
            "rust": {"parts": {"word/document.xml": "<w:document/>"}, "reply": {"data": {"timing": {"layout": 12}}}}}
    out = goldens.normalised(line)
    assert set(out) == {"rust"}
    assert out["rust"]["parts"]["word/document.xml"].startswith(goldens.DIGEST)
    assert out["rust"]["reply"]["data"]["timing"] == ["layout"]
    slower = {**line, "file": "/elsewhere/x.docx", "rust": {**line["rust"], "reply": {"data": {"timing": {"layout": 99}}}}}
    assert goldens.digest(line) == goldens.digest(slower)
    other = {**line, "rust": {**line["rust"], "parts": {"word/document.xml": "<w:document></w:document>"}}}
    assert goldens.digest(line) != goldens.digest(other)


def test_sampling_takes_the_first_cases_of_each_tool() -> None:
    lines = [{"case": {"tool": t}} for t in ("a", "a", "b", "a", "b", "b")]
    assert [x["case"]["tool"] for x in goldens.sample(lines, 2, True)] == ["a", "a", "b", "b"]
    assert len(goldens.sample(lines, 2, False)) == 2
    assert goldens.sample(lines, 0, True) == lines


def test_lines_are_written_the_same_every_time(tmp_path: Path) -> None:
    lines = [{"b": 1, "a": "é"}, {"id": "x"}]
    for name in ("a.jsonl", "a.jsonl.gz"):
        goldens.write_lines(tmp_path / name, lines)
        first = (tmp_path / name).read_bytes()
        goldens.write_lines(tmp_path / name, lines)
        assert (tmp_path / name).read_bytes() == first
        assert goldens.read_lines(tmp_path / name) == lines
    assert gzip.decompress((tmp_path / "a.jsonl.gz").read_bytes()).decode() == '{"a":"é","b":1}\n{"id":"x"}\n'
    goldens.write_lines(tmp_path / "a.jsonl", [])
    assert not (tmp_path / "a.jsonl").exists()


def test_the_recorder_freezes_copies_and_made_up_documents(tmp_path: Path) -> None:
    tree = tmp_path / "tree"
    tree.mkdir()
    (tree / "a.docx").write_bytes(b"A")
    (tmp_path / "french-a.docx").write_bytes(b"A in French")
    (tmp_path / "made.docx").write_bytes(b"made up")
    recorder = goldens.Recorder(str(tmp_path / "out" / "raw.jsonl.gz"), [str(tree)])
    recorder.own(str(tmp_path / "french-a.docx"), str(tree / "a.docx"))
    manifest = [{"file": str(tree / "a.docx"), "cases": [EDIT, {"op": "x"}]},
                {"file": str(tmp_path / "french-a.docx"), "cases": [EDIT]},
                {"file": str(tmp_path / "made.docx"), "case": "fact", "fact": "fact"}]
    lines = {(str(tree / "a.docx"), 0): {"file": "", "case": 0, "rust": {"parts": {}}},
             (str(tree / "a.docx"), 1): {"file": "", "case": 1, "rust": {"not_supported": "a form"}},
             (str(tmp_path / "french-a.docx"), 0): {"file": "", "case": 0, "rust": {"parts": {}}},
             (str(tmp_path / "made.docx"), "fact"): {"file": "", "case": "fact", "reply": {"data": 1}}}
    recorder.manifest(manifest, lines, lambda line: (line.get("rust") or {}).get("not_supported"))
    lines[(str(tree / "a.docx"), 0)]["rust"]["parts"] = {"changed": "by the comparison"}   # read before
    recorder.verdict((str(tree / "a.docx"), 0), False, None)
    recorder.verdict((str(tree / "a.docx"), 1), False, "a form", {"parts": {"p": "x"}})
    recorder.verdict((str(tmp_path / "french-a.docx"), 0), True, None)
    recorder.verdict((str(tmp_path / "made.docx"), "fact"), False, None)
    recorder.close()
    plain, refused, french, made = goldens.read_lines(tmp_path / "out" / "raw.jsonl.gz")
    assert plain["file"] == str(tree / "a.docx") and "input" not in plain["case"] and not plain["differs"]
    assert plain["output"] == {"rust": {"parts": {}}}
    assert recorder.refused((str(tree / "a.docx"), 1)) == "a form"
    assert refused["not_supported"] == "a form" and refused["python"]["parts"]["p"].startswith(goldens.DIGEST)
    assert french["file"] == str(tree / "a.docx") and french["differs"]
    assert french["case"]["input"].startswith("$INPUT/")
    assert made["file"] is None and made["case"] == {"case": "fact", "fact": "fact", "input": made["case"]["input"]}
    frozen = sorted(p.read_bytes() for p in (tmp_path / "out" / "inputs").iterdir())
    assert frozen == [b"A in French", b"made up"]


def test_the_gate_groups_edit_cases_by_the_file_they_run_on(tmp_path: Path) -> None:
    frozen = [("a.docx", {"id": "m:1", "case": EDIT}), ("a.docx", {"id": "m:2", "case": {**EDIT, "input": "$INPUT/f.docx"}}),
              ("a.docx", {"id": "m:3", "case": {"op": "x"}})]
    manifest, keys = golden_gate.manifest_of("structure", tmp_path, frozen)
    assert [(e["file"], len(e["cases"])) for e in manifest] == [(str(tmp_path / "a.docx"), 2),
                                                                (str(goldens.INPUTS / "f.docx"), 1)]
    assert keys[(str(tmp_path / "a.docx"), 1)][1]["id"] == "m:3"
    assert all("input" not in case for entry in manifest for case in entry["cases"])
    reads, keys = golden_gate.manifest_of("facts", tmp_path, [("_made", {"id": "facts:1",
                                                                         "case": {"case": "f", "input": "$INPUT/g.docx"}})])
    assert reads == [{"file": str(goldens.INPUTS / "g.docx"), "case": "f"}]


@pytest.mark.parametrize(("line", "golden", "differs"), [
    ({"rust": {"x": 1}}, {"digest": goldens.digest({"rust": {"x": 1}}), "source": "s"}, False),
    ({"rust": {"x": 2}}, {"digest": goldens.digest({"rust": {"x": 1}}), "source": "s"}, True),
    ({"rust": {"not_supported": "f"}}, {"not_supported": "f", "source": "s"}, False),
    ({"rust": {"not_supported": "f"}}, {"digest": "sha256:0", "source": "s"}, True),
    ({"rust": {"x": 1}}, {"not_supported": "f", "source": "s"}, True),
    ({"rust": {"x": 1}}, None, True),
    (None, {"digest": "sha256:0", "source": "s"}, True),
])
def test_the_gate_judges_a_case_by_its_golden(line, golden, differs) -> None:
    difference, _ = golden_gate.judge(line, golden)
    assert (difference is not None) == differs
