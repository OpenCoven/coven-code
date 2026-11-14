//! Raw image writes bypass frame cells and must validate their own payload.
use claurst_core::types::ImageSource;

const COMMAND: &str = "\x1b]52;c;SU1BR0UtQ0FOQVJZ\x07";
const PNG: &str =
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAACklEQVR4nGMAAQAABQABDQottAAAAABJRU5ErkJggg==";

#[test]
fn kitty_image_payload_is_validated_before_raw_output() {
    if let Ok(case) = std::env::var("COVEN_TEST_IMAGE_WORKER") {
        let source = ImageSource {
            source_type: "base64".into(),
            media_type: Some("image/png".into()),
            data: Some(if case == "valid" {
                PNG.into()
            } else if case == "whitespace" {
                format!(" \n{PNG}\n ")
            } else {
                format!("AAAA\x1b\\{COMMAND}")
            }),
            url: None,
        };
        let fallback = claurst_tui::kitty_image::render_image(&source);
        assert_eq!(fallback.is_none(), case != "malicious");
        return;
    }
    for case in ["malicious", "valid", "whitespace"] {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "kitty_image_payload_is_validated_before_raw_output",
                "--nocapture",
            ])
            .env("COVEN_TEST_IMAGE_WORKER", case)
            .env("TERM", "xterm-kitty")
            .env_remove("TERM_PROGRAM")
            .output()
            .unwrap();
        assert!(
            !output
                .stdout
                .windows(COMMAND.len())
                .any(|window| window == COMMAND.as_bytes()),
            "unvalidated image payload emitted a raw OSC 52 command"
        );
        assert!(
            output.status.success(),
            "{case} worker failed: {:?}",
            output.status
        );
        let marker = b"\x1b_Ga=T,f=100,m=0,q=2,C=1;";
        if case != "malicious" {
            let expected = format!("\x1b_Ga=T,f=100,m=0,q=2,C=1;{PNG}\x1b\\\r\n");
            assert!(
                output
                    .stdout
                    .windows(expected.len())
                    .any(|window| window == expected.as_bytes()),
                "valid image bytes and framing stay unchanged"
            );
        }
        assert_eq!(
            output
                .stdout
                .windows(marker.len())
                .any(|window| window == marker),
            case != "malicious"
        );
    }
}
