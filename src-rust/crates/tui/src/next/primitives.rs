//! Shared presentation primitives for the Next renderer.
//!
//! Every Next surface composes these instead of styling cells itself, so
//! coding, tasks, research and memory share one visual language. Each
//! primitive takes an [`Appearance`] and returns ratatui lines whose meaning
//! survives monochrome and ASCII terminals: status always carries a word and
//! a shape, selection and focus carry a marker, and no glyph depends on Nerd
//! Fonts, emoji width or image protocols.

use super::style::{Appearance, Glyphs, Motion, Role};
use ratatui::text::{Line, Span};

/// Marks drawn by primitives, with a Unicode and an ASCII form.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mark {
    /// Identity bar before the product label.
    Identity,
    Selected,
    Unselected,
    Collapsed,
    Expanded,
    Prompt,
    Crumb,
    /// Inline separator between related items.
    Separator,
    Rule,
    Verified,
    Pending,
    Blocked,
    Evidence,
    /// Diff line markers; the sign carries the meaning without color.
    Added,
    Removed,
}

impl Mark {
    pub fn glyph(self, glyphs: Glyphs) -> &'static str {
        let (unicode, ascii) = match self {
            Mark::Identity => ("▍", "|"),
            Mark::Selected => ("›", ">"),
            Mark::Unselected => (" ", " "),
            Mark::Collapsed => ("▸", "[+]"),
            Mark::Expanded => ("▾", "[-]"),
            Mark::Prompt => ("›", ">"),
            Mark::Crumb => ("/", "/"),
            Mark::Separator => ("·", "-"),
            Mark::Rule => ("─", "-"),
            Mark::Verified => ("✓", "[ok]"),
            Mark::Pending => ("…", "[..]"),
            Mark::Blocked => ("✕", "[x]"),
            Mark::Evidence => ("○", "[?]"),
            Mark::Added => ("+", "+"),
            Mark::Removed => ("-", "-"),
        };
        match glyphs {
            Glyphs::Unicode => unicode,
            Glyphs::Ascii => ascii,
        }
    }
}

/// The four Brand status semantics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Verified,
    Pending,
    Blocked,
    /// Neutral receipt, provenance or explicitly unknown state.
    Evidence,
}

impl Status {
    fn role(self) -> Role {
        match self {
            Status::Verified => Role::Verified,
            Status::Pending => Role::Pending,
            Status::Blocked => Role::Blocked,
            Status::Evidence => Role::Evidence,
        }
    }

    fn mark(self) -> Mark {
        match self {
            Status::Verified => Mark::Verified,
            Status::Pending => Mark::Pending,
            Status::Blocked => Mark::Blocked,
            Status::Evidence => Mark::Evidence,
        }
    }
}

fn span(appearance: &Appearance, role: Role, text: impl Into<String>) -> Span<'static> {
    Span::styled(text.into(), appearance.style(role))
}

/// Product header: identity mark plus the plain-text `Coven` label (the
/// canonical mark is SVG-only, so cells never imitate it).
pub fn header(appearance: &Appearance, suffix: &str) -> Line<'static> {
    Line::from(vec![
        span(
            appearance,
            Role::Identity,
            Mark::Identity.glyph(appearance.glyphs),
        ),
        span(appearance, Role::Text, " Coven"),
        span(
            appearance,
            Role::TextMuted,
            format!(" {} {suffix}", Mark::Separator.glyph(appearance.glyphs)),
        ),
    ])
}

/// Speaker label above a transcript entry.
pub fn transcript_label(appearance: &Appearance, speaker: &str) -> Line<'static> {
    Line::from(span(appearance, Role::TextMuted, speaker.to_string()))
}

/// Status label: shape and word together, never color alone.
pub fn status_label(appearance: &Appearance, status: Status, word: &str) -> Vec<Span<'static>> {
    let style_role = status.role();
    vec![span(
        appearance,
        style_role,
        format!("{} {word}", status.mark().glyph(appearance.glyphs)),
    )]
}

/// One-line tool disclosure row.
pub fn tool_row(
    appearance: &Appearance,
    name: &str,
    status: Status,
    word: &str,
    expanded: bool,
) -> Line<'static> {
    let disclosure = if expanded {
        Mark::Expanded
    } else {
        Mark::Collapsed
    };
    let mut spans = vec![
        span(
            appearance,
            Role::TextSubtle,
            format!("{} ", disclosure.glyph(appearance.glyphs)),
        ),
        span(appearance, Role::Text, format!("{name} ")),
    ];
    spans.extend(status_label(appearance, status, word));
    Line::from(spans)
}

/// A list row with a selection marker that does not depend on color.
pub fn list_item(appearance: &Appearance, text: &str, selected: bool) -> Line<'static> {
    let (mark, mark_role, role) = if selected {
        (Mark::Selected, Role::Focus, Role::Selection)
    } else {
        (Mark::Unselected, Role::Text, Role::Text)
    };
    Line::from(vec![
        span(appearance, mark_role, mark.glyph(appearance.glyphs)),
        span(appearance, Role::Text, " "),
        span(appearance, role, text.to_string()),
    ])
}

