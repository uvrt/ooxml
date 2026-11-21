"""A stand-in for the Rust core (``ooxml_core``) in the gates' reference processes: importing
it fails as it does where the Rust core is not installed (crates/ooxml-oracle/python/
reference_guard.py)."""

raise ModuleNotFoundError("No module named 'ooxml_core' (kept out of the gates' reference: "
                          "crates/ooxml-oracle/python/reference_guard.py)", name="ooxml_core")
