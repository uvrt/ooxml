"""The host goldens (host_goldens.py): every recorded case replayed through today's Python host
(docx-agent's Toolbox) gives what was recorded -- 0 differences but the known ones -- and the
format's own pieces (normalisation, part digests, known differences) do what they say."""

from __future__ import annotations

import functools
import io
import sys
import zipfile
from pathlib import Path

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

import host_goldens  # noqa: E402

STREAMS = host_goldens.streams()


@functools.lru_cache(maxsize=4)
def _cases(stream: str) -> dict[str, list[dict]]:
    return host_goldens.cases_of(host_goldens.read_stream(STREAMS[stream]))

CASES = [(name, case) for name, path in STREAMS.items()
         for case in host_goldens.cases_of(host_goldens.read_stream(path))]


def _package(parts: dict[str, bytes], level: int = zipfile.ZIP_DEFLATED) -> bytes:
    buffer = io.BytesIO()
    with zipfile.ZipFile(buffer, "w", level) as package:
        for name, body in parts.items():
            package.writestr(name, body)
    return buffer.getvalue()


def test_a_package_is_its_parts_whatever_the_zip() -> None:
    parts = {"[Content_Types].xml": b"<Types/>", "word/document.xml": b"<w:document/>"}
    deflated, stored = _package(parts), _package(dict(reversed(parts.items())), zipfile.ZIP_STORED)
    assert deflated != stored
    assert host_goldens.part_digests(deflated) == host_goldens.part_digests(stored)
    inner = host_goldens.part_digests(_package({"x.xlsx": deflated}))["x.xlsx"]
    assert inner == host_goldens.part_digests(_package({"x.xlsx": stored}))["x.xlsx"]
    assert inner.startswith("package:")


def test_timing_is_out_of_a_result_at_any_depth() -> None:
    data = {"timing": {"layout": 1.5}, "ops": [{"data": {"timing": {"x": 2}, "kept": 1}}]}
    assert host_goldens._without_timing(data) == {"ops": [{"data": {"kept": 1}}]}


def test_differences_name_the_paths_and_known_ones_are_admitted_by_field() -> None:
    want = [{"case": "c", "at": 0, "event": "case"},
            {"case": "c", "at": 1, "event": "call", "tool": "t", "arguments": {"doc": "d1", "x": ""},
             "result": {"ok": False, "error": {"code": "invalid_arguments"}, "warnings": []}}]
    have = [want[0], {**want[1], "result": {"ok": True, "error": None, "warnings": ["read '' as none"]}}]
    found = host_goldens.differences(want, have)
    assert [d["at"] for d in found] == [1]
    assert found[0]["paths"] == ["result.error", "result.error.code", "result.ok", "result.warnings",
                                 "result.warnings[0]"]
    entry = {"stream": "docx/*", "tool": "t", "arguments": {"x": ""}, "rule": 1, "cause": "rule 1",
             "fields": ["result.ok", "result.error", "result.warnings"]}
    left, used = host_goldens.unlisted("docx/tests/x", "c", found, [entry])
    assert left == [] and used == {0}
    narrow = {**entry, "fields": ["result.warnings"]}
    left, _ = host_goldens.unlisted("docx/tests/x", "c", found, [narrow])
    assert left[0]["paths"] == ["result.error", "result.error.code", "result.ok"]
    other = {**entry, "arguments": {"x": "y"}}
    assert host_goldens.unlisted("docx/tests/x", "c", found, [other])[0] == found


def test_blessing_named_cases_rewrites_only_their_lines() -> None:
    """``bless --case``: the named cases' lines replaced where they stood (a new one at the
    end), every other line kept byte for byte -- even one not as ``canonical`` writes it."""
    other = '{"case": "b",  "event": "case"}\n'
    stream = '{"case":"a","event":"case","v":1}\n' + other + '{"case":"a","event":"call","v":1}\n'
    lines = [{"case": "a", "event": "case", "v": 2}, {"case": "c", "event": "case"}]
    out = host_goldens.spliced(stream, lines).splitlines(keepends=True)
    assert out[0] == host_goldens.canonical(lines[0]) + "\n"
    assert out[1] == other
    assert out[2] == host_goldens.canonical(lines[1]) + "\n"
    assert len(out) == 3


def test_blessing_a_case_not_recorded_is_refused(tmp_path, capsys) -> None:
    (tmp_path / "docx" / "tests").mkdir(parents=True)
    (tmp_path / "docx" / "tests" / "test_x.jsonl").write_text(
        '{"case": "test_x::t#1", "event": "case", "at": 0, "source": "Python host at abc"}\n', encoding="utf-8")
    code = host_goldens.bless(tmp_path, tmp_path / "host", cases=["test_x::u#1"])
    assert code == 1 and "not recorded: test_x::u#1" in capsys.readouterr().out
    assert not (tmp_path / "host").exists()


def test_a_known_difference_needs_rule_1_a_cause_and_fields(tmp_path) -> None:
    pytest.importorskip("tomllib")
    path = tmp_path / "known.toml"
    path.write_text('[[difference]]\nstream = "*"\nrule = 4\ncause = "x"\nfields = ["result"]\n')
    with pytest.raises(ValueError, match="rule"):
        host_goldens.known_entries(path)
    path.write_text('[[difference]]\nstream = "*"\nrule = 1\nfields = ["result"]\n')
    with pytest.raises(ValueError, match="cause"):
        host_goldens.known_entries(path)
    path.write_text('[[difference]]\nstream = "*"\nrule = 1\ncause = "x"\nfields = ["result"]\n')
    assert len(host_goldens.known_entries(path)) == 1


def test_the_known_differences_file_reads() -> None:
    host_goldens.known_entries()


def test_the_streams_are_recorded_from_the_python_host() -> None:
    assert STREAMS, "no host goldens"
    for name, path in STREAMS.items():
        heads = [line for line in host_goldens.read_stream(path) if line["event"] == "case"]
        assert heads, name
        assert all(line["source"].startswith("Python host at ") for line in heads), name


def test_every_input_is_there_with_its_bytes() -> None:
    for path in STREAMS.values():
        for line in host_goldens.read_stream(path):
            if isinstance(line.get("input"), dict):
                host_goldens.input_bytes(line["input"])


@pytest.mark.parametrize("stream,case", CASES, ids=[f"{name}::{case}" for name, case in CASES])
def test_a_recorded_case_replays_with_no_unlisted_difference(stream: str, case: str) -> None:
    pytest.importorskip("docx_agent.tools")
    if stream.startswith("mixed/"):
        # A workbook the Python host holds: xlsx-agent's suite replays these (its tests'
        # test_host_goldens_mixed.py), where xlsx-agent is installed.
        pytest.importorskip("xlsx_agent.tools")
    events = _cases(stream)[case]
    found = host_goldens.differences(host_goldens.expected_of(events), host_goldens.replay_case(events))
    left, _used = host_goldens.unlisted(stream, case, found, host_goldens.known_entries())
    assert left == [], left[:3]
