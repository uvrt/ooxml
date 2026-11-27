"""On the free-threaded build (3.15t: R2 caveat 4, design phase 3 section 7.5), the facade
(``ooxml_edit.tools.Toolbox``) and docx-agent's tools import with the GIL off: nothing they load
turns it back on.  A fresh interpreter, so that nothing the suite imported first decides it."""

from __future__ import annotations

import subprocess
import sys
import sysconfig

import pytest

IMPORTS = ("import sys; from ooxml_edit.tools import Toolbox; import docx_agent.tools; "
           "sys.exit('the GIL is enabled' if sys._is_gil_enabled() else 0)")


@pytest.mark.skipif(not sysconfig.get_config_var("Py_GIL_DISABLED"), reason="a free-threaded build of Python")
def test_the_facade_and_docx_agents_tools_import_with_the_gil_off():
    done = subprocess.run([sys.executable, "-W", "error::RuntimeWarning", "-c", IMPORTS],
                          capture_output=True, text=True, timeout=600)
    assert done.returncode == 0, done.stdout + done.stderr
