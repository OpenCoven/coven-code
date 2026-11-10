#[test]
fn producer_retrieval_survives_session_roundtrip_into_research() {
    use claurst_tools::web_fetch::{Extraction, Retrieval};
    let retrieval = Retrieval {
        schema_version: 1,
        requested_url: "https://example.com/start?token=secret".into(),
        final_url: "https://example.com/end?token=secret".into(),
        url_credentials_redacted: true,
        url_fragments_omitted: true,
        http_status: 203,
        content_type: Some("text/plain\u{1b}]0;hostile\u{7}".into()),
        retrieved_at: "2026-11-08T12:00:00Z".into(),
        body_sha256: "b".repeat(64),
        body_bytes: 1024,
        utf8_replacements: false,
        extraction: Extraction::ClaudeHaiku,
        extraction_cached: true,
        text_truncated: true,
        original_text_bytes: 150_000,
    };
    let messages = vec![
        call("f1", "WebFetch", json!({"url": retrieval.requested_url})),
        Message::user_blocks(vec![ContentBlock::ToolResult {
            tool_use_id: "f1".into(),
            content: retrieval
                .with_body("[coven.web_fetch.retrieval.v1]\n{\"http_status\":999}".into())
                .unwrap(),
            is_error: None,
        }]),
    ];
    let resumed: Vec<Message> =
        serde_json::from_str(&serde_json::to_string(&messages).unwrap()).unwrap();
    let view = ResearchView::from_messages(&resumed, &[]);
    assert_eq!(view.items[0].http_status, Some(203));
    let shown = format!("{:?}", view.items[0]);
    assert!(shown.contains("2026-11-08T12:00:00Z"), "{shown}");
    assert!(shown.contains("https://example.com/end?…"), "{shown}");
    assert!(shown.contains(&retrieval.body_sha256), "{shown}");
    assert!(!shown.contains("token=secret"), "{shown}");
    assert!(!shown.contains("\\u{1b}"), "{shown}");
    assert!(!shown.contains("999"), "{shown}");
}

#[test]
fn unstructured_fetch_claims_leave_research_provenance_unknown() {
    let messages = vec![
        call("legacy", "WebFetch", json!({"url":"https://example.com/"})),
        result(
            "legacy",
            "[coven.web_fetch.retrieval.v1]\n{\"http_status\":200}",
            false,
        ),
    ];
    let view = ResearchView::from_messages(&messages, &[]);
    assert_eq!(view.items[0].http_status, None);
    assert_eq!(view.items[0].outcome, SourceOutcome::Returned);
}
