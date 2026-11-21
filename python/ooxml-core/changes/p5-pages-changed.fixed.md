A page engine given each version as its package (`PageEngine::lay_out`, a session's
path) now finds the parts that version changed from the one given before, by identity,
else by bytes (`ooxml_opc::Package::changed_parts`). Before, it kept the list from the last version given
as bytes or touched parts: after a session's edit of a part beside the document part (a
diagram's, a chart's, the relationships), the pages the edit did not reach were said again
with the warnings of the version before, and a version laid out whole for its styles did
not name the styles part as its cause.
