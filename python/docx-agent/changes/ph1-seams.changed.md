Architecture review phase 1 (lane ph1-seams): the owner of Word documents drops the view
protocol's refusing stubs (`open`, `follow`, `bridge`, `reader`) and the "not served by the
Rust session" check: every Word tool is the session's. `docx_agent.rust.served.Hosting`
lists every exported tool (`tools`) instead of the served ones, and `laid` no longer holds
`render` (its page engine is handed to it by `reads.serve_render`).
`docx_agent.rust.reads.last_check` is gone (nothing read it).
