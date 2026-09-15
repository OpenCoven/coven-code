//! Markdown -> ratatui lines renderer used by transcript message families.

use crate::figures;
use crate::overlays::{COVEN_CODE_ACCENT, COVEN_CODE_MUTED, COVEN_CODE_PANEL_BG, COVEN_CODE_TEXT};
use once_cell::sync::Lazy;
use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};
use regex::Regex;
use unicode_width::UnicodeWidthStr;

/// Regex pattern to detect URLs (http://, https://, ftp://, www.)
static URL_PATTERN: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?:https?|ftp)://\S+|www\.\S+").expect("Invalid URL regex pattern"));

/// Regex pattern to detect email addresses
static EMAIL_PATTERN: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\.[a-zA-Z]{2,}")
        .expect("Invalid email regex pattern")
});

/// Columns consumed by a code row's `"  │ "` gutter.
const CODE_GUTTER_WIDTH: usize = 4;

/// Normalize an author-supplied fence tag into a short chip label.
///
/// The tag comes from model output, so anything outside a conservative
/// identifier set is rejected rather than rendered, and the result is length
/// capped so a long tag cannot eat the rule.
fn code_block_label(tag: &str) -> Option<String> {
    const MAX_LABEL_CHARS: usize = 12;
    let accepted =
        |ch: char| ch.is_ascii_alphanumeric() || matches!(ch, '+' | '-' | '#' | '.' | '_');
    if tag.is_empty() || tag.chars().count() > MAX_LABEL_CHARS || !tag.chars().all(accepted) {
        return None;
    }
    Some(tag.to_ascii_uppercase())
}

/// Fit `spans` to exactly `width` columns: clip anything that overruns the
/// frame, then pad the remainder with surface-colored blanks so a code row
/// reads as one continuous slab.
///
/// Both halves matter. Without the pad, ratatui paints the background only as
/// far as the glyphs reach and the block's right edge follows the ragged shape
/// of the code. Without the clip, a narrow frame overflows — a code row's
/// gutter alone is four columns, and an opening rule's chip is wider still.
fn code_block_line(spans: Vec<Span<'static>>, width: u16) -> Line<'static> {
    let target = width as usize;
    let mut fitted: Vec<Span<'static>> = Vec::with_capacity(spans.len() + 1);
    let mut used = 0usize;

    for span in spans {
        if used >= target {
            break;
        }
        let span_width = UnicodeWidthStr::width(span.content.as_ref());
        if used + span_width <= target {
            used += span_width;
            fitted.push(span);
            continue;
        }
        let clipped = clip_to_width(span.content.as_ref(), target - used);
        used = target;
        fitted.push(Span::styled(clipped, span.style));
        break;
    }

    if used < target {
        fitted.push(Span::styled(
            " ".repeat(target - used),
            Style::default().bg(COVEN_CODE_PANEL_BG),
        ));
    }
    Line::from(fitted)
}

/// Hard-clip to `max_width` display columns with no marker. Used for chrome
/// (rules, gutters) where an `…` would be noise; body text uses
/// `truncate_to_width` instead, which marks the cut.
fn clip_to_width(value: &str, max_width: usize) -> String {
    let mut out = String::new();
    let mut used = 0usize;
    for ch in value.chars() {
        let w = UnicodeWidthStr::width(ch.encode_utf8(&mut [0u8; 4]) as &str);
        if used + w > max_width {
            break;
        }
        used += w;
        out.push(ch);
    }
    out
}

/// Top or bottom edge of a fenced block.
///
/// `open` picks the edge glyph and `label` carries the language chip. They are
/// independent: an untagged fence still opens with `┌`, it just has no chip.
fn code_block_rule(open: bool, label: Option<String>, width: u16) -> Line<'static> {
    let rule_style = Style::default()
        .fg(COVEN_CODE_MUTED)
        .bg(COVEN_CODE_PANEL_BG);
    let mut spans = vec![Span::styled("  ", Style::default().bg(COVEN_CODE_PANEL_BG))];
    match label {
        Some(label) if open => {
            spans.push(Span::styled("┌─ ", rule_style));
            spans.push(Span::styled(
                label,
                Style::default()
                    .fg(COVEN_CODE_ACCENT)
                    .bg(COVEN_CODE_PANEL_BG)
                    .add_modifier(Modifier::BOLD),
            ));
            spans.push(Span::styled(" ", rule_style));
        }
        _ => spans.push(Span::styled(if open { "┌" } else { "└" }, rule_style)),
    }
    let used: usize = spans
        .iter()
        .map(|span| UnicodeWidthStr::width(span.content.as_ref()))
        .sum();
    let remaining = (width as usize).saturating_sub(used);
    if remaining > 0 {
        spans.push(Span::styled("─".repeat(remaining), rule_style));
    }
    code_block_line(spans, width)
}

