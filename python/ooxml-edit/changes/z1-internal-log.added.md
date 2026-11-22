**Internal failures logged at ERROR, in one place (lane Z1).** Every call whose reply is an
`internal` failure -- a library's unexpected exception, or a panic inside the Rust session,
which the session now answers itself as "the call failed inside the library" -- is logged to
the `ooxml_edit.tools` logger at ERROR when the call is recorded (`logs.log_internal`): the
tool, its documents, its version, the arguments' digest, the reply's message and its
`details.trace` (a panic's names the tool and what it said). Never the arguments.
