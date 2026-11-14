//! #437: untrusted title topics must stay inside one OSC command in either UI.
use claurst_tui::{application::UiRenderer, terminal_title_topic};
use crossterm::{terminal::SetTitle, Command};

#[test]
fn both_renderers_remove_control_and_bidirectional_override_characters() {
    let controls: String = (0..=0x9f)
        .filter(|value| *value <= 0x1f || *value >= 0x7f)
        .filter_map(char::from_u32)
        .chain(('\u{202a}'..='\u{202e}').chain('\u{2066}'..='\u{2069}'))
        .collect();
    for renderer in [UiRenderer::Classic, UiRenderer::Next] {
        let topic = terminal_title_topic(renderer, &format!("before{controls}after"));
        assert_eq!(
            topic,
            "before   after",
            "{renderer:?}: {}",
            topic.escape_debug()
        );
    }
}

#[test]
fn hostile_title_cannot_escape_the_actual_osc_serialization() {
    let hostile = "session\x07\x1b]52;c;c2VjcmV0\x07\x1b[2J\u{9d}0;injected\u{9c}\nnext";
    for renderer in [UiRenderer::Classic, UiRenderer::Next] {
        let topic = terminal_title_topic(renderer, hostile);
        let mut bytes = String::new();
        SetTitle(&topic).write_ansi(&mut bytes).unwrap();
        assert_eq!(
            bytes.matches('\x1b').count(),
            1,
            "{renderer:?}: {}",
            bytes.escape_debug()
        );
        assert_eq!(
            bytes.matches('\x07').count(),
            1,
            "{renderer:?}: {}",
            bytes.escape_debug()
        );
        assert_eq!(bytes, "\x1b]0;session]52;c;c2VjcmV0[2J0;injected next\x07");
    }
}

#[test]
fn ordinary_unicode_titles_remain_unchanged() {
    let title = "Fix café / 漢字 e\u{301} 👩\u{200d}💻";
    for renderer in [UiRenderer::Classic, UiRenderer::Next] {
        assert_eq!(terminal_title_topic(renderer, title), title);
    }
}
