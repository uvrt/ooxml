- Chart edits and the DrawingML/chart views insert children by the child orders generated from the ECMA-376 XML
  schemas (`ooxml_schema::orders::local`), no longer by ooxml-chart's hand-kept copies (lane ph2-types2,
  architecture review R1 #3). The orders are the same; an axis title is now placed by its axis' whole sequence
  rather than its first seven steps, which puts it in the same place. The 5th edition Transitional `pml.xsd` is
  vendored for `p:sp`. No reply changes.
