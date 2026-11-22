`docx2svg.rust` (lane CL1): a version whose body holds a paragraph the Rust layout draws as a
placeholder is laid out in Rust (it fell back to the Python layout before): its
`placeholders` and its coverage (`partial`, the placeholder's block not among
`blocks_laid_out`) come from Rust. With `paginate.CARRY_ON` off the Rust layout is not used
(it always carries on): the fallback is counted as `layout stops`.
