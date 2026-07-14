// onboarding_dialog.rs — First-launch welcome / onboarding dialog.
//
// Mirrors the TypeScript first-launch experience:
// - Shown once on first run (when Settings.has_completed_onboarding == false).
// - Walks the user through a brief orientation: key bindings, model info, help.
// - Dismissed by pressing Enter or Esc; sets has_completed_onboarding in settings.

use ratatui::layout::Rect;
use ratatui::prelude::Stylize;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Widget, Wrap};
use ratatui::Frame;

use crate::overlays::centered_rect;

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

/// Which page of the onboarding flow we're on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OnboardingPage {
    /// Shown when no provider login is configured.
    ProviderSetup,
    /// Main entry page for users who already have credentials configured.
    /// Default so a freshly-constructed `OnboardingDialogState` lands here
    /// instead of jumping into the credential-setup flow on `.visible = true`.
    #[default]
    Welcome,
    KeyBindings,
    Done,
}

/// State for the first-launch onboarding dialog.
#[derive(Debug, Default, Clone)]
pub struct OnboardingDialogState {
    /// Whether the dialog is currently visible.
    pub visible: bool,
    /// Current page.
    pub page: OnboardingPage,
    /// Whether the flow was entered via the provider-setup page (no
    /// credentials found). Controls page count labels and back-navigation.
    entered_via_provider_setup: bool,
}

impl OnboardingDialogState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Show the normal onboarding (first-run with credentials already configured).
    pub fn show(&mut self) {
        self.visible = true;
        self.page = OnboardingPage::Welcome;
        self.entered_via_provider_setup = false;
    }

    /// Show the provider setup page (no credentials configured). The flow
    /// continues through Welcome and KeyBindings afterwards.
    pub fn show_provider_setup(&mut self) {
        self.visible = true;
        self.page = OnboardingPage::ProviderSetup;
        self.entered_via_provider_setup = true;
    }

    pub fn dismiss(&mut self) {
        self.visible = false;
    }

    /// `(current, total)` page label for the active flow.
    fn page_progress(&self) -> (usize, usize) {
        let total = if self.entered_via_provider_setup {
            3
        } else {
            2
        };
        let current = match (self.page, self.entered_via_provider_setup) {
            (OnboardingPage::ProviderSetup, _) => 1,
            (OnboardingPage::Welcome, true) => 2,
            (OnboardingPage::Welcome, false) => 1,
            (OnboardingPage::KeyBindings, true) => 3,
            (OnboardingPage::KeyBindings, false) => 2,
            (OnboardingPage::Done, _) => total,
        };
        (current, total)
    }

    /// Advance to the next page; returns true if we've reached Done and should dismiss.
    pub fn next_page(&mut self) -> bool {
        self.page = match self.page {
            OnboardingPage::ProviderSetup => OnboardingPage::Welcome,
            OnboardingPage::Welcome => OnboardingPage::KeyBindings,
            OnboardingPage::KeyBindings => OnboardingPage::Done,
            OnboardingPage::Done => OnboardingPage::Done,
        };
        self.page == OnboardingPage::Done
    }

    /// Go back to the previous page.
    pub fn prev_page(&mut self) {
        self.page = match self.page {
            OnboardingPage::ProviderSetup => OnboardingPage::ProviderSetup,
            OnboardingPage::Welcome if self.entered_via_provider_setup => {
                OnboardingPage::ProviderSetup
            }
            OnboardingPage::Welcome => OnboardingPage::Welcome,
            OnboardingPage::KeyBindings => OnboardingPage::Welcome,
            OnboardingPage::Done => OnboardingPage::KeyBindings,
        };
    }

    pub fn is_done(&self) -> bool {
        self.page == OnboardingPage::Done
    }

    /// True while the no-credentials provider-setup page is showing. The app
    /// uses this so Enter / a provider number opens the real connect dialog
    /// instead of advancing into the (purely informational) welcome pages.
    pub fn is_provider_setup(&self) -> bool {
        self.page == OnboardingPage::ProviderSetup
    }
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

