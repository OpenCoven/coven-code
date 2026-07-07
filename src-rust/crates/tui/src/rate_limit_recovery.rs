// rate_limit_recovery.rs — Interactive rate-limit recovery surface.
//
// Replaces the dead-end "API error 429" modal with an actionable flow: the
// diagnostic explains what happened, a live countdown auto-retries when the
// provider supplied a reset delay, and one-key actions let the user continue
// immediately on a lower model tier, retry now, or clean up duplicate account
// profiles — all without quitting the session or retyping the prompt.
//
// The state machine lives here; key handling is in `app.rs` and the retry
// itself is executed by the interactive main loop in the CLI crate, which
// polls [`RateLimitRecoveryState::take_retry_directive`] every frame.

use std::time::{Duration, Instant};

use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};
use ratatui::Frame;

/// Maximum reset delay we will honour with an automatic countdown. Longer
/// waits (e.g. "resets in 4 hours") only offer manual actions — silently
/// camping on an hours-long timer is not a happy path.
const MAX_AUTO_RETRY_DELAY: Duration = Duration::from_secs(15 * 60);

/// Minimum countdown so an auto-retry never fires before the user can read
/// the modal.
const MIN_AUTO_RETRY_DELAY: Duration = Duration::from_secs(5);

/// Cap on consecutive automatic retries within one rate-limit episode. Manual
/// actions (retry key, model switch) stay available past this point.
const MAX_AUTO_RETRIES: u32 = 3;

/// Anthropic model ids for the one-key tier-switch actions.
pub const SONNET_MODEL: &str = "claude-sonnet-4-6";
pub const HAIKU_MODEL: &str = "claude-haiku-4-5-20251001";

/// What the main loop should do when it picks up a retry directive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetryDirective {
    /// Model to retry on (`None` = same model). The model switch has already
    /// been applied to the app config by the key handler; this field is
    /// informational for status messages.
    pub model: Option<String>,
}

/// State for the rate-limit recovery modal.
#[derive(Debug, Clone, Default)]
pub struct RateLimitRecoveryState {
    /// Whether the recovery modal is visible.
    pub visible: bool,
    /// Full diagnostic text (enriched notice from the query layer).
    pub message: String,
    /// The model that hit the limit.
    pub model: String,
    /// Provider-stated reset delay, when known.
    pub retry_after_secs: Option<u64>,
    /// Deadline for the automatic retry countdown (None = manual only).
    pub auto_retry_deadline: Option<Instant>,
    /// Automatic retries already spent in this episode.
    pub auto_retries_used: u32,
    /// Redundant duplicate profiles detected in the account registry
    /// (same underlying account imported more than once).
    pub duplicate_profiles: usize,
    /// Whether the one-key Anthropic tier-switch actions (`s`/`h`) apply.
    /// False when the active provider is not Anthropic — switching a Codex
    /// session to Sonnet/Haiku would persist a broken provider config.
    pub tier_switch_available: bool,
    /// Retry directive waiting to be consumed by the main loop.
    pending_retry: Option<RetryDirective>,
}

impl RateLimitRecoveryState {
    /// Open the recovery modal for a fresh rate-limit error.
    ///
    /// `duplicate_profiles` is precomputed by the caller (it requires disk
    /// I/O, which must not happen during rendering).
    pub fn open(
        &mut self,
        message: String,
        model: String,
        retry_after_secs: Option<u64>,
        duplicate_profiles: usize,
        tier_switch_available: bool,
    ) {
        self.visible = true;
        self.message = message;
        self.model = model;
        self.retry_after_secs = retry_after_secs;
        self.duplicate_profiles = duplicate_profiles;
        self.tier_switch_available = tier_switch_available;
        self.pending_retry = None;
        self.auto_retry_deadline = retry_after_secs
            .map(Duration::from_secs)
            .filter(|d| *d <= MAX_AUTO_RETRY_DELAY)
            .filter(|_| self.auto_retries_used < MAX_AUTO_RETRIES)
            .map(|d| Instant::now() + d.max(MIN_AUTO_RETRY_DELAY));
    }

    /// Dismiss the modal and cancel any armed auto-retry. The prompt is
    /// immediately usable again.
    pub fn dismiss(&mut self) {
        self.visible = false;
        self.auto_retry_deadline = None;
        self.pending_retry = None;
    }

