"""The Python shim's side of a document laid out by the Rust owner: moved to docx-agent
(:mod:`docx_agent.rust.mirror`), which serves its checks from it."""
from docx_agent.rust.mirror import *  # noqa: F401,F403
from docx_agent.rust.mirror import Mirror, Unit, Unsupported, key_of  # noqa: F401