pub fn render_onboarding_dialog(frame: &mut Frame, state: &OnboardingDialogState, area: Rect) {
    if !state.visible {
        return;
    }

    let dialog_width = 72u16.min(area.width.saturating_sub(4));
    let dialog_height = 26u16.min(area.height.saturating_sub(4));
    let dialog_area = centered_rect(dialog_width, dialog_height, area);

    frame.render_widget(Clear, dialog_area);

    match state.page {
        OnboardingPage::ProviderSetup => render_provider_setup_page(frame, dialog_area),
        OnboardingPage::Welcome => render_welcome_page(frame, state, dialog_area),
        OnboardingPage::KeyBindings => render_keybindings_page(frame, state, dialog_area),
        OnboardingPage::Done => {} // should not be visible
    }
}

/// A provider entry on the setup page.
///
/// Coven Code supports two providers: Claude (Anthropic) and Codex. Claude
/// leads, then Codex.
struct ProviderEntry {
    name: &'static str,
    tagline: &'static str,
    setup: &'static str,
    setup_suffix: &'static str,
}

const PROVIDER_ENTRIES: &[ProviderEntry] = &[
    ProviderEntry {
        name: "Claude",
        tagline: "  Opus · Sonnet · Haiku",
        setup: "Claude CLI",
        setup_suffix: "  import existing login",
    },
    ProviderEntry {
        name: "Codex",
        tagline: "  gpt-5.6 via Codex CLI login",
        setup: "Codex CLI",
        setup_suffix: "",
    },
];

fn render_provider_setup_page(frame: &mut Frame, area: Rect) {
    // Theme pink — matches the header and mascot
    let pink = crate::overlays::COVEN_CODE_ACCENT;
    let dim = Color::Rgb(100, 100, 100);
    let esc_red = Color::Red;

    let block = Block::default()
        .borders(Borders::ALL)
        .title(Line::from(vec![
            Span::styled("─── ", Style::default().fg(pink)),
            Span::styled(
                " Connect a Provider ",
                Style::default().fg(pink).add_modifier(Modifier::BOLD),
            ),
            Span::styled(" ───", Style::default().fg(pink)),
        ]))
        .border_style(Style::default().fg(pink));

    let inner = block.inner(area);
    frame.render_widget(block, area);
    Paragraph::new(Line::from(Span::styled(
        " esc ",
        Style::default().fg(esc_red).add_modifier(Modifier::BOLD),
    )))
    .render(
        Rect {
            x: area.x + area.width.saturating_sub(6),
            y: area.y,
            width: 5,
            height: 1,
        },
        frame.buffer_mut(),
    );

    let sep = "  ─────────────────────────────────────────────────";

    let mut lines: Vec<Line<'static>> = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled(
                "  No credentials found. ",
                Style::default().fg(Color::White),
            ),
            Span::styled(
                "Pick a provider below:",
                Style::default().fg(Color::Rgb(180, 180, 180)),
            ),
        ]),
        Line::from(""),
    ];

    for (i, entry) in PROVIDER_ENTRIES.iter().enumerate() {
        lines.push(Line::from(vec![
            Span::styled(
                format!("  {}  ", i + 1),
                Style::default().fg(pink).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                entry.name,
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(entry.tagline, Style::default().fg(dim)),
        ]));
        lines.push(Line::from(vec![
            Span::styled("     › ", Style::default().fg(pink)),
            Span::styled(
                entry.setup,
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(entry.setup_suffix, Style::default().fg(dim)),
        ]));
        if i + 1 < PROVIDER_ENTRIES.len() {
            lines.push(Line::from(Span::styled(
                sep,
                Style::default().fg(Color::Rgb(45, 45, 55)),
            )));
        }
    }

    lines.extend([
        Line::from(""),
        Line::from(vec![
            Span::styled("  1·2", Style::default().fg(pink)),
            Span::styled(" or ", Style::default().fg(dim)),
            Span::styled("enter", Style::default().fg(pink)),
            Span::styled(" choose setup · ", Style::default().fg(dim)),
            Span::styled("esc", Style::default().fg(pink)),
            Span::styled(" skip · run ", Style::default().fg(dim)),
            Span::styled("/connect", Style::default().fg(Color::Rgb(150, 150, 150))),
            Span::styled(" anytime", Style::default().fg(dim)),
        ]),
    ]);

    Paragraph::new(lines)
        .wrap(Wrap { trim: false })
        .render(inner, frame.buffer_mut());
}

fn render_welcome_page(frame: &mut Frame, state: &OnboardingDialogState, area: Rect) {
    use crate::overlays::{render_dark_overlay, render_dialog_bg, COVEN_CODE_PANEL_BG};

    let pink = crate::overlays::COVEN_CODE_ACCENT;
    let dim = Color::Rgb(90, 90, 90);
    let text = Color::Rgb(210, 210, 215);

    render_dark_overlay(frame, area);
    render_dialog_bg(frame, area);

    let inner = Rect {
        x: area.x + 2,
        y: area.y + 1,
        width: area.width.saturating_sub(4),
        height: area.height.saturating_sub(2),
    };

    let cmd_label = |slash: &str, desc: &str| -> Line<'static> {
        Line::from(vec![
            Span::styled(format!("  {:<12}", slash), Style::default().fg(pink)),
            Span::styled(desc.to_string(), Style::default().fg(text)),
        ])
    };

    let (page_n, page_total) = state.page_progress();
    let lines: Vec<Line<'static>> = vec![
        Line::from(vec![
            Span::styled(
                " Welcome to Coven",
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!(
                    "{:>width$}",
                    format!("{}/{} ", page_n, page_total),
                    width = inner.width.saturating_sub(21) as usize
                ),
                Style::default().fg(dim),
            ),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "  Coven is an AI-powered coding assistant in your terminal.",
            Style::default().fg(text),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "  How to use:",
            Style::default().fg(pink).add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            "  Type your request and press Enter to send it.",
            Style::default().fg(text),
        )),
        Line::from(Span::styled(
            "  Coven can read, edit, and create files in your project.",
            Style::default().fg(text),
        )),
        Line::from(Span::styled(
            "  Coven can run bash commands, search the web, and more.",
            Style::default().fg(text),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "  Slash commands:",
            Style::default().fg(pink).add_modifier(Modifier::BOLD),
        )),
        cmd_label("/help", "show all commands"),
        cmd_label("/model", "switch AI model"),
        cmd_label("/connect", "connect a provider"),
        cmd_label("/compact", "summarise conversation to save context"),
        cmd_label("/cost", "show token usage and cost"),
        Line::from(""),
        Line::from(vec![
            Span::styled("  enter ", Style::default().fg(dim)),
            Span::styled("next", Style::default().fg(dim)),
            Span::styled("  ·  ", Style::default().fg(Color::Rgb(50, 50, 50))),
            Span::styled("esc ", Style::default().fg(dim)),
            Span::styled("skip", Style::default().fg(dim)),
        ]),
    ];

    Paragraph::new(lines)
        .bg(COVEN_CODE_PANEL_BG)
        .render(inner, frame.buffer_mut());
}