/// One body row of a fenced block, truncated to the frame rather than left to
/// run off the right edge.
fn code_block_row(raw: &str, width: u16) -> Line<'static> {
    let budget = (width as usize).saturating_sub(CODE_GUTTER_WIDTH);
    let visible = truncate_to_width(raw, budget);
    code_block_line(
        vec![
            Span::styled(
                "  │ ",
                Style::default()
                    .fg(COVEN_CODE_MUTED)
                    .bg(COVEN_CODE_PANEL_BG),
            ),
            Span::styled(
                visible,
                Style::default().fg(COVEN_CODE_TEXT).bg(COVEN_CODE_PANEL_BG),
            ),
        ],
        width,
    )
}

/// Truncate to `max_width` display columns, marking the cut with `…` so a
/// clipped line is distinguishable from a complete one.
fn truncate_to_width(value: &str, max_width: usize) -> String {
    if max_width == 0 {
        return String::new();
    }
    if UnicodeWidthStr::width(value) <= max_width {
        return value.to_string();
    }
    if max_width == 1 {
        return "…".to_string();
    }
    let budget = max_width - 1;
    let mut out = String::new();
    let mut used = 0usize;
    for ch in value.chars() {
        let w = UnicodeWidthStr::width(ch.encode_utf8(&mut [0u8; 4]) as &str);
        if used + w > budget {
            break;
        }
        used += w;
        out.push(ch);
    }
    out.push('…');
    out
}

/// Render markdown text to styled ratatui lines.
pub fn render_markdown(text: &str, width: u16) -> Vec<Line<'static>> {
    let all_lines: Vec<&str> = text.lines().collect();
    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut in_code_block = false;
    let mut code_lang = String::new();
    let mut idx = 0;

    while idx < all_lines.len() {
        let raw = all_lines[idx];
        if raw.trim_start().starts_with("```") {
            if in_code_block {
                lines.push(code_block_rule(false, None, width));
                in_code_block = false;
                code_lang.clear();
            } else {
                in_code_block = true;
                code_lang = raw.trim_start().trim_start_matches('`').trim().to_string();
                lines.push(code_block_rule(true, code_block_label(&code_lang), width));
            }
            idx += 1;
            continue;
        }

        if in_code_block {
            lines.push(code_block_row(raw, width));
            idx += 1;
            continue;
        }

        // Check for markdown tables
        if let Some((table, end_idx)) = super::markdown_enhanced::detect_table(&all_lines, idx) {
            lines.extend(super::markdown_enhanced::render_table(&table));
            idx = end_idx;
            continue;
        }

        if let Some(quoted) = raw.strip_prefix("> ") {
            lines.push(Line::from(vec![
                Span::styled(
                    format!("  {} ", figures::BLOCKQUOTE_BAR),
                    Style::default().fg(Color::DarkGray),
                ),
                Span::styled(quoted.to_string(), Style::default().fg(Color::DarkGray)),
            ]));
            idx += 1;
            continue;
        }

        if let Some(heading) = raw.strip_prefix("### ") {
            lines.push(Line::from(vec![Span::styled(
                format!("  {}", heading),
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            )]));
            idx += 1;
            continue;
        }
        if let Some(heading) = raw.strip_prefix("## ") {
            lines.push(Line::from(vec![Span::styled(
                format!("  {}", heading),
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            )]));
            idx += 1;
            continue;
        }
        if let Some(heading) = raw.strip_prefix("# ") {
            lines.push(Line::from(vec![Span::styled(
                format!("  {}", heading),
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD | Modifier::ITALIC | Modifier::UNDERLINED),
            )]));
            idx += 1;
            continue;
        }

        let padded = format!("  {}", raw);
        let effective_width = width.saturating_sub(4) as usize;
        for wrapped_line in word_wrap(&padded, effective_width) {
            let spans = parse_inline_spans(wrapped_line);
            lines.push(Line::from(spans));
        }

        idx += 1;
    }

    if in_code_block {
        lines.push(Line::from(vec![Span::styled(
            "  └──────────────────────────────────────────────────".to_string(),
            Style::default().fg(Color::Yellow),
        )]));
    }

    lines
}

