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

pub mod appearance;
pub mod layout;
pub mod primitives;
pub mod style;
pub mod tracked;

use crate::app::App;
use crate::application::view::{
    terminal_safe, EntryRole, EntrySource, ToolActivity, ToolActivityState, TranscriptEntry,
    TranscriptView,
};
use crate::application::TurnCancellation;
use claurst_core::appearance::Density;
use claurst_core::keybindings::{KeyContext, KeybindingResolver, UserKeybindings};
use claurst_core::types::Message;
use primitives::Status;
use ratatui::layout::{Position, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};
use ratatui::Frame;
use std::cell::{Cell, RefCell};
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
    /// This session's web search and fetch activity.
    Research,
    /// Familiar memory read from the Coven daemon, read only.
    Memory,
    /// Destinations of the shared navigation model and their availability.
    Views,
}

impl InspectorPage {
    pub const ALL: [InspectorPage; 6] = [
        InspectorPage::Activity,
        InspectorPage::Session,
        InspectorPage::Work,
        InspectorPage::Research,
        InspectorPage::Memory,
        InspectorPage::Views,
    ];

    pub fn title(self) -> &'static str {
        match self {
            InspectorPage::Activity => "Activity",
            InspectorPage::Session => "Session",
            InspectorPage::Work => "Work",
            InspectorPage::Research => "Research",
            InspectorPage::Memory => "Memory",
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
    "jumpToStart",
    "suspend",
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
    /// Set while the reader stays at the first row after going to the start:
    /// the offset from the bottom it was last drawn at. Layout changes and
    /// new rows move that offset, so it is recomputed until the reader
    /// scrolls down past it.
    start_hold: Option<i64>,
}

#[derive(Debug, Default)]
pub struct ShellState {
    /// Appearance changes being previewed, not yet saved.
    pub appearance_edits: claurst_core::appearance::AppearanceEdits,
    /// The one open inspector page, if any. It owns Escape and Tab; the
    /// composer keeps every other key, so the draft is never stranded.
    pub inspector: Option<InspectorPage>,
    pub inspector_scroll: usize,
    /// Largest useful `inspector_scroll` for the last drawn page.
    inspector_scroll_max: Cell<usize>,
    anchor: Cell<ReadingAnchor>,
    suspend_requested: bool,
    transcript_rows: RefCell<TranscriptRows>,
    tool_rows: RefCell<ToolRows>,
    /// Whether the last frame showed the current notice. A notice waits while
    /// the inspector replaces the transcript, and Esc must not dismiss it unseen.
    notice_shown: Cell<bool>,
    /// The selection's cells at the last frame. A change means the user moved
    /// the selection; no change means anything different under it moved there.
    selection_cells: Cell<Option<SelectionCells>>,
    /// The screen moved under the selection, so `App::selection_text` holds
    /// what was selected and no cell on screen shows it.
    selection_kept: Cell<bool>,
}

type SelectionCells = ((u16, u16), (u16, u16));

/// Rows retained until their tracked source or layout changes. Mutation
/// identities include indexed edits and same-length replacement; unchanged
/// frames need neither source clones nor comparisons of preceding history.
#[derive(Debug, Default)]
struct TranscriptRows {
    layout: Option<(usize, Appearance, Density)>,
    messages: Vec<(EntrySource, Vec<Line<'static>>)>,
    messages_total: usize,
    message_ends: Vec<usize>,
    messages_stamp: Option<tracked::Stamp>,
    live_stamp: Option<tracked::Stamp>,
    /// The complete lines of the streaming answer that `live_rows` shows.
    live_processed: usize,
    // Index visual rows without reallocating/copying the entire streaming
    // history when an appended row crosses a contiguous vector's capacity.
    live_rows: std::collections::BTreeMap<usize, Line<'static>>,
    live_tail: Vec<Line<'static>>,
    #[cfg(test)]
    visited_groups: Cell<usize>,
}

impl TranscriptRows {
    fn sync(
        &mut self,
        messages: &tracked::Tracked<Vec<Message>>,
        streaming: &tracked::Tracked<String>,
        presentation: &NextPresentation,
        width: usize,
    ) {
        let layout = (width, presentation.appearance, presentation.density);
        if self.layout != Some(layout) {
            self.layout = Some(layout);
            self.messages.clear();
            self.live_processed = 0;
            self.live_tail.clear();
            self.messages_stamp = None;
            self.live_stamp = None;
            self.live_rows.clear();
        }
        let stamp = messages.stamp();
        if self.messages_stamp.as_ref() != Some(&stamp) {
            let first = messages
                .take_dirty_from()
                .min(self.messages.len())
                .min(messages.len());
            self.messages.truncate(messages.len());
            for (index, message) in messages.iter().enumerate().skip(first) {
                let source = EntrySource::of(message);
                match self.messages.get_mut(index) {
                    Some((cached, _)) if *cached == source => {}
                    slot => {
                        let rows = source
                            .project()
                            .map(|entry| entry_lines(presentation, &entry))
                            .unwrap_or_default()
                            .into_iter()
                            .flat_map(|line| wrap_line(line, width))
                            .collect();
                        match slot {
                            Some(cached) => *cached = (source, rows),
                            None => self.messages.push((source, rows)),
                        }
                    }
                }
            }
            self.message_ends.truncate(first);
            let mut total = self.message_ends.last().copied().unwrap_or(0);
            for (_, rows) in self.messages.iter().skip(first) {
                total += rows.len();
                self.message_ends.push(total);
            }
            self.messages_total = total;
            self.messages_stamp = Some(stamp);
        }
        let stamp = streaming.stamp();
        if self.live_stamp.as_ref() != Some(&stamp) {
            let dirty = streaming.take_dirty_from();
            if self.live_stamp.is_none()
                || dirty < self.live_processed
                || streaming.len() < self.live_processed
            {
                self.live_processed = 0;
                self.live_tail.clear();
                self.live_rows.clear();
            }
            self.sync_live(streaming, presentation, width);
            self.live_stamp = Some(stamp);
        }
    }

