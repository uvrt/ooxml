"""docx-agent's checks served by the Rust core (lane F, 0.1.114): :mod:`.check` answers a
check of some ranges from the document's Rust owner (``ooxml_core``); :mod:`.mirror` keeps
docx2svg's model of the body and makes each block's measuring input (:mod:`.exporter`) and
record (:mod:`.records`), the change sets the owner is handed.  ``DOCX_AGENT_RUST=0`` turns
it off."""
