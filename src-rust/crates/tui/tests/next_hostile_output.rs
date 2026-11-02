//! T10 (#417), buffer-level safety and layout matrix for the Next renderer.
//!
//! One hostile corpus is placed in every untrusted field the Next renderer
//! shows: transcript text, tool calls and results, streaming text, status,
//! session title, tool rows, the approval prompt, notifications, the
//! composer draft and the Work, Research and Memory pages. Every page is
//! drawn across the #417 size matrix, every color mode and both glyph sets.
//! No cell may hold a control character (C0, DEL, C1) or a bidirectional
//! override, so clipboard (OSC 52), hyperlink (OSC 8), title, screen-clear
//! and cursor sequences can never reach the terminal from content. The
//! PTY-level counterpart is tmux case 11.
//!
//! Not covered here: text that merely *looks* like UI chrome (for example a
//! model writing "Allow? [y/n]") is shown as content; the real approval
//! panel is drawn by the shell, not by content (see next_tools.rs).

use claurst_core::config::Config;
use claurst_core::cost::CostTracker;
use claurst_core::coven_daemon::{MemoryOverview, MemoryPreview, MemorySummary};
use claurst_core::tasks::BackgroundTask;
use claurst_core::types::{ContentBlock, Message, ToolResultContent};
use claurst_tui::app::{App, ToolStatus, ToolUseBlock};
use claurst_tui::application::memory::MemorySnapshot;
use claurst_tui::application::UiRenderer;
use claurst_tui::dialogs::PermissionRequest;
use claurst_tui::next::style::{Appearance, ColorMode, Glyphs};
use claurst_tui::next::{InspectorPage, NextPresentation};
use claurst_tui::notifications::NotificationKind;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::Terminal;
use serde_json::json;

/// Clear screen, home cursor, OSC 52 clipboard write, OSC 8 hyperlink,
/// OSC 0 title, C1 CSI and OSC, carriage return, backspace, BEL, NUL, bidi
/// override and isolate, then CJK, a combining mark and a ZWJ emoji.
const HOSTILE: &str = "start\u{1b}[2J\u{1b}[H\u{1b}]52;c;Y2FuYXJ5\u{7}\u{1b}]8;;https://evil.example\u{1b}\\link\u{1b}]8;;\u{1b}\\\u{1b}]0;pwned\u{7}\u{9b}31m\u{9d}52;c;eA\u{9c}\r\u{8}\u{202e}cba\u{2066}x\u{2069} 漢字 e\u{301} 👩\u{200d}💻 \u{0}end";

const SIZES: [(u16, u16); 6] = [(20, 6), (40, 12), (60, 20), (80, 24), (120, 40), (180, 50)];
const COLORS: [ColorMode; 4] = [
    ColorMode::NoColor,
    ColorMode::Ansi16,
    ColorMode::Ansi256,
    ColorMode::TrueColor,
];