/// Split plain text into spans with URL/email detection and styling.
/// URLs and emails are styled with cyan color and underline.
fn split_and_style_links(text: &str) -> Vec<Span<'static>> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut last_end = 0;

    // Check for URLs first
    for url_match in URL_PATTERN.find_iter(text) {
        let match_start = url_match.start();
        let match_end = url_match.end();

        // Add text before the URL
        if match_start > last_end {
            spans.push(Span::raw(text[last_end..match_start].to_string()));
        }

        // Add the URL with special styling (cyan with underline)
        let url_text = url_match.as_str();
        spans.push(Span::styled(
            url_text.to_string(),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::UNDERLINED),
        ));
        last_end = match_end;
    }

    // Check for emails in remaining text (only if no URLs were found)
    if last_end == 0 {
        for email_match in EMAIL_PATTERN.find_iter(text) {
            let match_start = email_match.start();
            let match_end = email_match.end();

            // Add text before the email
            if match_start > last_end {
                spans.push(Span::raw(text[last_end..match_start].to_string()));
            }

            // Add the email with special styling (cyan with underline)
            let email_text = email_match.as_str();
            spans.push(Span::styled(
                email_text.to_string(),
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::UNDERLINED),
            ));
            last_end = match_end;
        }
    }

    // Add any remaining text
    if last_end < text.len() {
        spans.push(Span::raw(text[last_end..].to_string()));
    }

    // If no links/emails were found, return a simple raw span
    if spans.is_empty() {
        spans.push(Span::raw(text.to_string()));
    }

    spans
}

fn parse_inline_spans(text: String) -> Vec<Span<'static>> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut remaining = text.as_str();

    while !remaining.is_empty() {
        let bold_pos = remaining.find("**");
        let code_pos = remaining.find('`');

        match (bold_pos, code_pos) {
            (None, None) => {
                // No more formatting, but check for links/emails in plain text
                spans.extend(split_and_style_links(remaining));
                break;
            }
            (Some(b), Some(c)) if c < b => {
                // Code block comes first
                if c > 0 {
                    spans.extend(split_and_style_links(&remaining[..c]));
                }
                let after_tick = &remaining[c + 1..];
                if let Some(end) = after_tick.find('`') {
                    spans.push(Span::styled(
                        after_tick[..end].to_string(),
                        Style::default().fg(Color::Yellow),
                    ));
                    remaining = &after_tick[end + 1..];
                } else {
                    spans.push(Span::raw(remaining[c..].to_string()));
                    break;
                }
            }
            (Some(b), _) => {
                // Bold comes first
                if b > 0 {
                    spans.extend(split_and_style_links(&remaining[..b]));
                }
                let after_stars = &remaining[b + 2..];
                if let Some(end) = after_stars.find("**") {
                    spans.push(Span::styled(
                        after_stars[..end].to_string(),
                        Style::default().add_modifier(Modifier::BOLD),
                    ));
                    remaining = &after_stars[end + 2..];
                } else {
                    // Unmatched opening `**` — skip the markers and render the
                    // rest as plain text.  This prevents literal `**` appearing
                    // at the end of reasoning blocks when the model ends a
                    // thought mid-bold, or when word-wrap splits a bold span
                    // across lines.
                    spans.extend(split_and_style_links(after_stars));
                    break;
                }
            }
            (None, Some(c)) => {
                // Code block (no bold)
                if c > 0 {
                    spans.extend(split_and_style_links(&remaining[..c]));
                }
                let after_tick = &remaining[c + 1..];
                if let Some(end) = after_tick.find('`') {
                    spans.push(Span::styled(
                        after_tick[..end].to_string(),
                        Style::default().fg(Color::Yellow),
                    ));
                    remaining = &after_tick[end + 1..];
                } else {
                    spans.push(Span::raw(remaining[c..].to_string()));
                    break;
                }
            }
        }
    }

    if spans.is_empty() {
        spans.push(Span::raw(String::new()));
    }
    spans
}

fn word_wrap(text: &str, width: usize) -> Vec<String> {
    if width == 0 || UnicodeWidthStr::width(text) <= width {
        return vec![text.to_string()];
    }

    let mut result = Vec::new();
    let mut current_line = String::new();
    let mut current_width = 0usize;

    let push_long_word = |word: &str,
                          result: &mut Vec<String>,
                          current_line: &mut String,
                          current_width: &mut usize| {
        // Hard-break a word that on its own exceeds `width` (e.g. URLs).
        if !current_line.is_empty() {
            result.push(std::mem::take(current_line));
            *current_width = 0;
        }
        let mut chunk = String::new();
        let mut chunk_w = 0usize;
        for ch in word.chars() {
            let cw = UnicodeWidthStr::width(ch.to_string().as_str());
            if chunk_w + cw > width && !chunk.is_empty() {
                result.push(std::mem::take(&mut chunk));
                chunk_w = 0;
            }
            chunk.push(ch);
            chunk_w += cw;
        }
        if !chunk.is_empty() {
            *current_line = chunk;
            *current_width = chunk_w;
        }
    };

    for word in text.split_whitespace() {
        let word_w = UnicodeWidthStr::width(word);
        if word_w > width {
            push_long_word(word, &mut result, &mut current_line, &mut current_width);
            continue;
        }
        if current_width == 0 {
            current_line.push_str(word);
            current_width = word_w;
        } else if current_width + 1 + word_w <= width {
            current_line.push(' ');
            current_line.push_str(word);
            current_width += 1 + word_w;
        } else {
            result.push(std::mem::take(&mut current_line));
            current_line.push_str(word);
            current_width = word_w;
        }
    }

    if !current_line.is_empty() {
        result.push(current_line);
    }
    if result.is_empty() {
        result.push(text.to_string());
    }
    result
}

