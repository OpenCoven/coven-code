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
//! The matrix runs three variants: an approval pending (which hides the
//! inspector), plain, and an error notice, which Next draws itself.
//!
//! Not covered here:
//! - Frames Next hands to Classic (`delegates_to_classic`): slash and file
//!   suggestions, the effort picker, the familiar switcher and other modals.
//!   Classic draws them unchanged, including its transcript; tracked on #417.
//! - Text that merely *looks* like UI chrome (a model writing "Allow?
//!   [y/n]") is shown as content; the real approval panel is drawn by the
//!   shell, not by content (see next_tools.rs).
//! - Bidirectional overrides never reach a cell (ratatui drops zero-width
//!   graphemes); they matter on the title path, where
//!   `terminal_title_topic` drops them. Direction marks (U+200E/F, U+061C)
//!   and line separators (U+2028/9) are not filtered.

use claurst_core::config::Config;
use claurst_core::cost::CostTracker;
use claurst_core::coven_daemon::{MemoryOverview, MemoryPreview, MemorySummary};
use claurst_core::tasks::BackgroundTask;
use claurst_core::types::{ContentBlock, DocumentSource, ImageSource, Message, ToolResultContent};
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

/// Which overlay the frame carries: an approval takes the main pane and
/// hides the inspector, so the matrix draws every variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Variant {
    Approval,
    Plain,
    ErrorNotice,
}

const VARIANTS: [Variant; 3] = [Variant::Approval, Variant::Plain, Variant::ErrorNotice];

fn hostile_app(variant: Variant) -> App {
    let config = Config {
        model: Some(HOSTILE.to_string()),
        provider: Some("anthropic".to_string()),
        project_dir: Some(format!("/work/acme-api{HOSTILE}").into()),
        familiar: Some(HOSTILE.to_string()),
        ..Config::default()
    };
    let mut app = App::new(config, CostTracker::new());
    app.session_id = Some(HOSTILE.to_string());
    app.has_credentials = true;
    app.renderer = UiRenderer::Next;
    app.turn_start = None;
    app.model_name = HOSTILE.to_string();

    app.messages = vec![
        Message::user(HOSTILE),
        Message::user_blocks(vec![
            ContentBlock::Image {
                source: ImageSource {
                    source_type: "base64".to_string(),
                    media_type: Some(HOSTILE.to_string()),
                    data: None,
                    url: None,
                },
            },
            ContentBlock::Document {
                source: DocumentSource {
                    source_type: "base64".to_string(),
                    media_type: Some(HOSTILE.to_string()),
                    data: None,
                    url: None,
                },
                title: Some(HOSTILE.to_string()),
                context: None,
                citations: None,
            },
        ]),
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
    ]
    .into();
    app.streaming_text = HOSTILE.to_string().into();
    app.status_message = Some(HOSTILE.to_string());
    app.session_title = Some(HOSTILE.to_string());
    app.tool_use_blocks = vec![ToolUseBlock {
        id: "toolu_bash".to_string(),
        name: HOSTILE.to_string(),
        turn_index: None,
        status: ToolStatus::Running,
        output_preview: Some(HOSTILE.to_string()),
        input_json: json!({ "command": HOSTILE }).to_string(),
    }]
    .into();
    if variant == Variant::Approval {
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
    }
    let kind = if variant == Variant::ErrorNotice {
        NotificationKind::Error
    } else {
        NotificationKind::Warning
    };
    app.notifications.push(kind, HOSTILE.to_string(), None);
    app.prompt_input
        .pending_images
        .push(claurst_tui::image_paste::PastedImage {
            path: "/tmp/pasted.png".into(),
            label: HOSTILE.to_string(),
            dimensions: None,
        });
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
    let mut renders = 0;
    for variant in VARIANTS {
        let mut app = hostile_app(variant);
        for glyphs in [Glyphs::Unicode, Glyphs::Ascii] {
            for color in COLORS {
                present(&mut app, color, glyphs);
                for page in pages() {
                    app.next_shell.inspector = page;
                    for (width, height) in SIZES {
                        let found = unsafe_cells(&draw(&app, width, height));
                        assert!(
                            found.is_empty(),
                            "{variant:?} {page:?} {width}x{height} {color:?} {glyphs:?}: {found:?}"
                        );
                        renders += 1;
                    }
                }
            }
        }
    }
    assert_eq!(renders, 3 * 2 * 4 * 7 * 6, "every combination was drawn");
}

#[test]
fn without_an_approval_every_page_draws_its_own_content() {
    // Guards the matrix: a pending approval hides the inspector, so the
    // plain variant must really reach each page.
    let mut app = hostile_app(Variant::Plain);
    present(&mut app, ColorMode::NoColor, Glyphs::Ascii);
    for (width, height) in SIZES.into_iter().filter(|(w, h)| *w >= 40 && *h >= 12) {
        app.next_shell.inspector = None;
        let base = draw(&app, width, height);
        let mut seen = vec![base.clone()];
        for page in InspectorPage::ALL {
            app.next_shell.inspector = Some(page);
            let buffer = draw(&app, width, height);
            assert!(
                !seen.contains(&buffer),
                "{page:?} at {width}x{height} drew the same frame as another page"
            );
            seen.push(buffer);
        }
    }
}

#[test]
fn hostile_sequences_are_shown_as_inert_text() {
    let mut app = hostile_app(Variant::Plain);
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
    let mut app = hostile_app(Variant::Plain);
    // No toast, so its own "Esc" cannot satisfy the check.
    app.notifications = Default::default();
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
    for variant in VARIANTS {
        let mut app = hostile_app(variant);
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
                assert!(
                    colored.is_empty(),
                    "{variant:?} {page:?} {width}x{height}: {colored:?}"
                );
            }
        }
    }
}