    /// A turn completed successfully — reset the episode so future rate
    /// limits get a fresh auto-retry budget.
    pub fn reset_episode(&mut self) {
        self.auto_retries_used = 0;
    }

    /// Request an immediate retry (manual action). Closes the modal.
    pub fn request_retry(&mut self, model: Option<String>) {
        self.pending_retry = Some(RetryDirective { model });
        self.visible = false;
        self.auto_retry_deadline = None;
    }

    /// Advance the countdown. When the deadline passes, converts it into a
    /// pending retry (counting against the auto-retry budget). Call once per
    /// frame; cheap and I/O-free.
    pub fn tick(&mut self) {
        if !self.visible {
            return;
        }
        if let Some(deadline) = self.auto_retry_deadline {
            if Instant::now() >= deadline {
                self.auto_retries_used += 1;
                self.request_retry(None);
            }
        }
    }

    /// Take the retry directive, if one is ready. Consumed by the main loop.
    pub fn take_retry_directive(&mut self) -> Option<RetryDirective> {
        self.pending_retry.take()
    }

    /// Seconds remaining on the countdown, if armed.
    pub fn countdown_secs(&self) -> Option<u64> {
        self.auto_retry_deadline
            .map(|d| d.saturating_duration_since(Instant::now()).as_secs())
    }

    /// Whether the limited model is already at the cheapest tier.
    fn is_haiku(&self) -> bool {
        self.model.contains("haiku")
    }

    fn is_sonnet(&self) -> bool {
        self.model.contains("sonnet")
    }
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

/// Render the recovery modal. Anchored like the error modal: over the footer
/// area when available, centered on the welcome screen.
pub fn render_rate_limit_recovery(
    frame: &mut Frame,
    area: Rect,
    state: &RateLimitRecoveryState,
    footer_area: Rect,
    is_welcome_screen: bool,
) {
    if !state.visible {
        return;
    }

    let anchored_in_welcome_box = footer_area.width > 0 && footer_area.y < 12;
    let modal_area = if is_welcome_screen || anchored_in_welcome_box || footer_area.width == 0 {
        let modal_width = (area.width * 3 / 4).max(50).min(area.width);
        let modal_height = (area.height / 2).max(14).min(area.height.saturating_sub(2));
        Rect {
            x: area.x + (area.width.saturating_sub(modal_width)) / 2,
            y: area.y + (area.height.saturating_sub(modal_height)) / 2,
            width: modal_width,
            height: modal_height,
        }
    } else {
        let desired_height = (area.height / 2)
            .max(14)
            .min(area.height.saturating_sub(footer_area.y));
        Rect {
            x: footer_area.x,
            y: footer_area.y,
            width: footer_area.width,
            height: desired_height,
        }
    };

    frame.render_widget(Clear, modal_area);

    let accent = Color::Yellow;
    let modal_block = Block::default()
        .borders(Borders::ALL)
        .border_type(ratatui::widgets::BorderType::Rounded)
        .style(Style::default().fg(accent));
    frame.render_widget(modal_block, modal_area);

    let header_area = Rect {
        x: modal_area.x + 1,
        y: modal_area.y + 1,
        width: modal_area.width.saturating_sub(2),
        height: 1,
    };
    let header = Paragraph::new("  ⚡ Rate limited — recovery  ").style(
        Style::default()
            .bg(Color::Rgb(60, 45, 10))
            .fg(accent)
            .add_modifier(Modifier::BOLD),
    );
    frame.render_widget(header, header_area);

    let sep_area = Rect {
        x: modal_area.x + 1,
        y: modal_area.y + 2,
        width: modal_area.width.saturating_sub(2),
        height: 1,
    };
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            "─".repeat(sep_area.width as usize),
            Style::default().fg(Color::Rgb(80, 65, 20)),
        ))),
        sep_area,
    );

    // Action list is pinned to the bottom of the modal; the diagnostic body
    // wraps in whatever space remains above it.
    let actions = action_lines(state);
    let actions_height = actions.len() as u16;
    let body_top = modal_area.y + 3;
    let body_bottom = (modal_area.y + modal_area.height)
        .saturating_sub(1) // bottom border
        .saturating_sub(actions_height)
        .saturating_sub(1); // blank line above actions
    let body_area = Rect {
        x: modal_area.x + 2,
        y: body_top,
        width: modal_area.width.saturating_sub(4),
        height: body_bottom.saturating_sub(body_top).max(1),
    };
    frame.render_widget(
        Paragraph::new(state.message.as_str())
            .style(Style::default().fg(Color::Rgb(220, 220, 220)))
            .wrap(Wrap { trim: true }),
        body_area,
    );

    let actions_area = Rect {
        x: modal_area.x + 2,
        y: body_bottom + 1,
        width: modal_area.width.saturating_sub(4),
        height: actions_height.min(modal_area.height.saturating_sub(4)),
    };
    frame.render_widget(Paragraph::new(actions), actions_area);
}

