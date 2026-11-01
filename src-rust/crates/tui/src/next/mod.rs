//! Next renderer (opt-in preview, `COVEN_CODE_UI=next`).
//!
//! Renders the shared [`TranscriptView`] projection as the T3 shell: a
//! workspace line, the transcript, a status row and a persistent multiline
//! composer, plus one optional inspector laid out by [`layout`]. It reads the
//! same [`App`] state that Classic reads and receives input through the same
//! handlers, so selecting it never changes what the runtime executes,
//! persists or authorizes.
//!
//! Until #411 delivers Next surfaces for them, any frame in which a surface
//! owns input (permission request, overlays, pickers, dialogs, the familiar
//! switcher, slash suggestions) or an error modal is showing is drawn
//! entirely by Classic, so no choice that holds input focus is ever invisible
//! or partially drawn. The command palette and `/` suggestions are therefore
//! the existing ones.

pub mod layout;
pub mod primitives;
pub mod style;

use crate::app::App;
use crate::application::view::{EntryRole, ToolActivity, ToolActivityState, TranscriptView};
use crate::application::TurnCancellation;
use claurst_core::appearance::Density;
use claurst_core::keybindings::{KeyContext, KeybindingResolver, UserKeybindings};
use primitives::Status;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};
use ratatui::Frame;
use std::cell::Cell;
use style::{Appearance, Role};

/// Header text after the identity mark; the canonical mark is SVG-only, so
/// cells use plain text.
pub const PREVIEW_LABEL: &str = "Coven · preview";

/// Content of the optional key-hint row, resolved through the configurable
/// keybinding system.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct KeyHints {
    /// `<keys> <label>` parts, joined with the appearance's separator.
    pub parts: Vec<String>,
    /// Essential actions no key reaches; shown after the hints.
    pub unreachable: Vec<&'static str>,
}

impl KeyHints {
    pub fn resolve(user: &UserKeybindings) -> Self {
        let resolver = KeybindingResolver::new(user);
        // Hints show the chord that reaches each action from the composer now.
        let parts = [
            ("submit", "send"),
            ("openCommandPalette", "commands"),
            ("openHelp", "help"),
        ]
        .into_iter()
        .filter_map(|(action, label)| {
            resolver
                .reachable_chord(action)
                .map(|keys| format!("{keys} {label}"))
        })
        .collect();
        Self {
            parts,
            unreachable: resolver.unreachable_essential_actions(),
        }
    }
}

/// One line per composer chord whose final effect differs from its default.
pub fn key_conflict_notices(user: &UserKeybindings) -> Vec<String> {
    KeybindingResolver::new(user)
        .conflicts()
        .into_iter()
        .map(|conflict| {
            format!(
                "Key {} now {} (was {})",
                conflict.chord,
                conflict.by.as_deref().unwrap_or("unbound"),
                conflict.replaced
            )
        })
        .collect()
}

/// Presentation-only preferences for Next, resolved once at startup.
#[derive(Debug, Clone, Default)]
pub struct NextPresentation {
    pub appearance: Appearance,
    pub density: Density,
    /// `None` when the hint row is turned off.
    pub hints: Option<KeyHints>,
}

impl NextPresentation {
    pub fn from_settings(
        settings: &claurst_core::config::Settings,
        user_keybindings: &UserKeybindings,
        env: impl Fn(&str) -> Option<String>,
    ) -> Self {
        let appearance_settings = &settings.next_appearance;
        Self {
            appearance: Appearance::resolve(appearance_settings, settings.reduce_motion, env),
            density: appearance_settings.density,
            hints: appearance_settings
                .hints
                .then(|| KeyHints::resolve(user_keybindings)),
        }
    }
}

/// Whether this frame must be drawn by Classic because a surface Next does
/// not render yet holds input focus or demands attention.
///
/// `any_modal_open` covers every input-owning surface in
/// `App::handle_key_event` except the familiar switcher and effort picker,
/// which are checked here without changing the list Classic's Enter gating
/// relies on.
pub fn delegates_to_classic(app: &App) -> bool {
    // Next draws a pending permission request itself (#411) unless another
    // surface is open too, in which case Classic's precedence applies.
    app.modal_open_besides_permission()
        || app.familiar_switcher_open
        || app.effort_picker.visible
        || app.notifications.current_is_error()
        || !app.prompt_input.suggestions.is_empty()
}

/// Pages of the single Next inspector, in Tab order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InspectorPage {
    /// Tool activity and turn state of the current turn.
    Activity,
    /// Workspace, session, model and credential context.
    Session,
    /// Engine-local background tasks, model tasks and the session goal.
    Work,
    /// Destinations of the shared navigation model and their availability.
    Views,
}

impl InspectorPage {
    pub const ALL: [InspectorPage; 4] = [
        InspectorPage::Activity,
        InspectorPage::Session,
        InspectorPage::Work,
        InspectorPage::Views,
    ];

    pub fn title(self) -> &'static str {
        match self {
            InspectorPage::Activity => "Activity",
            InspectorPage::Session => "Session",
            InspectorPage::Work => "Work",
            InspectorPage::Views => "Views",
        }
    }

    fn step(self, forward: bool) -> Self {
        let count = Self::ALL.len();
        let index = Self::ALL.iter().position(|page| *page == self).unwrap_or(0);
        let next = if forward {
            (index + 1) % count
        } else {
            (index + count - 1) % count
        };
        Self::ALL[next]
    }
}