#[test]
fn the_next_toast_shows_kind_text_and_the_way_to_dismiss_it() {
    let mut app = hostile_app(Variant::Plain);
    app.notifications = Default::default();
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

#[test]
fn a_toast_never_covers_the_approval_or_the_inspector_title() {
    let mut app = hostile_app(Variant::Approval);
    app.notifications = Default::default();
    app.notifications.push(
        NotificationKind::Info,
        "Image attached: cat.png".to_string(),
        None,
    );
    present(&mut app, ColorMode::NoColor, Glyphs::Ascii);
    for (width, height) in [(40, 12), (60, 20), (120, 40)] {
        let text = text_of(&draw(&app, width, height));
        assert!(text.contains("Approve"), "{width}x{height}:\n{text}");
    }
    app.permission_request = None;
    app.next_shell.inspector = Some(InspectorPage::Activity);
    for (width, height) in [(40, 12), (120, 40)] {
        let text = text_of(&draw(&app, width, height));
        // The inspector title row ("- Activity" in ASCII) stays whole; in
        // Split the toast may share the row, but only left of the inspector.
        let row = text
            .lines()
            .find(|line| line.contains("- Activity"))
            .unwrap_or_else(|| panic!("{width}x{height} title lost:\n{text}"));
        if let Some(toast) = row.find("cat.png") {
            assert!(toast < row.find("- Activity").unwrap(), "{row}");
        }
    }
}

#[test]
fn an_error_notice_is_drawn_by_next_with_its_text_and_the_way_to_dismiss_it() {
    let mut app = hostile_app(Variant::Plain);
    app.notifications = Default::default();
    app.notifications.push(
        NotificationKind::Error,
        "API error: overloaded, retry later".to_string(),
        None,
    );
    assert!(!claurst_tui::next::delegates_to_classic(&app));
    present(&mut app, ColorMode::NoColor, Glyphs::Ascii);
    let text = text_of(&draw(&app, 80, 24));
    assert!(text.contains("overloaded"), "{text}");
    assert!(text.contains("Esc"), "{text}");
}

#[test]
fn a_pending_approval_defers_error_notices_without_losing_them() {
    for (width, height) in SIZES {
        let mut app = hostile_app(Variant::Approval);
        app.notifications = Default::default();
        present(&mut app, ColorMode::NoColor, Glyphs::Ascii);
        let approval = draw(&app, width, height);
        app.notifications.push(
            NotificationKind::Error,
            "API overloaded; retry later. ".repeat(100),
            None,
        );
        assert_eq!(
            draw(&app, width, height),
            approval,
            "the input-owning approval must stay intact at {width}x{height}"
        );
        assert!(app.notifications.current_is_error());
        app.permission_request = None;
        let text = text_of(&draw(&app, width, height));
        assert!(text.contains("error"), "{width}x{height}:\n{text}");
        app.handle_key_event(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Esc,
            crossterm::event::KeyModifiers::NONE,
        ));
        assert!(!app.notifications.current_is_error());
    }
}

#[test]
fn esc_closes_the_inspector_while_its_notice_waits_unseen() {
    let mut app = hostile_app(Variant::Plain);
    app.notifications = Default::default();
    app.notifications
        .push(NotificationKind::Info, "Familiar: sage".to_string(), None);
    present(&mut app, ColorMode::NoColor, Glyphs::Ascii);
    app.next_shell.inspector = Some(InspectorPage::Activity);
    // Below 120x32 the inspector replaces the transcript, so the notice waits.
    let text = text_of(&draw(&app, 80, 24));
    assert!(!text.contains("Familiar: sage"), "{text}");
    app.handle_key_event(crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Esc,
        crossterm::event::KeyModifiers::NONE,
    ));
    assert_eq!(app.next_shell.inspector, None, "Esc closes the inspector");
    let text = text_of(&draw(&app, 80, 24));
    assert!(
        text.contains("Familiar: sage"),
        "the notice is kept:\n{text}"
    );
    app.handle_key_event(crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Esc,
        crossterm::event::KeyModifiers::NONE,
    ));
    assert!(
        app.notifications.is_empty(),
        "a shown notice still dismisses"
    );
}

fn text_of(buffer: &Buffer) -> String {
    let width = usize::from(buffer.area.width.max(1));
    buffer
        .content
        .chunks(width)
        .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn every_scheme_and_motion_setting_keeps_cells_inert_and_the_way_back() {
    use claurst_tui::next::style::{Motion, Scheme};
    let mut app = hostile_app(Variant::Plain);
    app.notifications = Default::default();
    for scheme in [Scheme::Dark, Scheme::Light] {
        for motion in [Motion::Full, Motion::Reduced] {
            for color in COLORS {
                app.next_presentation = NextPresentation {
                    appearance: Appearance {
                        color,
                        scheme,
                        glyphs: Glyphs::Unicode,
                        motion,
                    },
                    ..NextPresentation::default()
                };
                for page in pages() {
                    app.next_shell.inspector = page;
                    for (width, height) in SIZES {
                        let buffer = draw(&app, width, height);
                        let found = unsafe_cells(&buffer);
                        assert!(
                            found.is_empty(),
                            "{scheme:?} {motion:?} {color:?} {page:?} {width}x{height}: {found:?}"
                        );
                        if page.is_some() && width >= 40 && height >= 12 {
                            assert!(
                                text_of(&buffer).contains("Esc"),
                                "{scheme:?} {motion:?} {color:?} {page:?} {width}x{height}"
                            );
                        }
                    }
                }
            }
        }
    }
}
