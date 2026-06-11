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
    /// Shown when no API credentials are configured — provider picker.
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
/// Ordering is deliberately neutral: Coven Code is multi-provider, so no
/// vendor is privileged. Free Mode leads (zero-friction), then providers
/// that need no API key, then key-based providers alphabetically.
struct ProviderEntry {
    name: &'static str,
    tagline: &'static str,
    setup: &'static str,
    setup_suffix: &'static str,
}

const PROVIDER_ENTRIES: &[ProviderEntry] = &[
    ProviderEntry {
        name: "Ollama",
        tagline: "  Local models · No key needed",
        setup: "coven-code --provider ollama",
        setup_suffix: "",
    },
    ProviderEntry {
        name: "Anthropic",
        tagline: "  Claude Opus · Sonnet · Haiku",
        setup: "ANTHROPIC_API_KEY",
        setup_suffix: "  or configured OAuth",
    },
    ProviderEntry {
        name: "Google",
        tagline: "  Gemini 2.5 Pro · Flash",
        setup: "set GOOGLE_API_KEY=<key>",
        setup_suffix: "  then restart",
    },
    ProviderEntry {
        name: "Groq",
        tagline: "  Fast inference · Free tier · groq.com/keys",
        setup: "set GROQ_API_KEY=<key>",
        setup_suffix: "  then restart",
    },
    ProviderEntry {
        name: "OpenAI",
        tagline: "  GPT-4o · o3 · o4-mini",
        setup: "set OPENAI_API_KEY=<key>",
        setup_suffix: "  then restart",
    },
];

fn render_provider_setup_page(frame: &mut Frame, area: Rect) {
    // Theme pink — matches the header and mascot
    let pink = Color::Rgb(139, 92, 246);
    let dim = Color::Rgb(100, 100, 100);

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
        // ── Free Mode — zero-friction entry point ─────────────
        Line::from(vec![
            Span::styled(
                "  ★  ",
                Style::default().fg(pink).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "Free Mode",
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "  Agentic coding · No API key (experimental)",
                Style::default().fg(dim),
            ),
        ]),
        Line::from(vec![
            Span::styled("     › ", Style::default().fg(pink)),
            Span::styled(
                "/connect",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("  then pick \"Free\"", Style::default().fg(dim)),
        ]),
        Line::from(Span::styled(
            sep,
            Style::default().fg(Color::Rgb(45, 45, 55)),
        )),
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
            Span::styled("  + ", Style::default().fg(Color::Rgb(120, 120, 120))),
            Span::styled(
                "20+ more providers: ",
                Style::default().fg(Color::Rgb(120, 120, 120)),
            ),
            Span::styled(
                "coven-code --help",
                Style::default().fg(Color::Rgb(150, 150, 150)),
            ),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("  enter", Style::default().fg(pink)),
            Span::styled(" next · ", Style::default().fg(dim)),
            Span::styled("esc", Style::default().fg(pink)),
            Span::styled(" dismiss · configure later with ", Style::default().fg(dim)),
            Span::styled("/providers", Style::default().fg(Color::Rgb(150, 150, 150))),
        ]),
    ]);

    Paragraph::new(lines)
        .wrap(Wrap { trim: false })
        .render(inner, frame.buffer_mut());
}

fn render_welcome_page(frame: &mut Frame, state: &OnboardingDialogState, area: Rect) {
    use crate::overlays::{render_dark_overlay, render_dialog_bg, COVEN_CODE_PANEL_BG};

    let pink = Color::Rgb(139, 92, 246);
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
                " Welcome to Coven Code",
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
            "  Coven Code is an AI-powered coding assistant in your terminal.",
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
            "  Coven Code can read, edit, and create files in your project.",
            Style::default().fg(text),
        )),
        Line::from(Span::styled(
            "  Coven Code can run bash commands, search the web, and more.",
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

    let pink = Color::Rgb(139, 92, 246);
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
        kb("Enter", "send message"),
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
        kb("Alt+H", "open help"),
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
        let content: String = terminal
            .backend()
            .buffer()
            .clone()
            .content()
            .iter()
            .map(|c| c.symbol().chars().next().unwrap_or(' '))
            .collect();
        // Free Mode hint is present and precedes every provider.
        let free = content.find("Free Mode").expect("Free Mode hint missing");
        assert!(content.contains("/connect"));
        // Neutral ordering: no-key local provider first, then alphabetical.
        let positions: Vec<usize> = ["Ollama", "Anthropic", "Google", "Groq", "OpenAI"]
            .iter()
            .map(|name| {
                content
                    .find(name)
                    .unwrap_or_else(|| panic!("{name} missing"))
            })
            .collect();
        assert!(free < positions[0], "Free Mode should lead the page");
        let mut sorted = positions.clone();
        sorted.sort_unstable();
        assert_eq!(positions, sorted, "providers out of neutral order");
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