#[cfg(test)]
mod code_block_tests {
    use super::*;

    fn row_text(line: &Line<'_>) -> String {
        line.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    fn row_width(line: &Line<'_>) -> usize {
        line.spans
            .iter()
            .map(|s| UnicodeWidthStr::width(s.content.as_ref()))
            .sum()
    }

    #[test]
    fn fenced_block_rows_fill_the_frame_on_one_surface() {
        let lines = render_markdown("```rust\nfn main() {}\n```", 40);
        assert_eq!(lines.len(), 3, "open rule, body, close rule");

        for line in &lines {
            assert_eq!(
                row_width(line),
                40,
                "row padded to the frame: {:?}",
                row_text(line)
            );
            assert!(
                line.spans
                    .iter()
                    .all(|s| s.style.bg == Some(COVEN_CODE_PANEL_BG)),
                "every span sits on the code surface: {:?}",
                row_text(line)
            );
        }
    }

    #[test]
    fn opening_rule_carries_an_uppercase_language_chip() {
        let lines = render_markdown("```rust\nx\n```", 40);
        let opening = row_text(&lines[0]);
        assert!(opening.contains("RUST"), "chip present: {opening:?}");
        assert!(
            lines[0]
                .spans
                .iter()
                .find(|s| s.content.as_ref() == "RUST")
                .is_some_and(|s| s.style.add_modifier.contains(Modifier::BOLD)),
            "chip carries weight"
        );
    }

    #[test]
    fn untagged_fence_still_opens_with_the_opening_edge() {
        let lines = render_markdown("```\nx\n```", 40);
        let opening = row_text(&lines[0]);
        let closing = row_text(&lines[2]);

        assert!(
            opening.starts_with("  ┌"),
            "an untagged fence opens with the opening edge, not the closing one: {opening:?}"
        );
        assert!(
            closing.starts_with("  └"),
            "and closes with the closing edge: {closing:?}"
        );
        assert!(
            !opening.chars().any(|c| c.is_ascii_alphabetic()),
            "no chip on an untagged fence: {opening:?}"
        );
    }

    #[test]
    fn tagged_fence_opens_and_closes_with_matching_edges() {
        let lines = render_markdown("```rust\nx\n```", 40);
        assert!(row_text(&lines[0]).starts_with("  ┌"));
        assert!(row_text(&lines[2]).starts_with("  └"));
    }

    #[test]
    fn untrusted_or_oversized_fence_tags_are_rejected() {
        // The fence tag is model output.
        for tag in ["\u{1b}[31mred", "c++/cli", &"a".repeat(64)] {
            assert_eq!(
                code_block_label(tag),
                None,
                "tag {tag:?} must not become a chip"
            );
        }
        assert_eq!(code_block_label("ts"), Some("TS".to_string()));
    }

    #[test]
    fn long_code_lines_are_truncated_instead_of_clipped() {
        // Before: the raw line was pushed verbatim and ratatui clipped it at
        // the buffer edge with no marker, so a truncated line was
        // indistinguishable from a complete one.
        let long = "x".repeat(200);
        let lines = render_markdown(&format!("```\n{long}\n```"), 40);
        let body = row_text(&lines[1]);

        assert_eq!(
            row_width(&lines[1]),
            40,
            "body still fills exactly one frame"
        );
        assert!(body.contains('…'), "the cut is marked: {body:?}");
    }

    #[test]
    fn narrow_frames_do_not_panic_or_overflow() {
        for width in [0u16, 1, 2, 4, 5] {
            let lines = render_markdown("```rust\nfn main() {}\n```", width);
            for line in &lines {
                assert!(
                    row_width(line) <= width as usize,
                    "width {width} row overflows: {:?}",
                    row_text(line)
                );
            }
        }
    }
}