/// Composer input lines: the first carries the prompt glyph (the only part
/// with the focus style), continuation lines align under the text.
pub fn composer(appearance: &Appearance, draft: &str) -> Vec<Line<'static>> {
    draft
        .split('\n')
        .enumerate()
        .map(|(index, text)| {
            let lead = if index == 0 {
                span(
                    appearance,
                    Role::Focus,
                    Mark::Prompt.glyph(appearance.glyphs),
                )
            } else {
                span(appearance, Role::Text, " ")
            };
            Line::from(vec![lead, span(appearance, Role::Text, format!(" {text}"))])
        })
        .collect()
}

/// One line of a diff: the `+`/`-` marker carries the meaning, the color
/// only reinforces it.
pub fn diff_line(appearance: &Appearance, added: bool, text: &str) -> Line<'static> {
    let (mark, role) = if added {
        (Mark::Added, Role::DiffAdded)
    } else {
        (Mark::Removed, Role::DiffRemoved)
    };
    Line::from(span(
        appearance,
        role,
        format!("{}{text}", mark.glyph(appearance.glyphs)),
    ))
}

/// Horizontal rule of `width` cells.
pub fn rule(appearance: &Appearance, width: u16) -> Line<'static> {
    let glyph = Mark::Rule.glyph(appearance.glyphs);
    Line::from(span(
        appearance,
        Role::Border,
        glyph.repeat(usize::from(width)),
    ))
}

/// Location breadcrumbs; the last part is the current location.
pub fn breadcrumbs(appearance: &Appearance, parts: &[&str]) -> Line<'static> {
    let separator = format!(" {} ", Mark::Crumb.glyph(appearance.glyphs));
    let mut spans = Vec::new();
    for (index, part) in parts.iter().enumerate() {
        if index > 0 {
            spans.push(span(appearance, Role::TextSubtle, separator.clone()));
        }
        let role = if index + 1 == parts.len() {
            Role::Text
        } else {
            Role::TextMuted
        };
        spans.push(span(appearance, role, part.to_string()));
    }
    Line::from(spans)
}

/// A status message with its semantic label.
pub fn status_message(
    appearance: &Appearance,
    status: Status,
    word: &str,
    detail: &str,
) -> Line<'static> {
    let mut spans = status_label(appearance, status, word);
    if !detail.is_empty() {
        spans.push(span(appearance, Role::TextMuted, format!(" {detail}")));
    }
    Line::from(spans)
}

/// A confirmation prompt with keyed options; the selected option carries a
/// marker as well as the selection style.
pub fn confirmation(
    appearance: &Appearance,
    prompt: &str,
    options: &[(char, &str)],
    selected: usize,
    sensitive: bool,
) -> Vec<Line<'static>> {
    let prompt_role = if sensitive {
        Role::Sensitive
    } else {
        Role::Text
    };
    let mut lines = vec![Line::from(span(
        appearance,
        prompt_role,
        prompt.to_string(),
    ))];
    for (index, (key, label)) in options.iter().enumerate() {
        lines.push(list_item(
            appearance,
            &format!("[{key}] {label}"),
            index == selected,
        ));
    }
    lines
}

/// Error with a recovery hint.
pub fn error_recovery(
    appearance: &Appearance,
    message: &str,
    recovery: &str,
) -> Vec<Line<'static>> {
    vec![
        status_message(appearance, Status::Blocked, "error", message),
        Line::from(span(appearance, Role::TextMuted, recovery.to_string())),
    ]
}

/// Title row of the single contextual inspector.
pub fn inspector_title(appearance: &Appearance, title: &str) -> Line<'static> {
    Line::from(vec![
        span(
            appearance,
            Role::Border,
            format!("{} ", Mark::Rule.glyph(appearance.glyphs)),
        ),
        span(appearance, Role::Text, title.to_string()),
    ])
}

/// Bounded activity indicator for an active operation. Idle state has no
/// indicator at all; reduced motion is static.
pub fn activity(appearance: &Appearance, frame: u64) -> &'static str {
    const UNICODE: [&str; 4] = ["◐", "◓", "◑", "◒"];
    const ASCII: [&str; 4] = ["|", "/", "-", "\\"];
    if appearance.motion == Motion::Reduced {
        return match appearance.glyphs {
            Glyphs::Unicode => "•",
            Glyphs::Ascii => "*",
        };
    }
    // Advance at most once per 8 frames so the indicator stays calm.
    let index = usize::try_from((frame / 8) % 4).unwrap_or(0);
    match appearance.glyphs {
        Glyphs::Unicode => UNICODE[index],
        Glyphs::Ascii => ASCII[index],
    }
}
