"""The module ``ooxml_core`` itself (internal: decision D6, used by ooxml-edit's facade and
docx2svg): its tool specs are the registry's exports, and on a free-threaded build it keeps the
interpreter lock off.  The Rust host's sessions are tested through the facade (docx-agent's
suite, the host goldens' facade replay) and in ``crates/ooxml-host``."""
from __future__ import annotations

import json
import sys
import sysconfig

import ooxml_core
import pytest


def test_tool_specs_are_the_exports():
    specs = json.loads(ooxml_core.tool_specs("docx"))
    # the registry's tools in registration order (ooxml-session's tests/integration/registry.rs holds each
    # to its golden export): describe first, read_batch over the others last, each once
    names = [s["name"] for s in specs]
    assert names[0] == "describe" and names[-1] == "read_batch" and len(set(names)) == len(names)
    assert {"word_read", "word_set_text"} <= set(names)
    assert json.loads(ooxml_core.tool_specs("pptx")) == []
    with pytest.raises(ValueError):
        ooxml_core.tool_specs("doc")


def test_the_module_keeps_a_free_threaded_build_free():
    if not sysconfig.get_config_var("Py_GIL_DISABLED"):
        pytest.skip("not a free-threaded build")
    assert not sys._is_gil_enabled()


def test_the_session_api_of_before_is_gone():
    # Phase 3 E2: the process's own sessions went with the Python host of Word documents
    for name in ("Session", "CallOptions", "CallDocuments", "CallLayout", "CallWriting", "CallContext",
                 "CancelToken", "Limits", "StepGroup", "GroupMark"):
        assert not hasattr(ooxml_core, name), name
