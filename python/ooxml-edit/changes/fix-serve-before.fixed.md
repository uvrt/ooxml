- A changing call an owner serves side by side no longer fails with an internal
  `TypeError: 'None' is not an instance of 'Version'` when other writers make more steps than the
  owner keeps for undo before its reply is built: the reply's `touched` and `changed_parts`, and
  the change the step counts, are read from the version the step went from as the owner handed it
  (`Served.before`, held), never looked up by its number again. Before, the step was kept but the
  call reported the internal error, and its counted change could have no parts.
