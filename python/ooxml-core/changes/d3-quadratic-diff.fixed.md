Lane D3: setting a long text to a wholly different one no longer spins for minutes. The text
diff behind set_text, tracked replacements and paragraph pairing finds each longest match with
a suffix automaton (the same opcodes as Python's `difflib`, autojunk off), and a diff that
would examine more than four million elements keeps only the common start and end, with one
change between.
