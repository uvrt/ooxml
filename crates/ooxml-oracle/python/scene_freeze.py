#!/usr/bin/env python3
"""Freeze the scenes ooxml-common's golden harness builds in code (test support; lane P3c).

ooxml-common's scene goldens (``tests/test_goldens.py``) draw two kinds of input: resolved
slides frozen as JSON under ``tests/fixtures/scenes/`` -- which the Rust test reads where
they are -- and the scenes ``tests/handmade_scene.py`` builds in Python. This writes the
second kind in the first's form (``goldens.to_data``: each dataclass a dict led by its class
name, exact numbers), so ``cargo test``'s ``tests/integration/scene_goldens.rs`` can draw them where
Python is not installed::

    PYTHONPATH=python/ooxml-common/src:python/ooxml-common/tests \\
        python crates/ooxml-oracle/python/scene_freeze.py crates/ooxml-oracle/tests/data/handmade-scenes.json

Regenerate it whenever ``handmade_scene.py`` changes. Python's behaviour is not changed:
this only calls the harness' own functions.
"""

from __future__ import annotations

import json
import sys
from pathlib import Path

import goldens
import handmade_scene


def main() -> None:
    out = {
        name: {"width_emu": handmade_scene.WIDTH, "height_emu": handmade_scene.HEIGHT,
               "elements": goldens.to_data(build())}
        for name, build in handmade_scene.SCENES.items()
    }
    Path(sys.argv[1]).write_text(json.dumps(out, indent=1, ensure_ascii=False) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
