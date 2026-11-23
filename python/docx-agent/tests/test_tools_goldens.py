"""Golden transcripts: the trial's Word tasks w1-w6 done with the tools only, replayed.

Each transcript (``goldens/transcripts``) is the tool calls that do one task of the end-to-end
trial -- the calls an agent with the tools makes, no Python -- with every call's normalised
result.  Replaying one on the task's inputs must give the same results, an output whose parts
are byte for byte the ones recorded (the session clock is fixed), and an output that passes the
trial's own check (``goldens/trial/_grading/<task>/check.py``, recovered unchanged).  So
"the tools can do every trial task without code" is a regression test.

The output is compared by its parts (names and uncompressed bytes, ``goldens_replay``'s
``package_digest``), not by the zip's bytes: the same parts deflated by zlib (the Python save)
and zlib-rs (the Rust session's) differ in compressed size, and the two write some parts in a
different order; Word sees neither.  For the
same reason ``save_document``'s ``size`` stays in its reply and in ``data_keys`` but out of
``data_sha`` (``goldens_replay.ENCODING_FIELDS``).

The parts byte for byte hold where Office's faces are installed (the Mac the transcripts were recorded
on): the layout -- page counts, reflow, coverage, field page numbers -- is in the results and
in the saved file.  Every other runner, CI's included, replays them too and compares what does
not depend on the layout: each result's ok, summary, ids, warnings, the schema of its data
(``data_keys``) and the digest of its data less what the layout measures (``data_sha_core``),
as ``goldens_replay`` splits them.  So a result that gains or loses a key fails on every
runner, not only on a Mac with Office.  ``tools/refresh_goldens.py`` re-records them (CONTRIBUTING.md,
"Golden transcripts").

``batch-30-edits`` is one ``batch`` call writing 30 tracked insertions and 10 comments
anchored on them by refs.
"""

from __future__ import annotations

import functools
import json
import os
import subprocess
import sys
from pathlib import Path

import pytest

pytest.importorskip("ooxml_edit.tools.shared", reason="needs ooxml-edit 0.4 (the shared tools)")

import goldens_replay  # noqa: E402
from conftest import FIXTURE_DIR, OFFICE_FACES_ABSENT

HERE = Path(__file__).parent
GOLDENS = HERE / "goldens"
TRANSCRIPTS = sorted((GOLDENS / "transcripts").glob("*.json"))


def inputs_of(transcript: dict) -> Path:
    if "fixture" in transcript:
        return (FIXTURE_DIR / transcript["fixture"]).parent
    return GOLDENS / "trial" / "_inputs" / transcript["task"] / "input"


@functools.lru_cache(maxsize=None)
def replayed(path: Path) -> tuple[dict, list, list]:
    """The transcript, its replayed results (normalised) and outputs: one replay for both
    tests below."""
    transcript = json.loads(path.read_text(encoding="utf-8"))
    results, outputs = goldens_replay.replay(transcript, inputs_of(transcript))
    return transcript, results, outputs


def passes_its_check(transcript: dict, output, tmp_path: Path) -> None:
    check = GOLDENS / "trial" / "_grading" / transcript["task"] / "check.py"
    if not check.exists():
        return
    target = tmp_path / output.name
    target.write_bytes(output.data)
    proc = subprocess.run([sys.executable, str(check), str(target)], capture_output=True, text=True,
                          encoding="utf-8", env={**os.environ, "PYTHONUTF8": "1"}, timeout=600)
    assert proc.stdout, proc.stderr
    report = json.loads(proc.stdout)
    failed = [r for r in report["results"] if not r["ok"]]
    assert not failed and report["passed"] == report["total"], failed


