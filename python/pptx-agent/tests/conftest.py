import os
from pathlib import Path

import pytest

# 0.117.0 (lane G): every kept serialization of an edited part is compared with the tree
# written out again, at every use (ooxml_edit.opc.OpcPackage.keep_serialized).
os.environ.setdefault("OOXML_EDIT_VERIFY_SERIALIZED", "1")

FIXTURE_DIR = Path(__file__).parent / "fixtures"


def fixture_paths() -> list[Path]:
    return sorted(FIXTURE_DIR.glob("*.pptx"))


@pytest.fixture(params=fixture_paths(), ids=lambda path: path.stem)
def pptx_path(request) -> Path:
    """Every fixture in turn -- the corpus a test must hold for, not just one happy deck."""
    return request.param


@pytest.fixture(scope="session")
def product_page() -> Path:
    return FIXTURE_DIR / "real-product-page.pptx"


@pytest.fixture(scope="session")
def financial_report() -> Path:
    """The deck with duplicate ``cNvPr@id`` values -- slides 2, 3 and 4 each have a pair."""
    return FIXTURE_DIR / "real-financial-report.pptx"