fn render_keybindings_page(frame: &mut Frame, state: &OnboardingDialogState, area: Rect) {
    use crate::overlays::{render_dark_overlay, render_dialog_bg, COVEN_CODE_PANEL_BG};

    let pink = crate::overlays::COVEN_CODE_ACCENT;
    let dim = Color::Rgb(90, 90, 90);
    let text = Color::Rgb(210, 210, 215);

    render_dark_overlay(frame, area);
    render_dialog_bg(frame, area);

    let inner = Rect {
        x: area.x + 2,
        y: area.y + 1,
        width: area.width.saturating_sub(4),
        height: area.height.saturating_sub(2),
    };

    let kb = |key: &str, desc: &str| -> Line<'static> {
        Line::from(vec![
            Span::styled(format!("  {:<15}", key), Style::default().fg(pink)),
            Span::styled(desc.to_string(), Style::default().fg(text)),
        ])
    };

    // The input-editing actions are user-customizable via keybindings.json, so
    // reflect the real binding rather than a hardcoded label that can drift.
    // Fixed keys (Tab mode cycle, F1/F2, permission y/Y/n) live outside that
    // system and stay static.
    let user_kb = claurst_core::keybindings::UserKeybindings::load(
        &claurst_core::config::Settings::config_dir(),
    );
    let submit_key = claurst_core::keybindings::display_binding_for(&user_kb, "submit")
        .unwrap_or_else(|| "Enter".to_string());
    let help_key = claurst_core::keybindings::display_binding_for(&user_kb, "openHelp")
        .unwrap_or_else(|| "Alt+H".to_string());

    let (page_n, page_total) = state.page_progress();
    let mut lines: Vec<Line<'static>> = vec![
        Line::from(vec![
            Span::styled(
                " Keyboard Shortcuts",
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!(
                    "{:>width$}",
                    format!("{}/{} ", page_n, page_total),
                    width = inner.width.saturating_sub(21) as usize
                ),
                Style::default().fg(dim),
            ),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "  Input",
            Style::default().fg(pink).add_modifier(Modifier::BOLD),
        )),
        kb(&submit_key, "send message"),
        kb("Shift+Enter", "newline"),
        kb("Ctrl+C", "interrupt / cancel"),
        kb("Tab", "cycle mode (build/plan/explore)"),
        kb("\u{2191}\u{2193}", "history"),
        Line::from(""),
        Line::from(Span::styled(
            "  Navigation",
            Style::default().fg(pink).add_modifier(Modifier::BOLD),
        )),
        kb("PgUp/PgDn", "scroll transcript"),
        kb("Ctrl+K", "command palette"),
        kb("Ctrl+Shift+A", "model picker"),
        kb("F1", "toggle help overlay"),
        kb("F2", "switch familiar"),
        kb(&help_key, "open help"),
        kb("Ctrl+B", "create / switch branch"),
        Line::from(""),
        Line::from(Span::styled(
            "  Permissions",
            Style::default().fg(pink).add_modifier(Modifier::BOLD),
        )),
        kb("y", "allow tool once"),
        kb("Y", "allow all this session"),
        kb("n", "deny tool"),
    ];

    // Footer at bottom
    let footer_y = inner.height.saturating_sub(1) as usize;
    while lines.len() < footer_y {
        lines.push(Line::from(""));
    }
    lines.push(Line::from(vec![
        Span::styled("  enter ", Style::default().fg(dim)),
        Span::styled("done", Style::default().fg(dim)),
        Span::styled("  ·  ", Style::default().fg(Color::Rgb(50, 50, 50))),
        Span::styled("\u{2190} ", Style::default().fg(dim)),
        Span::styled("back", Style::default().fg(dim)),
        Span::styled("  ·  ", Style::default().fg(Color::Rgb(50, 50, 50))),
        Span::styled("esc ", Style::default().fg(dim)),
        Span::styled("close", Style::default().fg(dim)),
    ]));

    Paragraph::new(lines)
        .bg(COVEN_CODE_PANEL_BG)
        .render(inner, frame.buffer_mut());
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    #[test]
    fn onboarding_defaults_hidden() {
        let state = OnboardingDialogState::new();
        assert!(!state.visible);
        assert_eq!(state.page, OnboardingPage::Welcome);
    }

    #[test]
    fn onboarding_show_sets_visible() {
        let mut state = OnboardingDialogState::new();
        state.show();
        assert!(state.visible);
        assert_eq!(state.page, OnboardingPage::Welcome);
    }

    #[test]
    fn onboarding_next_page_cycles() {
        let mut state = OnboardingDialogState::new();
        state.show();
        assert!(!state.next_page()); // Welcome → KeyBindings
        assert_eq!(state.page, OnboardingPage::KeyBindings);
        assert!(state.next_page()); // KeyBindings → Done
        assert_eq!(state.page, OnboardingPage::Done);
        assert!(state.is_done());
    }

    #[test]
    fn onboarding_prev_page() {
        let mut state = OnboardingDialogState::new();
        state.show();
        state.next_page();
        state.prev_page();
        assert_eq!(state.page, OnboardingPage::Welcome);
        // Without provider-setup entry, Welcome is the first page.
        state.prev_page();
        assert_eq!(state.page, OnboardingPage::Welcome);
    }

    #[test]
    fn onboarding_provider_setup_flow() {
        let mut state = OnboardingDialogState::new();
        state.show_provider_setup();
        assert_eq!(state.page, OnboardingPage::ProviderSetup);
        assert_eq!(state.page_progress(), (1, 3));

        assert!(!state.next_page()); // ProviderSetup → Welcome
        assert_eq!(state.page, OnboardingPage::Welcome);
        assert_eq!(state.page_progress(), (2, 3));

        // Back-navigation returns to provider setup in this flow.
        state.prev_page();
        assert_eq!(state.page, OnboardingPage::ProviderSetup);
        state.next_page();

        assert!(!state.next_page()); // Welcome → KeyBindings
        assert_eq!(state.page, OnboardingPage::KeyBindings);
        assert_eq!(state.page_progress(), (3, 3));

        assert!(state.next_page()); // KeyBindings → Done
        assert!(state.is_done());
    }

    #[test]
    fn provider_setup_renders_free_mode_first_and_neutral_order() {
        let mut terminal = Terminal::new(TestBackend::new(100, 40)).unwrap();
        let mut state = OnboardingDialogState::new();
        state.show_provider_setup();
        terminal
            .draw(|frame| {
                render_onboarding_dialog(frame, &state, frame.area());
            })
            .unwrap();
        let buffer = terminal.backend().buffer().clone();
        let width = buffer.area.width as usize;
        let rows: Vec<String> = buffer
            .content()
            .chunks(width)
            .map(|row| {
                row.iter()
                    .map(|c| c.symbol().chars().next().unwrap_or(' '))
                    .collect()
            })
            .collect();
        let content = rows.join("\n");
        let (title_y, title_row) = rows
            .iter()
            .enumerate()
            .find(|(_, line)| line.contains("Connect a Provider"))
            .expect("provider setup title row missing");
        assert!(
            title_row.contains("esc"),
            "provider setup title should render an esc hint in the top corner, got {title_row:?}"
        );
        let esc_byte = title_row
            .find("esc")
            .expect("provider setup title row missing esc hint");
        let esc_x = title_row[..esc_byte].chars().count();
        for offset in 0..3 {
            let cell = &buffer.content()[title_y * width + esc_x + offset];
            assert_eq!(cell.fg, Color::Red, "provider setup esc hint should be red");
        }
        // Claude leads, then Codex.
        let claude = content.find("Claude").expect("Claude entry missing");
        let codex = content.find("Codex").expect("Codex entry missing");
        assert!(claude < codex, "Claude should precede Codex");
        assert!(
            content.contains("Claude CLI"),
            "provider setup should point users at Claude CLI login/import"
        );
        assert!(
            content.contains("Codex CLI"),
            "provider setup should point users at Codex CLI login"
        );
        assert!(
            !content.to_ascii_lowercase().contains("api key"),
            "provider setup should not advertise API keys"
        );
        assert!(
            !content.to_ascii_lowercase().contains("oauth"),
            "provider setup should not advertise OAuth setup"
        );
    }

    #[test]
    fn onboarding_renders_without_panic() {
        let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
        let mut state = OnboardingDialogState::new();
        state.show();
        terminal
            .draw(|frame| {
                render_onboarding_dialog(frame, &state, frame.area());
            })
            .unwrap();
        let content: String = terminal
            .backend()
            .buffer()
            .clone()
            .content()
            .iter()
            .map(|c| c.symbol().chars().next().unwrap_or(' '))
            .collect();
        assert!(content.contains("Welcome") || content.contains("Coven Code"));
    }

    #[test]
    fn onboarding_keybindings_page_renders() {
        let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
        let mut state = OnboardingDialogState::new();
        state.show();
        state.next_page();
        terminal
            .draw(|frame| {
                render_onboarding_dialog(frame, &state, frame.area());
            })
            .unwrap();
        let content: String = terminal
            .backend()
            .buffer()
            .clone()
            .content()
            .iter()
            .map(|c| c.symbol().chars().next().unwrap_or(' '))
            .collect();
        assert!(content.contains("Keyboard") || content.contains("Enter"));
        for expected in ["F2", "Alt+H", "Ctrl+B", "Tab", "build/plan/explore"] {
            assert!(
                content.contains(expected),
                "onboarding keybindings should mention {expected}, got {content:?}"
            );
        }
    }

    #[test]
    fn onboarding_hidden_renders_nothing() {
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        let state = OnboardingDialogState::new(); // visible = false
        let before = terminal.backend().buffer().clone();
        terminal
            .draw(|frame| {
                render_onboarding_dialog(frame, &state, frame.area());
            })
            .unwrap();
        assert_eq!(terminal.backend().buffer().content(), before.content());
    }
}
