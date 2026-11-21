"""The reads gate's facts cases load without pytest: the gates' reads environment has none, and
twice a made-up document taken from a test module (which imports pytest) crashed the facts
harness before its first case. Runs ``word_facts_cases.made_up()`` in a fresh interpreter
with pytest blocked."""

from __future__ import annotations

import os
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
ORACLE = ROOT / "crates" / "ooxml-oracle" / "python"

PROBE = """
import sys
sys.modules["pytest"] = None
import word_facts_cases
documents = word_facts_cases.made_up()
assert documents, "no made-up documents"
print(len(documents))
"""


def test_the_facts_cases_load_without_pytest():
    env = dict(os.environ)
    env["PYTHONPATH"] = os.pathsep.join([str(ORACLE), env.get("PYTHONPATH", "")])
    done = subprocess.run([sys.executable, "-c", PROBE], env=env, capture_output=True, text=True, timeout=300)
    assert done.returncode == 0, done.stderr[-2000:]
    assert int(done.stdout.split()[-1]) > 0