/// Build the action-hint lines shown at the bottom of the modal.
fn action_lines(state: &RateLimitRecoveryState) -> Vec<Line<'static>> {
    let key_style = Style::default()
        .fg(Color::Yellow)
        .add_modifier(Modifier::BOLD);
    let text_style = Style::default().fg(Color::Rgb(200, 200, 200));
    let dim = Style::default().fg(Color::DarkGray);

    let mut lines: Vec<Line<'static>> = Vec::new();

    if let Some(secs) = state.countdown_secs() {
        lines.push(Line::from(vec![
            Span::styled("⟳ ", key_style),
            Span::styled(
                format!(
                    "Auto-retrying in {} — the session picks up right where it left off.",
                    claurst_query::format_reset_delay(secs.max(1))
                ),
                Style::default()
                    .fg(Color::Rgb(120, 220, 120))
                    .add_modifier(Modifier::BOLD),
            ),
        ]));
    }

    let mut keys: Vec<Span<'static>> = vec![Span::styled("[r]", key_style)];
    keys.push(Span::styled(" retry now   ", text_style));
    if state.tier_switch_available {
        if !state.is_sonnet() && !state.is_haiku() {
            keys.push(Span::styled("[s]", key_style));
            keys.push(Span::styled(" continue on Sonnet   ", text_style));
        }
        if !state.is_haiku() {
            keys.push(Span::styled("[h]", key_style));
            keys.push(Span::styled(" continue on Haiku", text_style));
        }
    }
    lines.push(Line::from(keys));

    if state.duplicate_profiles > 0 {
        lines.push(Line::from(vec![
            Span::styled("[d]", key_style),
            Span::styled(
                format!(
                    " clean {} duplicate account profile{} (all the same account — switching between them cannot help)",
                    state.duplicate_profiles,
                    if state.duplicate_profiles == 1 { "" } else { "s" }
                ),
                text_style,
            ),
        ]));
    }

    lines.push(Line::from(vec![
        Span::styled("[Esc]", key_style),
        Span::styled(" dismiss — keep typing; nothing is lost", dim),
    ]));

    lines
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_arms_countdown_for_short_delays() {
        let mut s = RateLimitRecoveryState::default();
        s.open(
            "limited".into(),
            "claude-opus-4-8".into(),
            Some(30),
            0,
            true,
        );
        assert!(s.visible);
        assert!(s.auto_retry_deadline.is_some());
        assert!(s.countdown_secs().unwrap() <= 30);
    }

    #[test]
    fn open_does_not_arm_countdown_for_long_delays() {
        let mut s = RateLimitRecoveryState::default();
        s.open(
            "limited".into(),
            "claude-opus-4-8".into(),
            Some(3600),
            0,
            true,
        );
        assert!(s.visible);
        assert!(s.auto_retry_deadline.is_none());
    }

    #[test]
    fn open_does_not_arm_countdown_without_retry_after() {
        let mut s = RateLimitRecoveryState::default();
        s.open("limited".into(), "claude-opus-4-8".into(), None, 0, true);
        assert!(s.auto_retry_deadline.is_none());
    }

    #[test]
    fn auto_retry_budget_is_capped() {
        let mut s = RateLimitRecoveryState {
            auto_retries_used: MAX_AUTO_RETRIES,
            ..Default::default()
        };
        s.open(
            "limited".into(),
            "claude-opus-4-8".into(),
            Some(10),
            0,
            true,
        );
        assert!(
            s.auto_retry_deadline.is_none(),
            "budget exhausted — no more auto-retries"
        );
        s.reset_episode();
        s.open(
            "limited".into(),
            "claude-opus-4-8".into(),
            Some(10),
            0,
            true,
        );
        assert!(s.auto_retry_deadline.is_some());
    }

    #[test]
    fn tick_fires_retry_at_deadline() {
        let mut s = RateLimitRecoveryState::default();
        s.open(
            "limited".into(),
            "claude-opus-4-8".into(),
            Some(10),
            0,
            true,
        );
        // Force the deadline into the past.
        s.auto_retry_deadline = Some(Instant::now() - Duration::from_secs(1));
        s.tick();
        assert!(!s.visible);
        assert_eq!(s.auto_retries_used, 1);
        let directive = s.take_retry_directive().expect("retry queued");
        assert_eq!(directive.model, None);
        assert!(s.take_retry_directive().is_none(), "directive is consumed");
    }

    #[test]
    fn manual_retry_with_model_switch() {
        let mut s = RateLimitRecoveryState::default();
        s.open(
            "limited".into(),
            "claude-opus-4-8".into(),
            Some(3600),
            0,
            true,
        );
        s.request_retry(Some(HAIKU_MODEL.to_string()));
        assert!(!s.visible);
        let directive = s.take_retry_directive().expect("retry queued");
        assert_eq!(directive.model.as_deref(), Some(HAIKU_MODEL));
    }

    #[test]
    fn dismiss_cancels_everything() {
        let mut s = RateLimitRecoveryState::default();
        s.open(
            "limited".into(),
            "claude-opus-4-8".into(),
            Some(10),
            0,
            true,
        );
        s.dismiss();
        assert!(!s.visible);
        s.tick();
        assert!(s.take_retry_directive().is_none());
    }

    #[test]
    fn tier_actions_reflect_current_model() {
        let mut s = RateLimitRecoveryState::default();
        s.open("limited".into(), "claude-opus-4-8".into(), None, 0, true);
        let text = lines_text(&action_lines(&s));
        assert!(text.contains("[s]"));
        assert!(text.contains("[h]"));

        s.model = SONNET_MODEL.to_string();
        let text = lines_text(&action_lines(&s));
        assert!(!text.contains("[s]"));
        assert!(text.contains("[h]"));

        s.model = HAIKU_MODEL.to_string();
        let text = lines_text(&action_lines(&s));
        assert!(!text.contains("[s]"));
        assert!(!text.contains("[h]"));
    }

    #[test]
    fn tier_actions_hidden_for_non_anthropic_provider() {
        let mut s = RateLimitRecoveryState::default();
        s.open("limited".into(), "gpt-5-codex".into(), None, 0, false);
        let text = lines_text(&action_lines(&s));
        assert!(!text.contains("[s]"));
        assert!(!text.contains("[h]"));
        assert!(text.contains("[r]"));
    }

    #[test]
    fn duplicate_cleanup_action_appears_when_duplicates_exist() {
        let mut s = RateLimitRecoveryState::default();
        s.open("limited".into(), "claude-opus-4-8".into(), None, 18, true);
        let text = lines_text(&action_lines(&s));
        assert!(text.contains("[d]"));
        assert!(text.contains("18 duplicate account profiles"));

        s.duplicate_profiles = 0;
        let text = lines_text(&action_lines(&s));
        assert!(!text.contains("[d]"));
    }

    #[test]
    fn render_smoke() {
        let mut s = RateLimitRecoveryState::default();
        s.open(
            "Claude rate limit for model `claude-opus-4-8`.".into(),
            "claude-opus-4-8".into(),
            Some(30),
            2,
            true,
        );
        let area = Rect {
            x: 0,
            y: 0,
            width: 80,
            height: 24,
        };
        let backend = ratatui::backend::TestBackend::new(80, 24);
        let mut terminal = ratatui::Terminal::new(backend).expect("terminal");
        terminal
            .draw(|frame| {
                render_rate_limit_recovery(frame, area, &s, Rect::default(), true);
            })
            .expect("draw");
        let buffer = terminal.backend().buffer();
        let rendered = buffer
            .content
            .iter()
            .map(|c| c.symbol())
            .collect::<Vec<_>>()
            .join("");
        assert!(rendered.contains("Rate limited"));
        assert!(rendered.contains("retry now"));
        assert!(rendered.contains("duplicate account profile"));
    }

    fn lines_text(lines: &[Line<'_>]) -> String {
        lines
            .iter()
            .map(|l| {
                l.spans
                    .iter()
                    .map(|s| s.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}