/// Keybinding actions only Next handles. Classic leaves their chords
/// unbound, so its behavior does not change.
pub const NEXT_ONLY_ACTIONS: &[&str] = &[
    "toggleInspector",
    "closeInspector",
    "nextInspectorPage",
    "previousInspectorPage",
    "scrollInspectorUp",
    "scrollInspectorDown",
    "jumpToLive",
];

const INSPECTOR_SCROLL_STEP: usize = 5;

/// Transcript rows measured by the last Next frame, used to keep a scrolled
/// reader on the same content while rows arrive, disappear or rewrap.
#[derive(Debug, Clone, Copy, Default)]
struct ReadingAnchor {
    width: u16,
    total: usize,
    /// Row offset from the bottom where the reader actually is, not yet
    /// folded into `App::scroll_offset` (may be negative).
    unsettled: i64,
    /// Rows that arrived since the reader scrolled away.
    unseen: usize,
}

#[derive(Debug, Default)]
pub struct ShellState {
    /// The one open inspector page, if any. It owns Escape and Tab; the
    /// composer keeps every other key, so the draft is never stranded.
    pub inspector: Option<InspectorPage>,
    pub inspector_scroll: usize,
    /// Largest useful `inspector_scroll` for the last drawn page.
    inspector_scroll_max: Cell<usize>,
    anchor: Cell<ReadingAnchor>,
}

impl ShellState {
    /// Apply one of [`NEXT_ONLY_ACTIONS`].
    pub fn apply_action(&mut self, action: &str) {
        match action {
            "toggleInspector" => {
                self.inspector = match self.inspector {
                    Some(_) => None,
                    None => Some(InspectorPage::Activity),
                };
                self.inspector_scroll = 0;
            }
            "closeInspector" => {
                self.inspector = None;
                self.inspector_scroll = 0;
            }
            "nextInspectorPage" | "previousInspectorPage" => {
                if let Some(page) = self.inspector {
                    self.inspector = Some(page.step(action == "nextInspectorPage"));
                    self.inspector_scroll = 0;
                }
            }
            "scrollInspectorUp" => {
                self.inspector_scroll = self.inspector_scroll.saturating_sub(INSPECTOR_SCROLL_STEP);
            }
            "scrollInspectorDown" => {
                self.inspector_scroll = self
                    .inspector_scroll
                    .saturating_add(INSPECTOR_SCROLL_STEP)
                    .min(self.inspector_scroll_max.get());
            }
            "jumpToLive" => self.anchor.set(ReadingAnchor::default()),
            _ => {}
        }
    }

    /// Signed row correction the loop adds to `App::scroll_offset`.
    pub(crate) fn take_unsettled_rows(&self) -> i64 {
        let mut anchor = self.anchor.get();
        let rows = std::mem::take(&mut anchor.unsettled);
        self.anchor.set(anchor);
        rows
    }

    /// First visible row of a `total`-row transcript in a `height`-row pane,
    /// and how many rows arrived since the reader scrolled away.
    ///
    /// `scroll_offset` counts rows above the bottom, so rows added or removed
    /// below a scrolled reader would move their text. Those changes are
    /// added to the offset until the loop settles them. A width change
    /// rewraps everything, so the offset is scaled to the new row count; the
    /// result is clamped to the scrollable range so later steps take effect
    /// immediately.
    fn window(
        &self,
        total: usize,
        height: usize,
        width: u16,
        auto_scroll: bool,
        scroll_offset: usize,
    ) -> (usize, usize) {
        let mut anchor = self.anchor.get();
        let max_scroll = total.saturating_sub(height);
        let offset = if auto_scroll {
            anchor = ReadingAnchor {
                width,
                total,
                ..ReadingAnchor::default()
            };
            0
        } else {
            let as_rows = |n: usize| i64::try_from(n).unwrap_or(i64::MAX);
            let current = as_rows(scroll_offset).saturating_add(anchor.unsettled);
            let wanted = if anchor.total == 0 {
                // Nothing measured yet: take the offset as given.
                current
            } else if anchor.width == width {
                anchor.unseen += total.saturating_sub(anchor.total);
                current.saturating_add(as_rows(total) - as_rows(anchor.total))
            } else {
                current.saturating_mul(as_rows(total)) / as_rows(anchor.total)
            };
            let clamped = wanted.clamp(0, as_rows(max_scroll));
            anchor.unsettled = clamped - as_rows(scroll_offset);
            anchor.width = width;
            anchor.total = total;
            usize::try_from(clamped).unwrap_or(0)
        };
        self.anchor.set(anchor);
        (max_scroll - offset.min(max_scroll), anchor.unseen)
    }
}