    /// Rewrap only the final two visual rows and the new delta. Keeping two
    /// rows lets a newly extended grapheme move back across the last wrap
    /// boundary (variation selectors and ZWJ sequences can change its width).
    /// Earlier visual rows are indexed and never copied on ordinary append.
    fn sync_live(&mut self, streaming: &str, presentation: &NextPresentation, width: usize) {
        let appearance = &presentation.appearance;
        for piece in streaming[self.live_processed..].split_inclusive('\n') {
            let newline = piece.ends_with('\n');
            let text = piece.strip_suffix('\n').unwrap_or(piece);
            let mut suffix = String::new();
            for row in &self.live_tail {
                for span in &row.spans {
                    suffix.push_str(&span.content);
                }
            }
            suffix.push_str(&terminal_safe(text));
            let mut rows = if suffix.is_empty() {
                Vec::new()
            } else {
                wrap_line(text_line(appearance, &suffix), width)
            };
            let retained = if newline { 0 } else { 2 };
            let complete = rows.len().saturating_sub(retained);
            for row in rows.drain(..complete) {
                self.live_rows.insert(self.live_rows.len(), row);
            }
            self.live_tail = rows;
            if newline && complete == 0 {
                // A genuine blank logical line still occupies one row.
                self.live_rows
                    .insert(self.live_rows.len(), text_line(appearance, ""));
            }
        }
        self.live_processed = streaming.len();
    }

