"""The gates' reference keeps the Rust core out (crates/ooxml-oracle/python/reference_guard.py):
docx-agent serves through ``ooxml_core`` wherever it is loaded, ``DOCX_AGENT_RUST=0`` or not, so
the edits and reads harnesses keep it out of their process and of every worker process the
reference starts, here where ooxml-core is installed as much as where it is not."""

from __future__ import annotations

import json
import os
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
HARNESS = ROOT / "crates" / "ooxml-oracle" / "python"
DOCUMENT = Path(__file__).resolve().parent / "fixtures" / "generated" / "lists-and-styles.docx"
STAND_IN = "kept out of the gates' reference"

#: A guarded process, a spawn worker of it and a subprocess of it each importing ooxml_core.
WORKERS = r"""
import importlib, json, multiprocessing, subprocess, sys

if __name__ == "__main__":
    sys.path.insert(0, sys.argv[1])
    from reference_guard import guard

    guard()
    found = {}
    try:
        importlib.import_module("ooxml_core")
    except ImportError as error:
        found["process"] = str(error)
    with multiprocessing.get_context("spawn").Pool(1) as pool:
        try:
            pool.apply(importlib.import_module, ("ooxml_core",))
        except ImportError as error:
            found["spawn worker"] = str(error)
    done = subprocess.run([sys.executable, "-c", "import ooxml_core"], capture_output=True, text=True)
    found["subprocess"] = done.stderr.strip().splitlines()[-1] if done.returncode else "imported"
    print(json.dumps(found))
"""

#: The edits harness's reference (docx-agent, Rust off) on one document in modes that call the
#: tools through a toolbox with workers; the Rust side needs the oracle binary, so it is left out.
REFERENCE = r"""
import json, sys

if __name__ == "__main__":
    sys.path.insert(0, sys.argv[1])
    import word_edits_diff as harness

    harness.run_rust = lambda *args, **kwargs: {}
    harness._compare = lambda want, have: (have, None)
    for flags in ([], ["--text"], ["--structure"]):
        harness.main([*flags, "--limit", "2", "--rust", "unused", sys.argv[2]])
    from docx_agent.rust.served import hosting

    print(json.dumps({"core": sorted(m for m in sys.modules if m.split(".")[0] == "ooxml_core"),
                      "served": sorted(hosting().served)}))
"""


def _run(script: str, tmp_path: Path, *args: str) -> dict:
    path = tmp_path / "reference_run.py"
    path.write_text(script)
    env = {**os.environ, "DOCX_AGENT_RUST": "0", "PYTHONWARNINGS": "ignore", "STRUCTURE_PER_TOOL": "1"}
    done = subprocess.run([sys.executable, str(path), str(HARNESS), *args], capture_output=True, text=True, env=env,
                          cwd=ROOT, timeout=300, check=False)
    assert done.returncode in (0, 1), done.stdout[-3000:] + done.stderr[-3000:]
    return json.loads(done.stdout.splitlines()[-1])


def test_the_rust_core_is_kept_out_of_the_process_and_its_workers(tmp_path: Path) -> None:
    found = _run(WORKERS, tmp_path)
    assert set(found) == {"process", "spawn worker", "subprocess"}, found
    for where, error in found.items():
        assert STAND_IN in error, (where, error)


def test_the_reference_runs_without_the_rust_core(tmp_path: Path) -> None:
    found = _run(REFERENCE, tmp_path, str(DOCUMENT))
    assert found == {"core": [], "served": []}, found