pub fn render_next(frame: &mut Frame, app: &App) {
    if delegates_to_classic(app) {
        crate::render::render_app(frame, app);
        return;
    }
    // Next registers only the transcript as a selection target. Clear the
    // per-message maps an earlier Classic frame recorded so right-click and
    // expansion hit-tests never target Classic's stale layout.
    app.last_input_area.set(Rect::default());
    app.footer_right_column_area.set(Rect::default());
    app.thinking_row_map.borrow_mut().clear();
    app.message_row_map.borrow_mut().clear();

    let presentation = &app.next_presentation;
    let appearance = &presentation.appearance;
    let view = TranscriptView::project(app);
    let area = frame.area();
    let width = usize::from(area.width.max(1));

    let draft_lines: Vec<Line<'static>> = primitives::composer(appearance, &view.draft)
        .into_iter()
        .flat_map(|line| wrap_line(line, width))
        .collect();
    let draft_rows = draft_lines.len();
    let mut composer: Vec<Line<'static>> = draft_lines;
    composer.extend(view.draft_attachments.iter().flat_map(|label| {
        wrap_line(
            Line::from(Span::styled(
                format!("  {label}"),
                appearance.style(Role::Evidence),
            )),
            width,
        )
    }));
    let hint_rows = u16::from(presentation.hints.is_some());
    // A pending approval takes the whole main pane at every size; the
    // inspector keeps its state and returns once the request is decided.
    let approval = view.permission.as_ref();
    let layout = layout::plan(
        area,
        app.next_shell.inspector.is_some() && approval.is_none(),
        hint_rows,
        u16::try_from(composer.len()).unwrap_or(u16::MAX),
    );

    frame.render_widget(
        Paragraph::new(workspace_line(appearance, app, area.width)),
        layout.header,
    );
    if let (Some(request), Some(main)) = (approval, layout.transcript) {
        let panel = approval_lines(appearance, app, request);
        let lines = fit_approval(appearance, panel, width, usize::from(main.height));
        frame.render_widget(Paragraph::new(lines), main);
    }
    match layout.transcript.filter(|_| approval.is_none()) {
        Some(transcript) => {
            app.last_msg_area.set(transcript);
            app.last_selectable_area.set(transcript);
            render_transcript(frame, app, &view, transcript);
            crate::render::cache_selectable_row_text(frame, app);
            crate::render::apply_selection_highlight(frame, app);
        }
        None => {
            app.last_msg_area.set(Rect::default());
            app.last_selectable_area.set(Rect::default());
            app.last_row_text.borrow_mut().clear();
        }
    }
    if let Some(separator) = layout.separator {
        let glyph = primitives::Mark::Divider.glyph(appearance.glyphs);
        let column: Vec<Line<'static>> = (0..separator.height)
            .map(|_| Line::from(Span::styled(glyph, appearance.style(Role::Border))))
            .collect();
        frame.render_widget(Paragraph::new(column), separator);
    }
    if let (Some(page), Some(inspector)) = (app.next_shell.inspector, layout.inspector) {
        render_inspector(frame, app, &view, page, inspector);
    }
    frame.render_widget(
        Paragraph::new(status_line(appearance, app, &view)),
        layout.status,
    );
    if let (Some(hints), Some(rect)) = (&presentation.hints, layout.hints) {
        frame.render_widget(Paragraph::new(hints_line(appearance, hints)), rect);
    }
    if let Some(rule) = layout.rule {
        frame.render_widget(
            Paragraph::new(composer_rule(appearance, &view, rule.width)),
            rule,
        );
    }

    // When the draft outgrows its rows, keep the newest rows unless that
    // would hide the cursor; then scroll just far enough to show it.
    let (row, column) = composer_cursor(&view.draft, view.draft_cursor, width);
    let visible = usize::from(layout.composer.height);
    let mut first = composer.len().saturating_sub(visible);
    if row < draft_rows {
        first = first.min(row);
        if row >= first + visible {
            first = row + 1 - visible;
        }
    }
    composer.drain(..first);
    frame.render_widget(Paragraph::new(composer), layout.composer);
    if row < draft_rows && row >= first {
        if let (Ok(y), Ok(x)) = (u16::try_from(row - first), u16::try_from(column)) {
            if y < layout.composer.height && x < area.width {
                frame.set_cursor_position((layout.composer.x + x, layout.composer.y + y));
            }
        }
    }

    if !app.notifications.is_empty() {
        crate::notifications::render_notification_banner(frame, &app.notifications, area);
    }
}

/// Cells the composer prompt glyph and its following space occupy; the
/// continuation lead is padded to the same width.
const COMPOSER_LEAD: usize = 2;

/// Wrapped (row, column) of the cursor at byte `cursor` of `draft`, using
/// the same cell-width wrapping as [`wrap_line`].
fn composer_cursor(draft: &str, cursor: usize, width: usize) -> (usize, usize) {
    // Walk one composer line the way `wrap_line` breaks it: a character that
    // would cross the edge starts the next row.
    let walk = |text: &str| {
        let mut row = 0;
        let mut used = COMPOSER_LEAD.min(width);
        for ch in text.chars() {
            let cells = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
            if used + cells > width && used > 0 {
                row += 1;
                used = 0;
            }
            used += cells;
        }
        (row, used)
    };
    let cursor = cursor.min(draft.len());
    let before = draft.get(..cursor).unwrap_or(draft);
    let line_start = before.rfind('\n').map_or(0, |index| index + 1);
    let earlier_rows: usize = draft[..line_start]
        .split_terminator('\n')
        .map(|line| walk(line).0 + 1)
        .sum();
    let (row, used) = walk(&before[line_start..]);
    // A full row keeps the cursor on its last cell rather than on a row
    // that belongs to the next line.
    (earlier_rows + row, used.min(width.saturating_sub(1)))
}

fn project_name(app: &App) -> String {
    let root = app
        .config
        .project_dir
        .clone()
        .or_else(|| std::env::current_dir().ok());
    root.as_deref()
        .and_then(std::path::Path::file_name)
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "no project".to_string())
}