    /// Up to `count` rows from row `start`, across the messages, then `tools`
    /// (tool rows and the live label), then the streaming answer.
    fn visible(&self, tools: &[Line<'static>], start: usize, count: usize) -> Vec<Line<'static>> {
        #[cfg(test)]
        self.visited_groups.set(0);
        let first = self.message_ends.partition_point(|end| *end <= start);
        let mut skip = start.saturating_sub(if first == 0 {
            0
        } else {
            self.message_ends[first - 1]
        });
        let mut lines = Vec::with_capacity(count);
        // Equal cumulative ends describe display-empty messages. Jump over
        // each run instead of walking invisible tool/thinking-only history.
        let groups =
            std::iter::successors((first < self.messages.len()).then_some(first), |&index| {
                let end = self.message_ends[index];
                let next = self
                    .message_ends
                    .partition_point(|candidate| *candidate <= end);
                (next < self.messages.len()).then_some(next)
            })
            .map(|index| self.messages[index].1.as_slice())
            .chain([tools]);
        for rows in groups {
            #[cfg(test)]
            self.visited_groups.set(self.visited_groups.get() + 1);
            if lines.len() == count {
                return lines;
            }
            if skip >= rows.len() {
                skip -= rows.len();
                continue;
            }
            lines.extend(rows[skip..].iter().take(count - lines.len()).cloned());
            skip = 0;
        }
        if lines.len() < count {
            lines.extend(
                self.live_rows
                    .range(skip..)
                    .take(count - lines.len())
                    .map(|(_, row)| row.clone()),
            );
            // The tail follows every indexed complete visual row.
            if lines.len() < count {
                let tail_skip = skip.saturating_sub(self.live_rows.len());
                lines.extend(
                    self.live_tail
                        .iter()
                        .skip(tail_skip)
                        .take(count - lines.len())
                        .cloned(),
                );
            }
        }
        lines
    }

    /// Rows `visible` can return, given the size of `tools`.
    fn total(&self, tools: usize) -> usize {
        self.messages_total + tools + self.live_rows.len() + self.live_tail.len()
    }
}

#[derive(Debug, PartialEq, Eq)]
struct ToolKey {
    active: tracked::Stamp,
    interrupted: tracked::Stamp,
    decisions: tracked::Stamp,
    cancellation: TurnCancellation,
    project: Option<std::path::PathBuf>,
    diagnostic_path: Option<std::path::PathBuf>,
    memory_diagnostics: bool,
    live: bool,
}

#[derive(Debug, Default)]
struct ToolRows {
    key: Option<ToolKey>,
    layout: Option<(usize, Appearance)>,
    tools: Vec<ToolActivity>,
    rows: Vec<Line<'static>>,
    running: usize,
    detail_layout: Option<(usize, Appearance)>,
    details: Vec<Line<'static>>,
}

impl ToolRows {
    fn sync_details(&mut self, presentation: &NextPresentation, width: usize) {
        let layout = (width, presentation.appearance);
        if self.detail_layout != Some(layout) {
            let appearance = &presentation.appearance;
            self.details = self
                .tools
                .iter()
                .flat_map(|tool| {
                    std::iter::once(tool_line(appearance, tool, true))
                        .chain(tool_detail(appearance, tool))
                })
                .flat_map(|line| wrap_line(line, width))
                .collect();
            self.detail_layout = Some(layout);
        }
    }

    fn sync(&mut self, app: &App, width: usize) {
        let key = ToolKey {
            active: app.tool_use_blocks.stamp(),
            interrupted: app.interrupted_tools.stamp(),
            decisions: app.permission_decisions.stamp(),
            cancellation: app.cancellation,
            project: app.config.project_dir.clone(),
            diagnostic_path: app.diagnostic_log_path.clone(),
            memory_diagnostics: app.diagnostic_buffer.is_some(),
            live: !app.streaming_text.is_empty(),
        };
        let changed = self.key.as_ref() != Some(&key);
        if changed {
            self.tools = TranscriptView::project_tools(app);
            self.running = self
                .tools
                .iter()
                .filter(|tool| tool.state == ToolActivityState::Running)
                .count();
            self.key = Some(key);
            self.detail_layout = None;
        }
        let layout = (width, app.next_presentation.appearance);
        if changed || self.layout != Some(layout) {
            self.rows = self
                .tools
                .iter()
                .flat_map(|tool| {
                    wrap_line(
                        tool_line(&app.next_presentation.appearance, tool, false),
                        width,
                    )
                })
                .collect();
            if !app.streaming_text.is_empty() {
                self.rows.push(primitives::transcript_label(
                    &app.next_presentation.appearance,
                    "coven",
                ));
            }
            let diagnostic_hint = app
                .diagnostic_log_path
                .as_ref()
                .map(|path| format!("Diagnostics: {}", terminal_safe(&path.to_string_lossy())))
                .or_else(|| {
                    app.diagnostic_buffer
                        .as_ref()
                        .map(|_| "Diagnostics: memory only · /status logs".to_owned())
                });
            if let Some(hint) = diagnostic_hint {
                self.rows.extend(wrap_line(
                    Line::from(Span::styled(
                        hint,
                        app.next_presentation.appearance.style(Role::TextMuted),
                    )),
                    width,
                ));
            }
            self.layout = Some(layout);
        }
    }
}

impl ShellState {
    /// A switched or resumed session brings its own reading offset; the
    /// outgoing session's anchor and kept rows cannot describe it.
    pub(crate) fn reset_transcript_for_session(&mut self) {
        self.anchor.set(ReadingAnchor::default());
        *self.transcript_rows.get_mut() = TranscriptRows::default();
    }

    /// Whether the last Next frame showed the current notice.
    pub fn notice_shown(&self) -> bool {
        self.notice_shown.get()
    }

    /// Whether the user asked to suspend since the last call. The event loop
    /// owns the terminal, so it carries the suspend out.
    pub fn take_suspend_request(&mut self) -> bool {
        std::mem::take(&mut self.suspend_requested)
    }

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
            "jumpToStart" => {
                let mut anchor = self.anchor.get();
                anchor.unsettled = 0;
                anchor.start_hold = Some(i64::MIN);
                self.anchor.set(anchor);
            }
            "suspend" => self.suspend_requested = true,
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
            let held = anchor.start_hold.is_some_and(|hold| current >= hold);
            if anchor.total != 0 && anchor.width == width {
                anchor.unseen += total.saturating_sub(anchor.total);
            }
            let wanted = if held {
                as_rows(max_scroll)
            } else if anchor.total == 0 {
                // Nothing measured yet: take the offset as given.
                current
            } else if anchor.width == width {
                current.saturating_add(as_rows(total) - as_rows(anchor.total))
            } else {
                current.saturating_mul(as_rows(total)) / as_rows(anchor.total)
            };
            let clamped = wanted.clamp(0, as_rows(max_scroll));
            anchor.start_hold = held.then_some(clamped);
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
    let view = TranscriptView::project_chrome(app);
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

    app.next_shell.tool_rows.borrow_mut().sync(
        app,
        usize::from(
            layout
                .transcript
                .map_or(area.width, |pane| pane.width)
                .max(1),
        ),
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
            draw_selection(frame, app);
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

    // A pending approval owns input, so every notice waits until it is
    // decided. An error notice is then modal (Esc dismisses it in either
    // renderer).
    app.next_shell.notice_shown.set(false);
    if app.notifications.current_is_error() && approval.is_none() {
        render_error_panel(frame, app, area, layout.composer);
    } else if approval.is_none() {
        render_toast(frame, app, layout.transcript);
    }
}

/// Mark the selection, unless the screen has moved under it.
///
/// A selection is stored as screen cells, and its text is read back from them
/// every frame. A resize, a scroll or streamed rows can put other text in those
/// cells, and copying would then return text nobody selected. So once the cells
/// stop changing the text is fixed: if they show anything else, the mark goes
/// and the text stays available to copy until typing, a paste or a new
/// selection lets it go.
fn draw_selection(frame: &mut Frame, app: &App) {
    let shell = &app.next_shell;
    let cells = app.selection_anchor.zip(app.selection_focus);
    let moved_by_user = shell.selection_cells.replace(cells) != cells;
    if moved_by_user {
        shell.selection_kept.set(false);
    }
    if shell.selection_kept.get() {
        return;
    }
    let selected = app.selection_text.borrow().clone();
    if moved_by_user || selected.is_empty() {
        crate::render::apply_selection_highlight(frame, app);
        return;
    }
    let Some(((column, row), _)) = cells else {
        return;
    };
    if !app
        .last_selectable_area
        .get()
        .contains(Position::new(column, row))
    {
        shell.selection_kept.set(true);
        return;
    }
    let unmarked = frame.buffer_mut().clone();
    crate::render::apply_selection_highlight(frame, app);
    if *app.selection_text.borrow() != selected {
        *frame.buffer_mut() = unmarked;
        *app.selection_text.borrow_mut() = selected;
        shell.selection_kept.set(true);
    }
}

/// The current error notice, drawn by Next over the composer. Its text can
/// quote provider or tool output, so it is made inert and wrapped.
fn render_error_panel(frame: &mut Frame, app: &App, area: Rect, composer: Rect) {
    let Some(notification) = app.notifications.current() else {
        return;
    };
    let appearance = &app.next_presentation.appearance;
    let width = usize::from(area.width.max(1));
    let mut lines = wrap_line(
        primitives::status_message(
            appearance,
            Status::Blocked,
            "error",
            &safe_one_line(&notification.message),
        ),
        width,
    );
    let max_rows = usize::from((area.height / 2).max(2)) - 1;
    lines.truncate(max_rows);
    lines.push(Line::from(Span::styled(
        "Esc dismiss".to_string(),
        appearance.style(Role::TextSubtle),
    )));
    let height = u16::try_from(lines.len())
        .unwrap_or(u16::MAX)
        .min(area.height);
    let bottom = (composer.y + composer.height).min(area.y + area.height);
    let panel = Rect {
        x: area.x,
        y: bottom.saturating_sub(height).max(area.y),
        width: area.width,
        height,
    };
    frame.render_widget(Clear, panel);
    frame.render_widget(Paragraph::new(lines), panel);
}

/// The current notification as one Next row at the top right of the
/// transcript, in the selected color mode, so it never covers the header,
/// an approval or the inspector. Its text can quote tool or model output, so
/// it is made inert. When the inspector replaces the transcript the notice
/// waits. Classic keeps its own banner.
fn render_toast(frame: &mut Frame, app: &App, transcript: Option<Rect>) {
    use crate::notifications::NotificationKind;

    let (Some(notification), Some(area)) = (app.notifications.current(), transcript) else {
        return;
    };
    let appearance = &app.next_presentation.appearance;
    let width = 52u16.min(area.width.saturating_sub(2));
    if width < 20 || area.height < 2 {
        return;
    }
    let (status, word) = match notification.kind {
        NotificationKind::Info => (Status::Evidence, "note"),
        NotificationKind::Success => (Status::Evidence, "done"),
        NotificationKind::Warning => (Status::Pending, "warning"),
        NotificationKind::Error => (Status::Blocked, "error"),
    };
    let hint = if notification.dismissible {
        "  Esc"
    } else {
        ""
    };
    let text_width = usize::from(width).saturating_sub(hint.chars().count());
    let message = primitives::status_message(
        appearance,
        status,
        word,
        &safe_one_line(&notification.message),
    );
    let mut rows = wrap_line(message, text_width.saturating_sub(1).max(1));
    let cut = rows.len() > 1;
    let mut line = rows.drain(..).next().unwrap_or_default();
    if cut {
        line.spans.push(Span::styled(
            "…".to_string(),
            appearance.style(Role::TextSubtle),
        ));
    }
    line.spans.push(Span::styled(
        hint.to_string(),
        appearance.style(Role::TextSubtle),
    ));
    let toast = Rect {
        x: area.x + area.width.saturating_sub(width + 1),
        y: area.y,
        width,
        height: 1,
    };
    frame.render_widget(Clear, toast);
    frame.render_widget(Paragraph::new(line), toast);
    app.next_shell.notice_shown.set(true);
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
        let span = Span::raw(text);
        for grapheme in span.styled_graphemes(ratatui::style::Style::default()) {
            let cells = unicode_width::UnicodeWidthStr::width(grapheme.symbol);
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

/// Untrusted text made terminal safe and folded onto one line.
fn safe_one_line(text: &str) -> String {
    crate::application::view::terminal_safe(text).replace('\n', " ")
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
            .as_deref()
            .map(crate::application::view::terminal_safe)
            .unwrap_or_else(|| "untitled session".to_string()),
        Role::TextMuted,
    ));
    let (ownership, _) = app.ownership.describe(app.session_id.as_deref());
    parts.push((ownership.to_string(), Role::Evidence));
    if let Some(model) = model_label(app) {
        parts.push((model, Role::TextMuted));
    }
    if let Some(familiar) = &app.config.familiar {
        // No producer binds a familiar name to an identity yet; a name or
        // avatar is never treated as one.
        parts.push((format!("{familiar} (unverified)"), Role::Evidence));
    }
    for (text, role) in parts {
        // Titles come from the model or the user, and paths and names from
        // config: show them inert, on one line.
        let text = safe_one_line(&text);
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

/// Rows of one committed message, before wrapping.
fn entry_lines(presentation: &NextPresentation, entry: &TranscriptEntry) -> Vec<Line<'static>> {
    let appearance = &presentation.appearance;
    let mut lines = vec![primitives::transcript_label(
        appearance,
        match entry.role {
            EntryRole::User => "you",
            EntryRole::Assistant => "coven",
        },
    )];
    lines.extend(
        entry
            .text
            .lines()
            .map(|content| text_line(appearance, content)),
    );
    lines.extend(entry.attachments.iter().map(|label| {
        Line::from(Span::styled(
            label.clone(),
            appearance.style(Role::Evidence),
        ))
    }));
    if presentation.density == Density::Comfortable {
        lines.push(Line::default());
    }
    lines
}

fn text_line(appearance: &Appearance, content: &str) -> Line<'static> {
    Line::from(Span::styled(
        content.to_string(),
        appearance.style(Role::Text),
    ))
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
    let is_command = claurst_core::permissions::is_command_tool(&request.tool_name);
    let consequence = |key: char| match key {
        'y' => Some("this call only"),
        'Y' if is_command => Some("this exact command for the rest of this session"),
        'Y' => Some("this tool or path for the rest of this session"),
        'p' if is_command => Some("saves this exact command to settings for future sessions"),
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

fn render_transcript(frame: &mut Frame, app: &App, _view: &TranscriptView, area: Rect) {
    let presentation = &app.next_presentation;
    let appearance = &presentation.appearance;
    let width = usize::from(area.width.max(1));
    let tools = app.next_shell.tool_rows.borrow();
    let mut rows = app.next_shell.transcript_rows.borrow_mut();
    rows.sync(&app.messages, &app.streaming_text, presentation, width);
    // A scrolled reader gets a notice row at the bottom of the pane.
    let notice_rows = usize::from(!app.auto_scroll && area.height > 1);
    let height = usize::from(area.height).saturating_sub(notice_rows);
    let (start, unseen) = app.next_shell.window(
        rows.total(tools.rows.len()),
        height,
        area.width,
        app.auto_scroll,
        app.scroll_offset,
    );
    let mut lines = rows.visible(&tools.rows, start, height);
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
    // In the header, so scrolling the rows never hides what they are.
    match page {
        InspectorPage::Work => header.extend(work_scope_notes(appearance)),
        InspectorPage::Research => header.extend(research_scope_notes(appearance)),
        InspectorPage::Memory => header.extend(memory_scope_notes(appearance)),
        _ => {}
    }
    let body = match page {
        InspectorPage::Activity => Vec::new(),
        InspectorPage::Session => session_page(appearance, app),
        InspectorPage::Work => work_page(appearance, app),
        InspectorPage::Research => research_page(appearance, app),
        InspectorPage::Memory => memory_page(appearance, app),
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
    if page == InspectorPage::Activity {
        let (prefix, suffix) = activity_page(appearance, app, view);
        let prefix: Vec<_> = prefix
            .into_iter()
            .flat_map(|line| wrap_line(line, width))
            .collect();
        let suffix: Vec<_> = suffix
            .into_iter()
            .flat_map(|line| wrap_line(line, width))
            .collect();
        let mut tools = app.next_shell.tool_rows.borrow_mut();
        tools.sync_details(&app.next_presentation, width);
        let total = prefix.len() + tools.details.len() + suffix.len();
        let scroll_max = total.saturating_sub(body_rows);
        app.next_shell.inspector_scroll_max.set(scroll_max);
        let mut skip = app.next_shell.inspector_scroll.min(scroll_max);
        let mut visible = Vec::with_capacity(body_rows);
        for rows in [
            prefix.as_slice(),
            tools.details.as_slice(),
            suffix.as_slice(),
        ] {
            if skip >= rows.len() {
                skip -= rows.len();
                continue;
            }
            visible.extend(rows[skip..].iter().take(body_rows - visible.len()).cloned());
            skip = 0;
            if visible.len() == body_rows {
                break;
            }
        }
        let mut lines = header;
        lines.extend(visible);
        lines.resize(height - footer_rows, Line::default());
        lines.extend(footer.into_iter().take(footer_rows));
        frame.render_widget(Clear, area);
        frame.render_widget(Paragraph::new(lines), area);
        return;
    }
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

/// A labelled value. Values can come from config, stored sessions or tools,
/// so every one is made inert here; already-safe text is unchanged.
fn field(appearance: &Appearance, name: &str, value: String) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{name:<12}"), appearance.style(Role::TextMuted)),
        Span::styled(safe_one_line(&value), appearance.style(Role::Text)),
    ])
}

fn activity_page(
    appearance: &Appearance,
    app: &App,
    view: &TranscriptView,
) -> (Vec<Line<'static>>, Vec<Line<'static>>) {
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
    let tools = app.next_shell.tool_rows.borrow();
    if tools.tools.is_empty() {
        lines.push(Line::from(Span::styled(
            "No tool activity in this turn.",
            appearance.style(Role::TextMuted),
        )));
    }
    let prefix = lines;
    let mut lines = Vec::new();
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
    (prefix, lines)
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
        Some(id) => format!(
            "{} (display name only; no identity contract binds it)",
            crate::application::view::terminal_safe(id)
        ),
        None => "none".to_string(),
    };
    let (ownership, ownership_detail) = app.ownership.describe(app.session_id.as_deref());
    vec![
        field(appearance, "Project", project),
        field(
            appearance,
            "Session",
            app.session_title
                .as_deref()
                .map_or_else(|| "untitled session".to_string(), safe_one_line),
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
            "Execution",
            format!("{ownership}: {ownership_detail}"),
        ),
        field(
            appearance,
            "Session id",
            app.session_id
                .as_deref()
                .map(crate::application::view::terminal_safe)
                .unwrap_or_else(|| "not bound".to_string()),
        ),
        field(
            appearance,
            "Renderer",
            "Next preview; unset COVEN_CODE_UI for Classic".to_string(),
        ),
    ]
}

/// What the Research page's rows are, and what it cannot show yet.
fn research_scope_notes(appearance: &Appearance) -> Vec<Line<'static>> {
    vec![
        primitives::status_message(
            appearance,
            Status::Evidence,
            "tool calls",
            "not a research job",
        ),
        primitives::status_message(
            appearance,
            Status::Blocked,
            "unavailable",
            "research jobs (#414)",
        ),
    ]
}

/// What the Memory page's rows are, and what it cannot do yet.
fn memory_scope_notes(appearance: &Appearance) -> Vec<Line<'static>> {
    vec![
        primitives::status_message(
            appearance,
            Status::Evidence,
            "read only",
            "daemon reads, no fallback",
        ),
        primitives::status_message(
            appearance,
            Status::Blocked,
            "unavailable",
            "reveal, selection (#415)",
        ),
    ]
}

fn memory_page(appearance: &Appearance, app: &App) -> Vec<Line<'static>> {
    use crate::application::memory::MemorySnapshot;
    use crate::application::view::terminal_safe;
    use claurst_core::coven_daemon::MemoryPreview;

