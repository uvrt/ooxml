- An edit whose core finds its own model and tree disagreeing is answered as an `internal` failure, a bug,
  never as a refusal of the call's input (lane ph2-types, architecture review R1 #7): XML the edit core makes
  itself that does not read back (a header or footer part, a chart's frame, a template's `docProps/app.xml`)
  was answered as `refused`. A control bound to a custom XML part whose relationship names a part the
  package lacks is refused as the missing part it is ("the custom XML part ... the control is bound to is
  missing"), no longer as an `internal` "no part". The session counts the calls of each tool answered as
  `internal` (`Session.internal()`, and `internal` in `stats()`).