fn model_label(app: &App) -> Option<String> {
    if !app.model_name.is_empty() {
        return Some(app.model_name.clone());
    }
    app.config.model.clone()
}

/// Header: product mark, then workspace context in priority order. Parts
/// that do not fit are dropped from the end rather than truncated.
fn workspace_line(appearance: &Appearance, app: &App, width: u16) -> Line<'static> {
    let separator = format!(" {} ", primitives::Mark::Separator.glyph(appearance.glyphs));
    let mut line = primitives::header(appearance, "preview");
    let mut parts = vec![(project_name(app), Role::Text)];
    parts.push((
        app.session_title
            .clone()
            .unwrap_or_else(|| "untitled session".to_string()),
        Role::TextMuted,
    ));
    if let Some(model) = model_label(app) {
        parts.push((model, Role::TextMuted));
    }
    if let Some(familiar) = &app.config.familiar {
        // A familiar id is a display name until #412 qualifies identity.
        parts.push((format!("{familiar} (unverified)"), Role::Evidence));
    }
    for (text, role) in parts {
        let cells =
            separator.chars().count() + unicode_width::UnicodeWidthStr::width(text.as_str());
        if line.width() + cells > usize::from(width) {
            break;
        }
        line.spans.push(Span::styled(
            separator.clone(),
            appearance.style(Role::TextSubtle),
        ));
        line.spans.push(Span::styled(text, appearance.style(role)));
    }
    line
}

fn transcript_lines(presentation: &NextPresentation, view: &TranscriptView) -> Vec<Line<'static>> {
    let appearance = &presentation.appearance;
    let text = |content: &str| {
        Line::from(Span::styled(
            content.to_string(),
            appearance.style(Role::Text),
        ))
    };
    let mut lines = Vec::new();
    for entry in &view.entries {
        lines.push(primitives::transcript_label(
            appearance,
            match entry.role {
                EntryRole::User => "you",
                EntryRole::Assistant => "coven",
            },
        ));
        lines.extend(entry.text.lines().map(text));
        lines.extend(entry.attachments.iter().map(|label| {
            Line::from(Span::styled(
                label.clone(),
                appearance.style(Role::Evidence),
            ))
        }));
        if presentation.density == Density::Comfortable {
            lines.push(Line::default());
        }
    }
    lines.extend(
        view.tools
            .iter()
            .map(|tool| tool_line(appearance, tool, false)),
    );
    if !view.live_text.is_empty() {
        lines.push(primitives::transcript_label(appearance, "coven"));
        lines.extend(view.live_text.lines().map(text));
    }
    lines
}

/// Completion is not verification: a finished tool is neutral evidence.
fn tool_status(state: ToolActivityState) -> (Status, &'static str) {
    match state {
        ToolActivityState::Running => (Status::Pending, "running"),
        ToolActivityState::Done => (Status::Evidence, "done"),
        ToolActivityState::Error => (Status::Blocked, "error"),
        ToolActivityState::CancelRequested => (Status::Pending, "cancel requested"),
        ToolActivityState::Cancelled => (Status::Blocked, "cancelled"),
    }
}

/// One tool row: operation, safe target and state. A permission decision
/// is shown next to the execution state, never in place of it: an allowed
/// call is still running until the tool reports its result.
fn tool_line(appearance: &Appearance, tool: &ToolActivity, expanded: bool) -> Line<'static> {
    let label = match &tool.target {
        Some(target) => format!("{} {target}", tool.name),
        None => tool.name.clone(),
    };
    let (status, word) = tool_status(tool.state);
    let (status, word) = match (tool.allowed, tool.state) {
        (Some(false), _) => (Status::Blocked, "denied".to_string()),
        (Some(true), ToolActivityState::Running) => (status, format!("allowed, {word}")),
        _ => (status, word.to_string()),
    };
    primitives::tool_row(appearance, &label, status, &word, expanded)
}

/// Rows of input shown per tool in the inspector before truncation.
const DETAIL_OUTPUT_LINES: usize = 6;
const DETAIL_INPUT_CHARS: usize = 300;

fn tool_detail(appearance: &Appearance, tool: &ToolActivity) -> Vec<Line<'static>> {
    let indent = |name: &str, value: String| {
        Line::from(vec![
            Span::styled(format!("    {name:<8}"), appearance.style(Role::TextMuted)),
            Span::styled(value, appearance.style(Role::Text)),
        ])
    };
    let mut input: String = tool.input.chars().take(DETAIL_INPUT_CHARS).collect();
    if tool.input.chars().count() > DETAIL_INPUT_CHARS {
        input.push_str(" … (truncated)");
    }
    let mut lines = vec![indent("input", input.replace('\n', " "))];
    if let Some(output) = &tool.output {
        let mut shown = output.lines();
        for (index, line) in shown.by_ref().take(DETAIL_OUTPUT_LINES).enumerate() {
            let name = if index == 0 { "output" } else { "" };
            lines.push(indent(name, line.to_string()));
        }
        let rest = shown.count();
        if rest > 0 {
            lines.push(indent("", format!("… {rest} more line(s) not shown")));
        }
    }
    lines.push(indent(
        "owner",
        "engine-local tool in this coven-code process".to_string(),
    ));
    lines
}

