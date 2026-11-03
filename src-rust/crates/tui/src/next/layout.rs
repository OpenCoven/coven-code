//! Cell layout for the Next shell.
//!
//! One pure function maps the terminal size and the inspector request to
//! regions, following the thresholds accepted in the TUI v2 design (#421):
//!
//! | Cells | Rule |
//! |---|---|
//! | width < 80 or height < 24 | Focus: the inspector replaces the main pane |
//! | otherwise, width < 120 or height < 32 | Single: the inspector replaces the main pane |
//! | width >= 120 and height >= 32 | Split: transcript, separator and inspector side by side |
//!
//! Rows are given out in priority order so small terminals degrade
//! predictably: composer, status, header, composer rule, main pane, hints.

use ratatui::layout::Rect;

/// Narrowest transcript column in Split; the inspector gets the rest.
const SPLIT_TRANSCRIPT_MIN: u16 = 72;
const INSPECTOR_MIN: u16 = 40;
const INSPECTOR_MAX: u16 = 60;
/// Main pane rows kept clear before the optional hint row is shown.
const MAIN_ROWS_BEFORE_HINTS: u16 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SizeClass {
    Focus,
    Single,
    Split,
}

impl SizeClass {
    pub fn of(area: Rect) -> Self {
        if area.width < 80 || area.height < 24 {
            SizeClass::Focus
        } else if area.width < 120 || area.height < 32 {
            SizeClass::Single
        } else {
            SizeClass::Split
        }
    }
}

/// Regions of one Next frame. Zero-height regions are not drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShellLayout {
    pub size_class: SizeClass,
    pub header: Rect,
    /// `None` when the inspector replaces the main pane.
    pub transcript: Option<Rect>,
    /// Column between transcript and inspector in Split.
    pub separator: Option<Rect>,
    pub inspector: Option<Rect>,
    pub status: Rect,
    pub hints: Option<Rect>,
    /// Rule above the composer; dropped first on very short terminals.
    pub rule: Option<Rect>,
    pub composer: Rect,
}

/// Lay out a frame. `composer_rows` is the wrapped draft height; it is
/// capped at a third of the screen, but never below one row.
pub fn plan(area: Rect, inspector_open: bool, hint_rows: u16, composer_rows: u16) -> ShellLayout {
    let size_class = SizeClass::of(area);
    // Each row is taken from what is left, so nothing can underflow.
    fn take(remaining: &mut u16, wanted: u16) -> u16 {
        let given = wanted.min(*remaining);
        *remaining -= given;
        given
    }
    let mut remaining = area.height;
    let composer = take(
        &mut remaining,
        composer_rows.max(1).min((area.height / 3).max(1)),
    );
    let status = take(&mut remaining, 1);
    // Header and rule each need a main row left over to be worth showing.
    let wanted = u16::from(remaining >= 2);
    let header = take(&mut remaining, wanted);
    let wanted = u16::from(remaining >= 2);
    let rule = take(&mut remaining, wanted);
    let hints_wanted = if remaining >= hint_rows + MAIN_ROWS_BEFORE_HINTS && area.height >= 12 {
        hint_rows
    } else {
        0
    };
    let hints = take(&mut remaining, hints_wanted);
    let main = remaining;

    let row = |y: u16, height: u16| Rect::new(area.x, y, area.width, height);
    let header_rect = row(area.y, header);
    let main_rect = row(area.y + header, main);
    let status_rect = row(main_rect.y + main, status);
    let hints_rect = row(status_rect.y + status, hints);
    let rule_rect = row(hints_rect.y + hints, rule);
    let composer_rect = row(rule_rect.y + rule, composer);

    let (transcript, separator, inspector) = match (inspector_open, size_class) {
        (false, _) => (Some(main_rect), None, None),
        (true, SizeClass::Split) => {
            let inspector_width = area
                .width
                .saturating_sub(SPLIT_TRANSCRIPT_MIN + 1)
                .clamp(INSPECTOR_MIN, INSPECTOR_MAX);
            let transcript_width = area.width - inspector_width - 1;
            let at = |x: u16, width: u16| Rect::new(x, main_rect.y, width, main_rect.height);
            (
                Some(at(area.x, transcript_width)),
                Some(at(area.x + transcript_width, 1)),
                Some(at(area.x + transcript_width + 1, inspector_width)),
            )
        }
        (true, SizeClass::Focus | SizeClass::Single) => (None, None, Some(main_rect)),
    };

    ShellLayout {
        size_class,
        header: header_rect,
        transcript,
        separator,
        inspector,
        status: status_rect,
        hints: (hints > 0).then_some(hints_rect),
        rule: (rule > 0).then_some(rule_rect),
        composer: composer_rect,
    }
}