    let safe = |text: &str| terminal_safe(text).replace('\n', " ");
    // `/memory` is a different, engine-local surface: instruction files.
    let not_here = field(
        appearance,
        "Not here",
        "/memory is Local instructions".to_string(),
    );
    let notice = |status, word: &str, detail: &str| {
        vec![
            primitives::status_message(appearance, status, word, detail),
            Line::default(),
            not_here.clone(),
        ]
    };
    let (rows, overview) = match &app.memory_snapshot {
        MemorySnapshot::NotRead => {
            return notice(Status::Pending, "reading", "asking the Coven daemon")
        }
        MemorySnapshot::Offline => {
            return notice(
                Status::Blocked,
                "offline",
                "daemon unreachable; no disk reads",
            )
        }
        MemorySnapshot::UpdateRequired => {
            return notice(
                Status::Blocked,
                "update required",
                "this Coven daemon has no memory read API",
            )
        }
        MemorySnapshot::Failed(reason) => return notice(Status::Blocked, "unavailable", reason),
        MemorySnapshot::Loaded { rows, overview } => (rows, overview),
    };

    let mut lines = Vec::new();
    if rows.is_empty() {
        lines.push(Line::from(Span::styled(
            "The daemon reports no familiar memory.".to_string(),
            appearance.style(Role::TextMuted),
        )));
        lines.push(Line::default());
    }
    // Rows first, so a small terminal still shows an entry.
    for row in rows {
        lines.push(Line::from(vec![
            Span::styled(
                "[familiar memory] ".to_string(),
                appearance.style(Role::TextMuted),
            ),
            Span::styled(safe(&row.familiar_id), appearance.style(Role::Text)),
            Span::styled(
                " (unverified)".to_string(),
                appearance.style(Role::TextMuted),
            ),
        ]));
        match &row.preview {
            MemoryPreview::Withheld => lines.push(field(
                appearance,
                "Preview",
                "withheld: not cleared by the producer".to_string(),
            )),
            MemoryPreview::Allowed { title, excerpt } => {
                let title = if title.trim().is_empty() {
                    "untitled".to_string()
                } else {
                    safe(title)
                };
                lines.push(field(appearance, "Title", title));
                if !excerpt.is_empty() {
                    lines.push(field(appearance, "Excerpt", safe(excerpt)));
                }
            }
        }
        lines.push(field(appearance, "Reference", safe(&row.id)));
        lines.push(field(
            appearance,
            "Updated",
            row.updated_at
                .as_deref()
                .map_or("unknown".to_string(), safe),
        ));
        lines.push(field(
            appearance,
            "Source",
            row.source_label
                .as_deref()
                .map_or("unknown".to_string(), safe),
        ));
        lines.push(field(
            appearance,
            "Check",
            format!(
                "{} (producer label, not attested)",
                safe(&row.verification_state)
            ),
        ));
        lines.push(Line::default());
    }
    let count = |value: Option<u64>| value.map_or("unknown".to_string(), |n| n.to_string());
    match overview {
        Ok(overview) => {
            lines.push(field(
                appearance,
                "Entries",
                format!(
                    "{} entries in {} familiars",
                    count(overview.entries),
                    count(overview.familiars)
                ),
            ));
            lines.push(field(
                appearance,
                "Index",
                format!(
                    "check {}; changes {}",
                    safe(overview.verification_state.as_deref().unwrap_or("unknown")),
                    if overview.mutations {
                        "offered by the daemon, not here"
                    } else {
                        "off"
                    },
                ),
            ));
        }
        Err(reason) => lines.push(primitives::status_message(
            appearance,
            Status::Blocked,
            "no overview",
            reason,
        )),
    }
    lines.push(not_here);
    lines
}

fn research_page(appearance: &Appearance, app: &App) -> Vec<Line<'static>> {
    use crate::application::research::{ResearchView, SourceOrigin, SourceOutcome};