@pytest.mark.parametrize("path", TRANSCRIPTS, ids=[p.stem for p in TRANSCRIPTS])
def test_a_golden_transcript_replays_to_the_same_results_and_bytes(path, tmp_path):
    if OFFICE_FACES_ABSENT:
        # The transcripts' layout facts (pages, reflow, coverage, field page numbers, the page
        # count saved in app.xml) were recorded on macOS with Office's faces; a runner
        # without them lays out with substitutes, or stops.  The core replay below holds the
        # rest there.
        pytest.skip("Office's faces are not all installed here: the core replay compares what "
                    "does not depend on them")
    transcript, results, outputs = replayed(path)
    mismatches = [(index, call["tool"], call["expect"], got)
                  for index, (call, got) in enumerate(zip(transcript["calls"], results))
                  if got != call["expect"]]
    assert mismatches == [], mismatches[:2]
    assert outputs, "the transcript saved nothing"
    output = outputs[-1]
    assert output.name == transcript["output"]["name"]
    # The parts Word reads, not the zip's compressed bytes (the module docstring says why).
    assert goldens_replay.package_digest(output.data) == transcript["output"]["parts_sha256"]
    assert output.validate["new"] == []
    passes_its_check(transcript, output, tmp_path)


@pytest.mark.any_faces
@pytest.mark.parametrize("path", TRANSCRIPTS, ids=[p.stem for p in TRANSCRIPTS])
def test_a_golden_transcript_replays_to_the_same_schema_and_core_anywhere(path, tmp_path):
    """Needs no Office face: what the layout measures (``goldens_replay``'s split) is left
    out, everything else -- each result's data keys included -- must be as recorded."""
    transcript, results, outputs = replayed(path)
    mismatches = goldens_replay.core_mismatches(transcript, results)
    assert mismatches == [], mismatches[:2]
    assert outputs, "the transcript saved nothing"
    output = outputs[-1]
    assert output.name == transcript["output"]["name"]
    assert output.validate["new"] == []
    if not goldens_replay.writes_layout(transcript):  # else its page numbers are this runner's
        passes_its_check(transcript, output, tmp_path)


def test_the_trial_tasks_are_done_by_tools_alone():
    names = {json.loads(p.read_text(encoding="utf-8"))["task"] for p in TRANSCRIPTS}
    assert {f"w{n}" for n in range(1, 7)} <= {name[:2] for name in names}
    for path in TRANSCRIPTS:
        for call in json.loads(path.read_text(encoding="utf-8"))["calls"]:
            assert call["expect"]["ok"], (path.stem, call["tool"])


def test_one_batch_writes_30_tracked_edits_and_10_comments_as_one_step():
    from ooxml_edit.tools import Toolbox

    from docx_agent.tools import FORMAT, GROUPS, TOOLS
    import word_docs

    path = GOLDENS / "transcripts" / "batch-30-edits.json"
    transcript = json.loads(path.read_text(encoding="utf-8"))
    _, outputs = goldens_replay.run(transcript, inputs_of(transcript))
    data = outputs[-1].data
    root = word_docs.tree(data)
    mine = [r for r in root.iter() if r.get(f"{{{word_docs.W}}}author") == "Claude"
            and r.tag in (f"{{{word_docs.W}}}ins", f"{{{word_docs.W}}}del", f"{{{word_docs.W}}}rPrChange",
                          f"{{{word_docs.W}}}pPrChange")]
    assert mine and all(r.tag == f"{{{word_docs.W}}}ins" for r in mine)
    comments = word_docs.tree(data, "word/comments.xml").findall(f"{{{word_docs.W}}}comment")
    assert len(comments) == 10
    with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS, workers=0) as box:
        session = box.session()
        doc = session.open(data, "batch.docx")
        listed = word_docs.call(box, session, "word_changes", doc=doc, action="list", author="Claude")
        assert (listed.total if listed.total is not None else len(listed.data)) == 30, listed.to_json()
        assert word_docs.validation(box, session, doc)["problems"] == []
    batch = next(call for call in transcript["calls"] if call["tool"] == "batch")
    assert len(batch["expect"]["created"]) == 10 and len(batch["expect"]["refs"]) == 30
