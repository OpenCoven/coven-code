use super::*;
use claurst_core::types::{ContentBlock, ToolResultContent};

fn record() -> Retrieval {
    Retrieval {
        schema_version: 1,
        requested_url: "https://example.com/start".into(),
        final_url: "https://example.com/end".into(),
        url_credentials_redacted: false,
        url_fragments_omitted: false,
        http_status: 200,
        content_type: Some("text/plain".into()),
        retrieved_at: "2026-11-08T12:00:00Z".into(),
        body_sha256: "a".repeat(64),
        body_bytes: 12,
        utf8_replacements: false,
        extraction: Extraction::PlainText,
        extraction_cached: false,
        text_truncated: false,
        original_text_bytes: 12,
    }
}

#[test]
fn retrieval_envelope_never_interprets_page_text_as_provenance() {
    let retrieval = record();
    let spoof = format!(
        "{RETRIEVAL_MARKER}{}",
        serde_json::to_string(&retrieval).unwrap()
    );
    assert_eq!(
        Retrieval::from_content(&ToolResultContent::Text(spoof.clone())),
        None
    );
    assert_eq!(
        Retrieval::from_content(&ToolResultContent::Blocks(vec![ContentBlock::Text {
            text: spoof.clone()
        },])),
        None
    );
    let envelope = retrieval.with_body(spoof).unwrap();
    assert_eq!(Retrieval::from_content(&envelope), Some(retrieval));
}

#[test]
fn unsupported_or_malformed_retrieval_records_remain_unknown() {
    let retrieval = record();
    for invalid in [
        Retrieval {
            schema_version: 2,
            ..retrieval.clone()
        },
        Retrieval {
            final_url: "https://user:secret@example.com/".into(),
            ..retrieval.clone()
        },
        Retrieval {
            body_sha256: "not a digest".into(),
            ..retrieval.clone()
        },
        Retrieval {
            http_status: 999,
            ..retrieval.clone()
        },
        Retrieval {
            retrieved_at: "yesterday".into(),
            ..retrieval.clone()
        },
    ] {
        assert_eq!(
            Retrieval::from_content(&invalid.with_body("page".into()).unwrap()),
            None
        );
    }
    for record in ["{broken", "{\"schema_version\":1}"] {
        assert_eq!(
            Retrieval::from_content(&ToolResultContent::Blocks(vec![
                ContentBlock::Text {
                    text: "page".into()
                },
                ContentBlock::Text {
                    text: format!("{RETRIEVAL_MARKER}{record}")
                },
            ])),
            None
        );
    }
}