/// Next's approval surface. It replaces the main pane while a request is
/// pending, keeps the draft visible below it, focuses deny by default and
/// names who enforces the decision. Keys are still read by the loop's
/// permission handler; this only draws.
/// The parts of the approval panel, in screen order, before fitting.
struct ApprovalPanel {
    title: Line<'static>,
    /// Every line of the request's input; never silently shortened.
    preview: Vec<Line<'static>>,
    danger: Option<Line<'static>>,
    options: Vec<Line<'static>>,
    details: Vec<Line<'static>>,
}

/// Fit the approval panel into `height` rows. Options are never dropped;
/// then the danger note and title; then as much input as fits, with an
/// explicit count of hidden rows; then supporting details.
fn fit_approval(
    appearance: &Appearance,
    panel: ApprovalPanel,
    width: usize,
    height: usize,
) -> Vec<Line<'static>> {
    let wrap = |lines: Vec<Line<'static>>| -> Vec<Line<'static>> {
        lines
            .into_iter()
            .flat_map(|line| wrap_line(line, width))
            .collect()
    };
    let options = wrap(panel.options);
    let danger = wrap(panel.danger.into_iter().collect());
    let title = wrap(vec![panel.title]);
    let preview = wrap(panel.preview);
    let mut details = wrap(panel.details);

    let mut budget = height;
    let mut take = |rows: &[Line<'static>]| -> bool {
        if rows.len() <= budget {
            budget -= rows.len();
            true
        } else {
            false
        }
    };
    let show_options = take(&options);
    let show_danger = take(&danger);
    let show_title = take(&title);
    let shown_preview = if preview.len() <= budget {
        preview.len()
    } else {
        // Reserve one row to say how much of the input is hidden.
        budget.saturating_sub(1)
    };
    budget -= shown_preview.min(budget);
    let hidden = preview.len() - shown_preview;
    let marker = if hidden > 0 && budget > 0 {
        budget -= 1;
        wrap_line(
            primitives::status_message(
                appearance,
                Status::Blocked,
                &format!("{hidden} more input rows hidden"),
                "enlarge the terminal to read all of it before approving",
            ),
            width,
        )
        .into_iter()
        .take(1)
        .collect()
    } else {
        Vec::new()
    };
    details.truncate(budget);

    let mut lines = Vec::with_capacity(height);
    if show_title {
        lines.extend(title);
    }
    lines.extend(preview.into_iter().take(shown_preview));
    lines.extend(marker);
    if show_danger {
        lines.extend(danger);
    }
    if show_options {
        lines.extend(options);
    } else {
        // Too small for every option: keep the focused end, deny.
        let skip = options.len().saturating_sub(height);
        lines.extend(options.into_iter().skip(skip));
    }
    lines.extend(details);
    lines.truncate(height);
    lines
}

fn approval_lines(
    appearance: &Appearance,
    app: &App,
    request: &crate::application::view::PendingPermission,
) -> ApprovalPanel {
    let title = primitives::inspector_title(
        appearance,
        &format!("Approve {} (engine-local)", request.tool_name),
    );
    let preview = request
        .input_preview
        .iter()
        .flat_map(|preview| preview.lines())
        .map(|line| {
            Line::from(Span::styled(
                line.to_string(),
                appearance.style(Role::Sensitive),
            ))
        })
        .collect();
    let options: Vec<(char, &str)> = request
        .options
        .iter()
        .map(|(key, label)| (*key, label.as_str()))
        .collect();
    let options = primitives::confirmation(appearance, "", &options, request.selected_option, true)
        .into_iter()
        .skip(1)
        .collect();
    let danger = (!request.danger.is_empty()).then(|| {
        Line::from(Span::styled(
            request.danger.clone(),
            appearance.style(Role::Sensitive),
        ))
    });
    let mut details = Vec::new();
    let armed = app.permission_request.as_ref().is_none_or(|pending| {
        !pending.arming || pending.quiet_since.elapsed() >= crate::dialogs::PERMISSION_ARM_DELAY
    });
    if !armed {
        details.push(primitives::status_message(
            appearance,
            Status::Pending,
            "arming",
            "keys typed before this appeared are ignored",
        ));
    }
    if !request.description.is_empty() {
        details.push(Line::from(Span::styled(
            request.description.clone(),
            appearance.style(Role::TextMuted),
        )));
    }
    details.push(field(appearance, "Project", project_name(app)));
    details.push(field(
        appearance,
        "Enforced by",
        "this coven-code process; no daemon protection applies".to_string(),
    ));
    let consequence = |key: char| match key {
        'y' => Some("this call only"),
        'Y' => Some("this tool or path for the rest of this session"),
        'p' => Some("saves a persistent allow rule to settings for future sessions"),
        'P' => Some("commands with this prefix for the rest of this session"),
        'n' => Some("the call is refused and the model is told"),
        _ => None,
    };
    for (key, _) in &request.options {
        if let Some(text) = consequence(*key) {
            details.push(field(appearance, &format!("[{key}]"), text.to_string()));
        }
    }
    details.push(Line::from(Span::styled(
        "Allowing authorizes the call; its result is shown when the tool finishes.",
        appearance.style(Role::TextSubtle),
    )));
    ApprovalPanel {
        title,
        preview,
        danger,
        options,
        details,
    }
}

