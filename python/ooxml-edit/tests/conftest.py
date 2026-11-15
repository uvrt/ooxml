from __future__ import annotations

import os

import pytest

from ooxml_edit.opc import OpcPackage
from ooxml_edit.xml import register_namespaces

import synthetic

# 0.117.0 (lane G): every kept serialization of an edited part is compared with the tree
# written out again, at every use (ooxml_edit.opc.OpcPackage.keep_serialized).
os.environ.setdefault("OOXML_EDIT_VERIFY_SERIALIZED", "1")

# The test vocabulary, registered once the way a format layer registers its own.
register_namespaces({"tst": synthetic.NS})


@pytest.fixture
def data() -> bytes:
    """The synthetic package's bytes, as written by an ordinary zip writer."""
    return synthetic.outer_package()


@pytest.fixture
def package(data) -> OpcPackage:
    return OpcPackage.open(data)
