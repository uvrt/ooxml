"""The gates' reference keeps the Rust core out (``reference_guard.py``): a guarded process, a
spawn worker of it and a subprocess of it each fail to import ``ooxml_core`` with the
stand-in's message, here where ooxml-core is installed.  (That the frozen reference's harness
runs without it needs the frozen reference installed: the gates' environment.)"""

from __future__ import annotations

import json
import os
import subprocess
import sys
from pathlib import Path

HARNESS = Path(__file__).resolve().parent.parent
STAND_IN = "kept out of the gates' reference"

#: A guarded process, a spawn worker of it and a subprocess of it, each importing ooxml_core.
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


def test_the_rust_core_is_kept_out_of_the_process_and_its_workers(tmp_path: Path) -> None:
    script = tmp_path / "guarded.py"
    script.write_text(WORKERS)
    env = {**os.environ, "PYTHONWARNINGS": "ignore"}
    done = subprocess.run([sys.executable, str(script), str(HARNESS)], capture_output=True, text=True, env=env,
                          timeout=300, check=False)
    assert done.returncode == 0, done.stdout[-3000:] + done.stderr[-3000:]
    found = json.loads(done.stdout.splitlines()[-1])
    assert set(found) == {"process", "spawn worker", "subprocess"}, found
    for where, error in found.items():
        assert STAND_IN in error, (where, error)