fn render_transcript(frame: &mut Frame, app: &App, view: &TranscriptView, area: Rect) {
    let presentation = &app.next_presentation;
    let appearance = &presentation.appearance;
    let width = usize::from(area.width.max(1));
    let mut lines: Vec<Line<'static>> = transcript_lines(presentation, view)
        .into_iter()
        .flat_map(|line| wrap_line(line, width))
        .collect();
    // A scrolled reader gets a notice row at the bottom of the pane.
    let notice_rows = usize::from(!app.auto_scroll && area.height > 1);
    let height = usize::from(area.height).saturating_sub(notice_rows);
    let (start, unseen) = app.next_shell.window(
        lines.len(),
        height,
        area.width,
        app.auto_scroll,
        app.scroll_offset,
    );
    lines.drain(..start);
    lines.truncate(height);
    if notice_rows > 0 {
        lines.resize(height, Line::default());
        let jump = app
            .keybindings
            .reachable_chord("jumpToLive")
            .map(|keys| format!("{keys} jump to live"))
            .unwrap_or_else(|| "scroll down to return to live".to_string());
        let separator = primitives::Mark::Separator.glyph(appearance.glyphs);
        let notice = if unseen > 0 || app.new_messages_while_scrolled > 0 {
            primitives::status_message(
                appearance,
                Status::Pending,
                "new activity below",
                &format!("{separator} {jump}"),
            )
        } else {
            Line::from(Span::styled(
                format!("scrolled up {separator} {jump}"),
                appearance.style(Role::TextMuted),
            ))
        };
        lines.extend(wrap_line(notice, width).into_iter().take(1));
    }
    frame.render_widget(Paragraph::new(lines), area);
}

fn render_inspector(
    frame: &mut Frame,
    app: &App,
    view: &TranscriptView,
    page: InspectorPage,
    area: Rect,
) {
    let appearance = &app.next_presentation.appearance;
    let width = usize::from(area.width.max(1));
    let separator = primitives::Mark::Separator.glyph(appearance.glyphs);
    let mut tabs = Vec::new();
    for candidate in InspectorPage::ALL {
        let (mark, role) = if candidate == page {
            (primitives::Mark::Selected, Role::Focus)
        } else {
            (primitives::Mark::Unselected, Role::TextMuted)
        };
        tabs.push(Span::styled(
            format!("{} {}  ", mark.glyph(appearance.glyphs), candidate.title()),
            appearance.style(role),
        ));
    }
    let mut header = vec![
        primitives::inspector_title(appearance, page.title()),
        Line::from(tabs),
    ];
    if page == InspectorPage::Work {
        // In the header, so scrolling the rows never hides what they are.
        header.extend(work_scope_notes(appearance));
    }
    let body = match page {
        InspectorPage::Activity => activity_page(appearance, app, view),
        InspectorPage::Session => session_page(appearance, app),
        InspectorPage::Work => work_page(appearance, app),
        InspectorPage::Views => views_page(appearance, app),
    };
    let key = |action: &str| {
        app.keybindings
            .reachable_chord_in(action, &KeyContext::Inspector)
    };
    let footer_text = [
        key("nextInspectorPage").map(|keys| format!("{keys} next page")),
        key("closeInspector").map(|keys| format!("{keys} close")),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(&format!(" {separator} "));
    let footer = if footer_text.is_empty() {
        primitives::status_message(
            appearance,
            Status::Blocked,
            "unreachable",
            "no key closes the inspector",
        )
    } else {
        Line::from(Span::styled(
            footer_text,
            appearance.style(Role::TextSubtle),
        ))
    };

    let mut header: Vec<Line<'static>> = header
        .drain(..)
        .flat_map(|line| wrap_line(line, width))
        .collect();
    let footer = wrap_line(footer, width);
    let body: Vec<Line<'static>> = body
        .into_iter()
        .flat_map(|line| wrap_line(line, width))
        .collect();
    let height = usize::from(area.height);
    // The way back stays visible: the footer outranks body rows.
    let footer_rows = footer.len().min(height);
    header.truncate(height - footer_rows);
    let body_rows = height - footer_rows - header.len();
    let scroll_max = body.len().saturating_sub(body_rows);
    app.next_shell.inspector_scroll_max.set(scroll_max);
    let scroll = app.next_shell.inspector_scroll.min(scroll_max);
    let mut lines = header;
    lines.extend(body.into_iter().skip(scroll).take(body_rows));
    lines.resize(height - footer_rows, Line::default());
    lines.extend(footer.into_iter().take(footer_rows));
    frame.render_widget(Clear, area);
    frame.render_widget(Paragraph::new(lines), area);
}

fn field(appearance: &Appearance, name: &str, value: String) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{name:<12}"), appearance.style(Role::TextMuted)),
        Span::styled(value, appearance.style(Role::Text)),
    ])
}

fn activity_page(appearance: &Appearance, app: &App, view: &TranscriptView) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    if app.cancellation != TurnCancellation::Idle {
        lines.push(primitives::status_message(
            appearance,
            Status::Pending,
            "cancelling",
            "waiting for the runtime to stop the turn",
        ));
    } else if view.is_streaming {
        lines.push(primitives::status_message(
            appearance,
            Status::Pending,
            "working",
            "a response is streaming",
        ));
    }
    if view.tools.is_empty() {
        lines.push(Line::from(Span::styled(
            "No tool activity in this turn.",
            appearance.style(Role::TextMuted),
        )));
    }
    for tool in &view.tools {
        lines.push(tool_line(appearance, tool, true));
        lines.extend(tool_detail(appearance, tool));
    }
    if !app.queued_messages.is_empty() {
        lines.push(field(
            appearance,
            "Queued",
            format!(
                "{} message(s) send after this turn",
                app.queued_messages.len()
            ),
        ));
    }
    lines.push(Line::default());
    lines.push(Line::from(Span::styled(
        "Tool detail and diffs arrive with #411.",
        appearance.style(Role::TextSubtle),
    )));
    lines
}

