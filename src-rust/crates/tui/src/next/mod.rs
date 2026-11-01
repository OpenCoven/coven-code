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
use crate::application::view::{EntryRole, ToolActivityState, TranscriptView};
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
    app.any_modal_open()
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
    /// Destinations of the shared navigation model and their availability.
    Views,
}

impl InspectorPage {
    pub const ALL: [InspectorPage; 3] = [
        InspectorPage::Activity,
        InspectorPage::Session,
        InspectorPage::Views,
    ];

    pub fn title(self) -> &'static str {
        match self {
            InspectorPage::Activity => "Activity",
            InspectorPage::Session => "Session",
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
/// reader on the same content while output arrives below.
#[derive(Debug, Clone, Copy, Default)]
struct ReadingAnchor {
    width: u16,
    total: usize,
    /// Growth not yet folded into `App::scroll_offset`.
    unsettled: usize,
    /// Growth since the reader scrolled away; drives the activity notice.
    unseen: usize,
}

/// Navigation state of the Next shell. Lives on [`App`] but only Next reads
/// or changes it.
#[derive(Debug, Default)]
pub struct ShellState {
    /// The one open inspector page, if any. It owns Escape and Tab; the
    /// composer keeps every other key, so the draft is never stranded.
    pub inspector: Option<InspectorPage>,
    pub inspector_scroll: usize,
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
                self.inspector_scroll = self.inspector_scroll.saturating_add(INSPECTOR_SCROLL_STEP);
            }
            "jumpToLive" => self.anchor.set(ReadingAnchor::default()),
            _ => {}
        }
    }

    pub(crate) fn take_unsettled_growth(&self) -> usize {
        let mut anchor = self.anchor.get();
        let growth = std::mem::take(&mut anchor.unsettled);
        self.anchor.set(anchor);
        growth
    }

    /// First visible row of a `total`-row transcript in a `height`-row pane,
    /// and how many rows arrived since the reader scrolled away.
    ///
    /// `scroll_offset` counts rows above the bottom, so new output would
    /// otherwise push a scrolled reader's text upward. Growth measured at an
    /// unchanged width is added to the offset until the loop settles it.
    /// A width change rewraps everything, so it re-measures instead.
    fn window(
        &self,
        total: usize,
        height: usize,
        width: u16,
        auto_scroll: bool,
        scroll_offset: usize,
    ) -> (usize, usize) {
        let mut anchor = self.anchor.get();
        let offset = if auto_scroll {
            anchor = ReadingAnchor {
                width,
                total,
                ..ReadingAnchor::default()
            };
            0
        } else {
            if anchor.width == width && total > anchor.total {
                let growth = total - anchor.total;
                anchor.unsettled += growth;
                anchor.unseen += growth;
            }
            anchor.width = width;
            anchor.total = total;
            scroll_offset.saturating_add(anchor.unsettled)
        };
        self.anchor.set(anchor);
        let max_scroll = total.saturating_sub(height);
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
    let layout = layout::plan(
        area,
        app.next_shell.inspector.is_some(),
        hint_rows,
        u16::try_from(composer.len()).unwrap_or(u16::MAX),
    );

    frame.render_widget(
        Paragraph::new(workspace_line(appearance, app, area.width)),
        layout.header,
    );
    match layout.transcript {
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

    // Keep the newest composer rows when the draft outgrows its rows.
    let overflow = composer
        .len()
        .saturating_sub(usize::from(layout.composer.height));
    composer.drain(..overflow);
    frame.render_widget(Paragraph::new(composer), layout.composer);
    let (row, column) = composer_cursor(&view.draft, view.draft_cursor, width);
    if row < draft_rows && row >= overflow {
        if let (Ok(y), Ok(x)) = (u16::try_from(row - overflow), u16::try_from(column)) {
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
    let cursor = cursor.min(draft.len());
    let before = draft.get(..cursor).unwrap_or(draft);
    let line_start = before.rfind('\n').map_or(0, |index| index + 1);
    let mut row: usize = draft[..line_start]
        .split_terminator('\n')
        .map(|line| {
            let cells = COMPOSER_LEAD + unicode_width::UnicodeWidthStr::width(line);
            cells.div_ceil(width).max(1)
        })
        .sum();
    let mut used = COMPOSER_LEAD.min(width);
    for ch in before[line_start..].chars() {
        let cells = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
        if used + cells > width && used > 0 {
            row += 1;
            used = 0;
        }
        used += cells;
    }
    if used >= width {
        (row + 1, 0)
    } else {
        (row, used)
    }
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
    for tool in &view.tools {
        let (status, word) = tool_status(tool.state);
        lines.push(primitives::tool_row(
            appearance, &tool.name, status, word, false,
        ));
    }
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
    let body = match page {
        InspectorPage::Activity => activity_page(appearance, app, view),
        InspectorPage::Session => session_page(appearance, app),
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
    let scroll = app
        .next_shell
        .inspector_scroll
        .min(body.len().saturating_sub(body_rows));
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
        let (status, word) = tool_status(tool.state);
        lines.push(primitives::tool_row(
            appearance, &tool.name, status, word, false,
        ));
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
        view("Work", "needs a qualified task producer (#413)"),
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
