//! The Markdown parser (`insert_markdown`'s and `word_set_text`'s reading): no input
//! panics or loops, and every token's nesting balances.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    let (tokens, _) = ooxml_markdown::parse(&text);
    let depth = tokens.iter().fold(0i64, |depth, t| {
        let depth = depth + i64::from(t.nesting);
        assert!(depth >= 0, "a token closes what is not open");
        depth
    });
    assert_eq!(depth, 0, "every open token is closed");
    let _ = ooxml_markdown::styled::parse(&text);
});