    let interrupted: Vec<String> = app
        .interrupted_tools
        .iter()
        .map(|tool| tool.id.clone())
        .collect();
    let view = ResearchView::from_app(app, &interrupted);
    let mut lines = Vec::new();
    if view.items.is_empty() {
        lines.push(Line::from(Span::styled(
            "No web search or fetch in this session.".to_string(),
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
            Span::styled(item.request.clone(), appearance.style(Role::Text)),
        ]));
        let (status, word, detail) = match &item.outcome {
            SourceOutcome::Pending => (Status::Pending, "pending", "no result yet".to_string()),
            SourceOutcome::Returned => (
                Status::Evidence,
                "returned",
                "the tool returned without an error".to_string(),
            ),
            SourceOutcome::Failed(error) => (Status::Blocked, "failed", error.clone()),
            SourceOutcome::Interrupted => (
                Status::Pending,
                "interrupted",
                "the turn stopped before a result".to_string(),
            ),
            SourceOutcome::NoResult => {
                (Status::Pending, "unknown", "no result recorded".to_string())
            }
        };
        let mut state = Line::from(Span::styled(
            format!("{:<12}", "State"),
            appearance.style(Role::TextMuted),
        ));
        state
            .spans
            .extend(primitives::status_message(appearance, status, word, &detail).spans);
        lines.push(state);
        lines.push(field(appearance, "Reference", item.reference.clone()));
        if let Some(code) = item.http_status {
            lines.push(field(appearance, "HTTP", code.to_string()));
        }
        if let Some(retrieval) = &item.retrieval {
            use claurst_tools::web_fetch::Extraction;
            lines.push(field(
                appearance,
                "Evidence",
                "engine-reported retrieval".to_string(),
            ));
            lines.push(field(
                appearance,
                "Retrieved",
                retrieval.retrieved_at.clone(),
            ));
            lines.push(field(appearance, "Final URL", retrieval.final_url.clone()));
            lines.push(field(appearance, "SHA-256", retrieval.body_sha256.clone()));
            lines.push(field(
                appearance,
                "Body bytes",
                retrieval.body_bytes.to_string(),
            ));
            lines.push(field(
                appearance,
                "Media type",
                retrieval
                    .content_type
                    .clone()
                    .unwrap_or_else(|| "unknown".to_string()),
            ));
            let method = match retrieval.extraction {
                Extraction::PlainText => "plain text",
                Extraction::HtmlToText => "HTML to text",
                Extraction::ClaudeHaiku => "Claude Haiku (model-extracted, not verbatim)",
            };
            let cache = if retrieval.extraction_cached {
                "cached extraction; fresh HTTP body"
            } else {
                "fresh extraction"
            };
            lines.push(field(appearance, "Text", format!("{method}; {cache}")));
            if retrieval.text_truncated {
                lines.push(field(
                    appearance,
                    "Truncated",
                    format!(
                        "yes; {} original UTF-8 bytes",
                        retrieval.original_text_bytes
                    ),
                ));
            }
            if retrieval.utf8_replacements {
                lines.push(field(
                    appearance,
                    "Decoding",
                    "invalid UTF-8 replacement characters".to_string(),
                ));
            }
            if retrieval.url_credentials_redacted || retrieval.url_fragments_omitted {
                lines.push(field(
                    appearance,
                    "URL record",
                    format!(
                        "credentials redacted: {}; fragments omitted: {}",
                        retrieval.url_credentials_redacted, retrieval.url_fragments_omitted
                    ),
                ));
            }
        } else {
            lines.push(field(
                appearance,
                "Retrieved",
                "unknown time, final URL and content hash (#433)".to_string(),
            ));
        }
        if item.origin == SourceOrigin::Fetch
            && item.outcome == SourceOutcome::Returned
            && item.retrieval.is_none()
        {
            lines.push(field(
                appearance,
                "Text",
                "may be cached or model-extracted (#433)".to_string(),
            ));
        }
        if item.origin == SourceOrigin::Search
            && item.outcome == SourceOutcome::Returned
            && item.hits.is_empty()
        {
            lines.push(field(
                appearance,
                "Hits",
                "none parsed (output format not recognised)".to_string(),
            ));
        }
        for hit in &item.hits {
            lines.push(field(
                appearance,
                "Hit",
                format!("{}  {} (parsed, unverified)", hit.title, hit.url),
            ));
        }
        lines.push(Line::default());
    }
    lines
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
        {
            let mut line = Line::from(Span::styled(
                format!("  {:<10}", "Research"),
                appearance.style(Role::Text),
            ));
            line.spans.extend(
                primitives::status_message(
                    appearance,
                    Status::Pending,
                    "local only",
                    "Research page; research jobs wait on #414",
                )
                .spans,
            );
            line
        },
        {
            let mut line = Line::from(Span::styled(
                format!("  {:<10}", "Memory"),
                appearance.style(Role::Text),
            ));
            line.spans.extend(
                primitives::status_message(
                    appearance,
                    Status::Evidence,
                    "read only",
                    "Memory page; reveal and selection wait on #415",
                )
                .spans,
            );
            line
        },
        Line::default(),
    ];
    let palette = app
        .keybindings
        .reachable_chord("openCommandPalette")
        .map(|keys| format!("{keys} command palette, / suggestions"))
        .unwrap_or_else(|| "/ suggestions (no key opens the palette)".to_string());
    lines.push(field(appearance, "Commands", palette));
    lines.push(field(
        appearance,
        "Appearance",
        "/config appearance previews, saves and resets".to_string(),
    ));
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
        for grapheme in span.styled_graphemes(span.style) {
            let cells = unicode_width::UnicodeWidthStr::width(grapheme.symbol);
            if used + cells > width && used > 0 {
                if !text.is_empty() {
                    spans.push(Span::styled(std::mem::take(&mut text), span.style));
                }
                lines.push(Line::from(std::mem::take(&mut spans)));
                used = 0;
            }
            text.push_str(grapheme.symbol);
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
    if app.next_shell.selection_kept.get() {
        spans.push(Span::styled(
            format!(" {separator} selection kept"),
            appearance.style(Role::TextMuted),
        ));
    }
    let running = app.next_shell.tool_rows.borrow().running;
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
    fn wrapped_graphemes_and_composer_cursor_use_the_same_cell_width() {
        let text = "\u{1f469}\u{200d}\u{1f4bb}x";
        let wrapped = wrap_line(Line::from(text.to_string()), 2);
        assert_eq!(
            wrapped.iter().map(ToString::to_string).collect::<Vec<_>>(),
            ["\u{1f469}\u{200d}\u{1f4bb}", "x"]
        );
        assert_eq!(composer_cursor(text, text.len(), 4), (1, 1));
    }

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

#[cfg(test)]
mod transcript_rows_tests {
    use super::*;
    use tracked::Tracked;

    fn live_text(rows: &TranscriptRows) -> Vec<String> {
        rows.visible(&[], rows.messages_total, 10_000)
            .iter()
            .map(ToString::to_string)
            .collect()
    }

    #[test]
    fn visible_rows_skip_display_empty_history_groups() {
        let presentation = NextPresentation::default();
        let mut messages: Tracked<Vec<Message>> = vec![Message::user("first")].into();
        for _ in 0..100_000 {
            messages.push(Message::assistant(""));
        }
        messages.push(Message::assistant("last"));
        let live = "live".into();
        let mut rows = TranscriptRows::default();
        rows.sync(&messages, &live, &presentation, 80);
        let all = rows
            .visible(&[], 0, 24)
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>();
        assert!(all.iter().any(|line| line == "first"));
        assert!(all.iter().any(|line| line == "last"));
        assert!(all.iter().any(|line| line == "live"));
        assert!(
            rows.visited_groups.get() <= 3,
            "visited {} groups",
            rows.visited_groups.get()
        );
    }

    #[test]
    fn only_a_changed_message_is_rebuilt() {
        let presentation = NextPresentation::default();
        let mut messages: Tracked<Vec<Message>> = vec![
            Message::user("first"),
            Message::assistant("second"),
            Message::assistant("third"),
        ]
        .into();
        let live: Tracked<String> = String::new().into();
        let mut rows = TranscriptRows::default();
        rows.sync(&messages, &live, &presentation, 40);
        let kept = [rows.messages[0].1.as_ptr(), rows.messages[2].1.as_ptr()];
        let stamp = rows.messages_stamp.clone();
        rows.sync(&messages, &live, &presentation, 40);
        assert_eq!(rows.messages_stamp, stamp);
        messages[1] = Message::assistant("SECOND");
        rows.sync(&messages, &live, &presentation, 40);
        assert_eq!(rows.messages[0].1.as_ptr(), kept[0]);
        assert_eq!(rows.messages[2].1.as_ptr(), kept[1]);
        assert!(rows.messages[1]
            .1
            .iter()
            .map(ToString::to_string)
            .collect::<String>()
            .contains("SECOND"));
        rows.sync(&messages, &live, &presentation, 20);
        assert!(rows.total(0) > 0);
        assert!(rows.visible(&[], rows.total(0), 5).is_empty());
    }

    #[test]
    fn visible_rows_cross_messages_and_reach_the_live_rows() {
        let presentation = NextPresentation::default();
        let messages = vec![Message::user("one"), Message::assistant("two")].into();
        let live = "live\npart".into();
        let mut rows = TranscriptRows::default();
        rows.sync(&messages, &live, &presentation, 40);
        let tools = vec![Line::from("tool")];
        let total = rows.total(tools.len());
        let all = rows
            .visible(&tools, 0, total)
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>();
        assert_eq!(all.len(), total);
        assert_eq!(all[total - 3..], ["tool", "live", "part"]);
        for start in 0..=total {
            for count in [0, 1, 2, 5, total] {
                assert_eq!(
                    rows.visible(&tools, start, count)
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>(),
                    all.iter()
                        .skip(start)
                        .take(count)
                        .cloned()
                        .collect::<Vec<_>>()
                );
            }
        }
    }

    #[test]
    fn a_stream_shows_the_same_rows_however_it_arrives() {
        let presentation = NextPresentation::default();
        let messages = Vec::<Message>::new().into();
        // Include width-changing selectors and ZWJ across chunk boundaries.
        let text = "alpha\u{1b}]52;c;eA\u{7}\r\n\n漢字 \u{1f469}\u{200d}\u{1f4bb} ".to_string()
            + &"wide 漢字 text ".repeat(9)
            + "\nlast line\n\ntail \u{9b}31m end";
        let mut whole = TranscriptRows::default();
        whole.sync(&messages, &text.clone().into(), &presentation, 17);
        for step in [1, 2, 3, 7, 40] {
            let mut rows = TranscriptRows::default();
            let mut live: Tracked<String> = String::new().into();
            let mut from = 0;
            for cut in text
                .char_indices()
                .map(|(index, _)| index)
                .step_by(step)
                .chain([text.len()])
            {
                live.push_str(&text[from..cut]);
                rows.sync(&messages, &live, &presentation, 17);
                from = cut;
            }
            assert_eq!(live_text(&rows), live_text(&whole), "step {step}");
        }
    }

    #[test]
    fn chunk_boundaries_match_fresh_wrapping_for_context_sensitive_graphemes() {
        let presentation = NextPresentation::default();
        let messages = Vec::<Message>::new().into();
        let alphabet = [
            "a",
            " ",
            "\r",
            "\t",
            "漢",
            "\u{1f469}",
            "\u{200d}",
            "\u{1f4bb}",
            "\u{fe0e}",
            "\u{fe0f}",
            "\u{301}",
            "\u{1f1fa}",
            "\u{1f1f8}",
            "ل",
            "ا",
            "\n",
        ];
        let mut seed = 17_u64;
        let mut chunks = Vec::new();
        for _ in 0..128 {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            chunks.push(alphabet[((seed >> 32) as usize) % alphabet.len()]);
        }
        for width in [1, 2, 3, 4, 7, 17, 40] {
            let mut rows = TranscriptRows::default();
            let mut live: Tracked<String> = String::new().into();
            for chunk in &chunks {
                live.push_str(chunk);
                rows.sync(&messages, &live, &presentation, width);
                let mut fresh = TranscriptRows::default();
                fresh.sync(&messages, &live.clone().into(), &presentation, width);
                assert_eq!(
                    live_text(&rows),
                    live_text(&fresh),
                    "width {width}, source {:?}",
                    live.as_str()
                );
            }
        }
    }

    #[test]
    fn stream_rows_are_kept_as_it_grows_and_dropped_when_it_is_replaced() {
        let presentation = NextPresentation::default();
        let messages = Vec::<Message>::new().into();
        let mut live: Tracked<String> = "one\ntwo\nthr".into();
        let mut rows = TranscriptRows::default();
        rows.sync(&messages, &live, &presentation, 40);
        let first = rows.live_rows.get(&0).unwrap() as *const Line<'static>;
        live.push_str("ee");
        rows.sync(&messages, &live, &presentation, 40);
        assert_eq!(
            rows.live_rows.get(&0).unwrap() as *const Line<'static>,
            first
        );
        assert_eq!(live_text(&rows), ["one", "two", "three"]);
        live = "uno\ndos\ntres".into();
        rows.sync(&messages, &live, &presentation, 40);
        assert_eq!(live_text(&rows), ["uno", "dos", "tres"]);
        live.clear();
        rows.sync(&messages, &live, &presentation, 40);
        assert!(live_text(&rows).is_empty());
        live.push_str("abcdefgh\nx");
        rows.sync(&messages, &live, &presentation, 40);
        rows.sync(&messages, &live, &presentation, 4);
        assert_eq!(live_text(&rows), ["abcd", "efgh", "x"]);
    }
}
