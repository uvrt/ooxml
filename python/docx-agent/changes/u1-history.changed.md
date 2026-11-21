Lane U3: the Rust session numbers an owned document's states; docx-agent reads them
(`Version.state`) for its versions, its kept versions and the record of retired ids, and its
owner answers the tool layer's history from the session (`Session.numbers`, `numbered`,
`highest`, `continue_numbers`). Calls the session serves side by side no longer wait for one
another to be numbered.