fn session_page(appearance: &Appearance, app: &App) -> Vec<Line<'static>> {
    let project = app
        .config
        .project_dir
        .clone()
        .or_else(|| std::env::current_dir().ok())
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| "no project".to_string());
    let credentials = if app.has_credentials {
        "configured".to_string()
    } else {
        "not configured; nothing connects until you choose a provider".to_string()
    };
    let familiar = match &app.config.familiar {
        Some(id) => format!("{id} (display name; identity unverified until #412)"),
        None => "none".to_string(),
    };
    vec![
        field(appearance, "Project", project),
        field(
            appearance,
            "Session",
            app.session_title
                .clone()
                .unwrap_or_else(|| "untitled session".to_string()),
        ),
        field(
            appearance,
            "Model",
            model_label(app).unwrap_or_else(|| "not selected".to_string()),
        ),
        field(
            appearance,
            "Provider",
            app.config
                .provider
                .clone()
                .unwrap_or_else(|| "not selected".to_string()),
        ),
        field(appearance, "Credentials", credentials),
        field(appearance, "Familiar", familiar),
        field(
            appearance,
            "Renderer",
            "Next preview; unset COVEN_CODE_UI for Classic".to_string(),
        ),
    ]
}

/// What the Work page's rows are, and what it cannot show yet.
fn work_scope_notes(appearance: &Appearance) -> Vec<Line<'static>> {
    vec![
        primitives::status_message(
            appearance,
            Status::Evidence,
            "engine-local",
            "in-memory tasks span this process, not just this session",
        ),
        primitives::status_message(
            appearance,
            Status::Blocked,
            "unavailable",
            "canonical tasks, lanes and leases need Psyche #11/#12 (#413)",
        ),
    ]
}

fn work_page(appearance: &Appearance, app: &App) -> Vec<Line<'static>> {
    use crate::application::work::{WorkStateKind, WorkView};

    let view = WorkView::from_sources(&app.work_sources);
    let mut lines = Vec::new();
    if let Some(error) = &view.goal_error {
        lines.push(primitives::status_message(
            appearance,
            Status::Blocked,
            "unreadable",
            &format!("session goal: {error}"),
        ));
        lines.push(Line::default());
    }
    if view.items.is_empty() {
        lines.push(Line::from(Span::styled(
            "There is no engine-local work in this process.".to_string(),
            appearance.style(Role::TextMuted),
        )));
        return lines;
    }
    for item in &view.items {
        lines.push(Line::from(vec![
            Span::styled(
                format!("{} ", item.origin.label()),
                appearance.style(Role::TextMuted),
            ),
            Span::styled(item.title.clone(), appearance.style(Role::Text)),
        ]));
        let status = match item.kind {
            WorkStateKind::Open => Status::Pending,
            WorkStateKind::Done => Status::Verified,
            WorkStateKind::Failed => Status::Blocked,
            WorkStateKind::Stopping => Status::Pending,
            WorkStateKind::Limited => Status::Pending,
        };
        let mut state = Line::from(Span::styled(
            format!("{:<12}", "State"),
            appearance.style(Role::TextMuted),
        ));
        state
            .spans
            .extend(primitives::status_message(appearance, status, &item.state, "").spans);
        lines.push(state);
        lines.push(field(appearance, "Id", item.id.clone()));
        if let Some(owner) = &item.owner {
            lines.push(field(appearance, "Owner", owner.clone()));
        }
        if !item.blocked_by.is_empty() {
            lines.push(field(appearance, "Blocked by", item.blocked_by.join(", ")));
        }
        for missing in &item.missing_blockers {
            let mut line = Line::from(Span::styled(
                format!("{:<12}", ""),
                appearance.style(Role::TextMuted),
            ));
            line.spans.extend(
                primitives::status_message(
                    appearance,
                    Status::Blocked,
                    "missing",
                    &format!("{missing} matches no listed task"),
                )
                .spans,
            );
            lines.push(line);
        }
        for (name, value) in &item.detail {
            lines.push(field(appearance, name, value.clone()));
        }
        lines.push(Line::default());
    }
    lines
}

fn views_page(appearance: &Appearance, app: &App) -> Vec<Line<'static>> {
    let view = |name: &str, detail: &str| {
        let mut line = Line::from(Span::styled(
            format!("  {name:<10}"),
            appearance.style(Role::Text),
        ));
        line.spans.extend(
            primitives::status_message(appearance, Status::Blocked, "unavailable", detail).spans,
        );
        line
    };
    let mut lines = vec![
        primitives::list_item(appearance, "Chat      current", true),
        {
            let mut line = Line::from(Span::styled(
                format!("  {:<10}", "Work"),
                appearance.style(Role::Text),
            ));
            line.spans.extend(
                primitives::status_message(
                    appearance,
                    Status::Pending,
                    "local only",
                    "Work page; canonical tasks wait on #413",
                )
                .spans,
            );
            line
        },
        view("Research", "needs a qualified research lifecycle (#414)"),
        view("Memory", "needs a qualified memory reader (#415)"),
        Line::default(),
    ];
    let palette = app
        .keybindings
        .reachable_chord("openCommandPalette")
        .map(|keys| format!("{keys} command palette, / suggestions"))
        .unwrap_or_else(|| "/ suggestions (no key opens the palette)".to_string());
    lines.push(field(appearance, "Commands", palette));
    lines
}