fn hostile_app() -> App {
    let config = Config {
        model: Some("claude-shell-model".to_string()),
        provider: Some("anthropic".to_string()),
        project_dir: Some("/work/acme-api".into()),
        ..Config::default()
    };
    let mut app = App::new(config, CostTracker::new());
    app.has_credentials = true;
    app.renderer = UiRenderer::Next;
    app.turn_start = None;

    app.messages = vec![
        Message::user(HOSTILE),
        Message::assistant(HOSTILE),
        Message::assistant_blocks(vec![
            ContentBlock::ToolUse {
                id: "toolu_fetch".to_string(),
                name: "WebFetch".to_string(),
                input: json!({ "url": format!("https://h.example/{HOSTILE}") }),
            },
            ContentBlock::ToolUse {
                id: "toolu_search".to_string(),
                name: "WebSearch".to_string(),
                input: json!({ "query": HOSTILE }),
            },
        ]),
        Message::user_blocks(vec![
            ContentBlock::ToolResult {
                tool_use_id: "toolu_fetch".to_string(),
                content: ToolResultContent::Text(HOSTILE.to_string()),
                is_error: Some(true),
            },
            ContentBlock::ToolResult {
                tool_use_id: "toolu_search".to_string(),
                content: ToolResultContent::Text(format!(
                    "1. **{HOSTILE}**\n   URL: https://h.example/{HOSTILE}\n   {HOSTILE}\n"
                )),
                is_error: Some(false),
            },
        ]),
    ];
    app.streaming_text = HOSTILE.to_string();
    app.status_message = Some(HOSTILE.to_string());
    app.session_title = Some(HOSTILE.to_string());
    app.tool_use_blocks = vec![ToolUseBlock {
        id: "toolu_bash".to_string(),
        name: HOSTILE.to_string(),
        turn_index: None,
        status: ToolStatus::Running,
        output_preview: Some(HOSTILE.to_string()),
        input_json: json!({ "command": HOSTILE }).to_string(),
    }];
    app.permission_request = Some(
        PermissionRequest::bash(
            "toolu_bash".to_string(),
            "Bash".to_string(),
            HOSTILE.to_string(),
            HOSTILE.to_string(),
            None,
        )
        .guarded(),
    );
    app.notifications
        .push(NotificationKind::Warning, HOSTILE.to_string(), None);
    app.prompt_input.replace_text(HOSTILE.to_string());
    app.refresh_prompt_input();

    app.work_sources.background = vec![BackgroundTask::new(HOSTILE)];
    app.memory_snapshot = MemorySnapshot::Loaded {
        rows: vec![MemorySummary {
            id: HOSTILE.to_string(),
            familiar_id: HOSTILE.to_string(),
            updated_at: Some(HOSTILE.to_string()),
            source_label: Some(HOSTILE.to_string()),
            verification_state: HOSTILE.to_string(),
            preview: MemoryPreview::Allowed {
                title: HOSTILE.to_string(),
                excerpt: HOSTILE.to_string(),
            },
        }],
        overview: Ok(MemoryOverview {
            verification_state: Some(HOSTILE.to_string()),
            ..MemoryOverview::default()
        }),
    };
    app
}

fn present(app: &mut App, color: ColorMode, glyphs: Glyphs) {
    app.next_presentation = NextPresentation {
        appearance: Appearance {
            color,
            glyphs,
            ..Appearance::default()
        },
        ..NextPresentation::default()
    };
}

fn draw(app: &App, width: u16, height: u16) -> Buffer {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("terminal");
    terminal
        .draw(|frame| claurst_tui::next::render_next(frame, app))
        .expect("draw");
    terminal.backend().buffer().clone()
}

fn is_unsafe(ch: char) -> bool {
    matches!(ch,
        '\u{0}'..='\u{1f}'
        | '\u{7f}'..='\u{9f}'
        | '\u{202a}'..='\u{202e}'
        | '\u{2066}'..='\u{2069}')
}

fn unsafe_cells(buffer: &Buffer) -> Vec<(u16, u16, String)> {
    let mut found = Vec::new();
    for y in 0..buffer.area.height {
        for x in 0..buffer.area.width {
            let symbol = buffer[(x, y)].symbol();
            if symbol.chars().any(is_unsafe) {
                found.push((x, y, symbol.escape_unicode().to_string()));
            }
        }
    }
    found
}

fn pages() -> Vec<Option<InspectorPage>> {
    std::iter::once(None)
        .chain(InspectorPage::ALL.into_iter().map(Some))
        .collect()
}

