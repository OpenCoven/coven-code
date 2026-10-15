use claurst_core::strip_jsonc_comments;

#[test]
fn jsonc_comments_after_backslash_terminated_strings() {
    for count in 1..=4 {
        let text = format!("directory{}", "\\".repeat(count));
        let quoted = serde_json::to_string(&text).unwrap();
        for comment in ["// trailing comment\n", "/* block\ncomment */"] {
            let input = format!("[{quoted}, {comment}\"next\"]");
            let stripped = strip_jsonc_comments(&input);
            let values: Vec<String> = serde_json::from_str(&stripped).unwrap_or_else(|error| {
                panic!("count={count}, comment={comment:?}: {error}; {stripped:?}")
            });
            assert_eq!(values, [text.clone(), "next".into()]);
            assert_eq!(input.matches('\n').count(), stripped.matches('\n').count());
        }
    }
}

#[test]
fn jsonc_preserves_comment_markers_and_escaped_quotes_in_strings() {
    for text in [
        "https://example.test/a/*literal*/",
        "quote: \" // still string",
        "slash and quote: \\\" /* still string */",
        "two slashes and quote: \\\\\" // still string",
        "会話 /* literal */ \\",
    ] {
        let input = serde_json::to_string(text).unwrap();
        assert_eq!(strip_jsonc_comments(&input), input);
        let commented = format!("/* before */ {input} // after\n");
        let parsed: String = serde_json::from_str(&strip_jsonc_comments(&commented)).unwrap();
        assert_eq!(parsed, text);
    }
}