/// Wrap a line to `width` cells, keeping each span's style on the cells it
/// covers.
fn wrap_line(line: Line<'static>, width: usize) -> Vec<Line<'static>> {
    if line.width() <= width {
        return vec![line];
    }
    let mut lines = Vec::new();
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut used = 0;
    for span in line.spans {
        let mut text = String::new();
        for ch in span.content.chars() {
            let cells = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
            if used + cells > width && used > 0 {
                if !text.is_empty() {
                    spans.push(Span::styled(std::mem::take(&mut text), span.style));
                }
                lines.push(Line::from(std::mem::take(&mut spans)));
                used = 0;
            }
            text.push(ch);
            used += cells;
        }
        if !text.is_empty() {
            spans.push(Span::styled(text, span.style));
        }
    }
    lines.push(Line::from(spans));
    lines
}

/// Rule above the composer; a non-empty draft is labeled as not sent.
fn composer_rule(appearance: &Appearance, view: &TranscriptView, width: u16) -> Line<'static> {
    if view.draft.is_empty() && view.draft_attachments.is_empty() {
        return primitives::rule(appearance, width);
    }
    let label = "draft not sent ";
    let fill = usize::from(width).saturating_sub(label.len());
    Line::from(vec![
        Span::styled(label, appearance.style(Role::TextMuted)),
        Span::styled(
            primitives::Mark::Rule.glyph(appearance.glyphs).repeat(fill),
            appearance.style(Role::Border),
        ),
    ])
}

fn status_line(appearance: &Appearance, app: &App, view: &TranscriptView) -> Line<'static> {
    let separator = primitives::Mark::Separator.glyph(appearance.glyphs);
    if app.cancellation != TurnCancellation::Idle {
        // Not "Cancelled." until the runtime has actually stopped the turn.
        return primitives::status_message(
            appearance,
            Status::Pending,
            "cancelling",
            "waiting for the runtime to stop the turn",
        );
    }
    let mut spans = if view.permission.is_some() {
        primitives::status_label(appearance, Status::Pending, "awaiting permission")
    } else if app.voice_recording {
        primitives::status_label(appearance, Status::Pending, "recording voice")
    } else if view.is_streaming {
        vec![Span::styled(
            format!(
                "{} working",
                primitives::activity(appearance, app.frame_count)
            ),
            appearance.style(Role::Presence),
        )]
    } else {
        vec![Span::styled("ready", appearance.style(Role::TextMuted))]
    };
    let running = view
        .tools
        .iter()
        .filter(|tool| tool.state == ToolActivityState::Running)
        .count();
    if running > 0 {
        spans.push(Span::styled(
            format!(" {separator} {running} tool(s) running"),
            appearance.style(Role::TextMuted),
        ));
    }
    if !app.queued_messages.is_empty() {
        spans.push(Span::styled(
            format!(" {separator} {} queued", app.queued_messages.len()),
            appearance.style(Role::TextMuted),
        ));
    }
    if let Some(status) = &view.status {
        spans.push(Span::styled(
            format!(" {separator} {status}"),
            appearance.style(Role::TextMuted),
        ));
    }
    Line::from(spans)
}

fn hints_line(appearance: &Appearance, hints: &KeyHints) -> Line<'static> {
    let separator = format!(" {} ", primitives::Mark::Separator.glyph(appearance.glyphs));
    let mut spans = vec![Span::styled(
        hints.parts.join(&separator),
        appearance.style(Role::TextSubtle),
    )];
    if !hints.unreachable.is_empty() {
        if !hints.parts.is_empty() {
            spans.push(Span::styled(separator, appearance.style(Role::TextSubtle)));
        }
        spans.extend(
            primitives::status_message(
                appearance,
                Status::Blocked,
                "unreachable",
                &format!("no key reaches {}", hints.unreachable.join(", ")),
            )
            .spans,
        );
    }
    Line::from(spans)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::{Color, Style};

    #[test]
    fn wrapping_keeps_each_span_style_on_both_sides_of_the_break() {
        let red = Style::default().fg(Color::Red);
        let blue = Style::default().fg(Color::Blue);
        let line = Line::from(vec![Span::styled("abcd", red), Span::styled("efgh", blue)]);
        let wrapped = wrap_line(line, 3);
        let rows: Vec<Vec<(String, Style)>> = wrapped
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| (span.content.to_string(), span.style))
                    .collect()
            })
            .collect();
        assert_eq!(
            rows,
            vec![
                vec![("abc".to_string(), red)],
                vec![("d".to_string(), red), ("ef".to_string(), blue)],
                vec![("gh".to_string(), blue)],
            ]
        );
        // A character wider than the whole width still gets its own row.
        assert_eq!(wrap_line(Line::from("漢字"), 1).len(), 2);
    }
}