#[test]
fn no_control_character_reaches_any_cell_on_any_page_size_color_or_glyph_set() {
    let mut app = hostile_app();
    let mut renders = 0;
    for glyphs in [Glyphs::Unicode, Glyphs::Ascii] {
        for color in COLORS {
            present(&mut app, color, glyphs);
            for page in pages() {
                app.next_shell.inspector = page;
                for (width, height) in SIZES {
                    let found = unsafe_cells(&draw(&app, width, height));
                    assert!(
                        found.is_empty(),
                        "{page:?} {width}x{height} {color:?} {glyphs:?}: {found:?}"
                    );
                    renders += 1;
                }
            }
        }
    }
    assert_eq!(renders, 2 * 4 * 7 * 6, "every combination was drawn");
}

#[test]
fn hostile_sequences_are_shown_as_inert_text() {
    let mut app = hostile_app();
    present(&mut app, ColorMode::NoColor, Glyphs::Ascii);
    let buffer = draw(&app, 180, 50);
    let text: String = buffer.content.iter().map(|cell| cell.symbol()).collect();
    assert!(text.contains("^[[2J"), "screen clear shown as text");
    assert!(text.contains("^[]52;c;"), "clipboard write shown as text");
    // A wide character fills two cells, so check each on its own.
    assert!(text.contains('漢') && text.contains('字'), "wide text kept");
}

#[test]
fn the_way_back_stays_visible_on_every_page_from_40x12_up() {
    let mut app = hostile_app();
    app.permission_request = None;
    for color in COLORS {
        present(&mut app, color, Glyphs::Ascii);
        for page in InspectorPage::ALL {
            app.next_shell.inspector = Some(page);
            for (width, height) in SIZES.into_iter().filter(|(w, h)| *w >= 40 && *h >= 12) {
                let buffer = draw(&app, width, height);
                let text: String = buffer.content.iter().map(|cell| cell.symbol()).collect();
                assert!(
                    text.contains("Esc"),
                    "{page:?} {width}x{height} {color:?}: no way back"
                );
            }
        }
    }
}

#[test]
fn no_color_mode_draws_no_color_on_any_page() {
    use ratatui::style::Color;
    let mut app = hostile_app();
    present(&mut app, ColorMode::NoColor, Glyphs::Ascii);
    for page in pages() {
        app.next_shell.inspector = page;
        for (width, height) in SIZES {
            let buffer = draw(&app, width, height);
            let colored: Vec<_> = buffer
                .content
                .iter()
                .enumerate()
                .filter(|(_, cell)| cell.fg != Color::Reset || cell.bg != Color::Reset)
                .map(|(index, cell)| {
                    (
                        index % usize::from(width),
                        index / usize::from(width),
                        cell.fg,
                        cell.bg,
                    )
                })
                .take(4)
                .collect();
            assert!(colored.is_empty(), "{page:?} {width}x{height}: {colored:?}");
        }
    }
}

#[test]
fn the_next_toast_shows_kind_text_and_the_way_to_dismiss_it() {
    let mut app = hostile_app();
    app.notifications = Default::default();
    app.permission_request = None;
    app.notifications.push(
        NotificationKind::Warning,
        "Disk almost full".to_string(),
        None,
    );
    present(&mut app, ColorMode::NoColor, Glyphs::Ascii);
    let buffer = draw(&app, 100, 30);
    let row: String = (0..100)
        .map(|x| buffer[(x, 1)].symbol().to_string())
        .collect();
    assert!(row.contains("warning Disk almost full"), "{row}");
    assert!(row.contains("Esc"), "{row}");
}

#[test]
fn the_terminal_title_topic_carries_no_control_characters_in_next() {
    let hostile = "show me\u{7}\u{1b}]52;c;cHduZWQ=\u{7}\u{1b}]0;PWNED\u{9c}\nnext";
    let topic = claurst_tui::terminal_title_topic(UiRenderer::Next, hostile);
    assert!(!topic.chars().any(is_unsafe), "{}", topic.escape_debug());
    assert!(topic.starts_with("show me"), "{topic}");
    // Classic is unchanged by this slice; its title is tracked separately.
    assert_eq!(
        claurst_tui::terminal_title_topic(UiRenderer::Classic, hostile),
        hostile
    );
}
