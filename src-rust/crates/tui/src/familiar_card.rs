//! Static themed familiar card composer.
//!
//! Given a [`crate::familiar_theme::FamiliarTheme`] and a [`CardSize`], produces
//! the lines that render the active familiar in the welcome panel, the F2
//! switcher, and the `/agents` detail view. The glyph itself never animates;
//! only the eye row spins while the assistant is in the `Loading` state, so
//! the user still has a signal that work is in progress.
//!
//! Built-in archetypes dispatch to the pixel-art builders in
//! [`crate::rustle`]. Procedural archetypes ([`Archetype::SigilCrystal`] etc.)
//! draw a colored frame around the familiar's emoji so any user-defined entry
//! from `~/.coven/familiars.toml` gets first-class visual identity.

use crate::familiar_theme::{Archetype, FamiliarPalette, FamiliarTheme};
use crate::rustle::{archetype_lines, RustlePose};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

/// Card size selection. Welcome panel picks via [`pick_size`]; F2 and
/// `/agents` pin to a fixed size that matches their available room.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CardSize {
    /// Glyph only, no border, no text. Used when the host column is narrow.
    Compact,
    /// Bordered card with name + access tier dot in the title.
    Standard,
    /// Standard + role line + decorative accent rule.
    Large,
}

/// Pick a card size from the available column width.
///
/// Thresholds tuned for the welcome panel where `left_w` is clamped to 22-32:
/// most installs land in Standard, terminals with wide welcome panels see Large,
/// only very narrow terminals fall back to Compact.
pub fn pick_size(width: u16) -> CardSize {
    if width < 20 {
        CardSize::Compact
    } else if width < 28 {
        CardSize::Standard
    } else {
        CardSize::Large
    }
}

/// Render the full card. `loading` is `Some(frame_count)` to spin the eyes,
/// `None` for the resting state.
pub fn render_card(
    theme: &FamiliarTheme,
    size: CardSize,
    loading: Option<u64>,
) -> Vec<Line<'static>> {
    let pose = match loading {
        Some(frame) => RustlePose::Loading { frame },
        None => RustlePose::Static,
    };
    let glyph = glyph_lines(theme, &pose);

    match size {
        CardSize::Compact => glyph_only(glyph),
        CardSize::Standard => bordered(theme, glyph, false),
        CardSize::Large => bordered(theme, glyph, true),
    }
}

/// One-line preview used in the F2 switcher list.
///
/// Format: ` [glyph-token] name  · tier `, painted in the familiar's palette.
/// `width` is the popup interior column count; the row is left-trimmed to
/// fit without wrapping.
pub fn render_mini_row(theme: &FamiliarTheme, width: u16) -> Line<'static> {
    let primary = Style::default()
        .fg(theme.palette.primary)
        .add_modifier(Modifier::BOLD);
    let muted = Style::default().fg(Color::Rgb(148, 163, 184));
    let dot = Span::styled("\u{25cf}", Style::default().fg(theme.access_color()));
    let mut spans = vec![
        Span::raw(" "),
        Span::styled(theme.emoji.clone(), Style::default()),
        Span::raw(" "),
        Span::styled(theme.display_name.clone(), primary),
    ];
    if let Some(role) = &theme.role {
        spans.push(Span::raw("  "));
        spans.push(Span::styled(
            truncate(
                role,
                (width as usize).saturating_sub(theme.display_name.len() + 10),
            ),
            muted,
        ));
    }
    spans.push(Span::raw("  "));
    spans.push(dot);
    spans.push(Span::raw(" "));
    spans.push(Span::styled(short_tier(&theme.access).to_string(), muted));
    Line::from(spans)
}

// ── Layout helpers ───────────────────────────────────────────────────────────

fn glyph_only(glyph: Vec<Line<'static>>) -> Vec<Line<'static>> {
    glyph
}

fn bordered(
    theme: &FamiliarTheme,
    glyph: Vec<Line<'static>>,
    include_role: bool,
) -> Vec<Line<'static>> {
    let primary = theme.palette.primary;
    let inner_w = 22u16; // 11-wide glyph + 2 pad + ~9 right margin

    let mut out = Vec::with_capacity(glyph.len() + 5);

    // Top border with the name + tier dot as a title.
    out.push(title_line(theme, primary, inner_w));

    for line in glyph {
        out.push(wrap_line(line, primary, inner_w));
    }

    if include_role {
        out.push(blank_line(primary, inner_w));
        if let Some(role) = &theme.role {
            out.push(role_line(role, theme, primary, inner_w));
        }
        out.push(rule_line(primary, inner_w));
    }

    out.push(access_line(theme, primary, inner_w));
    out.push(bottom_border(primary, inner_w));
    out
}

fn title_line(theme: &FamiliarTheme, primary: Color, inner_w: u16) -> Line<'static> {
    let dot = Span::styled("\u{25cf}", Style::default().fg(theme.access_color()));
    let name = Span::styled(
        theme.display_name.clone(),
        Style::default().fg(primary).add_modifier(Modifier::BOLD),
    );
    let title_prefix = Span::styled("\u{256d} ".to_string(), Style::default().fg(primary));
    let title_gap = Span::raw(" ");
    // Approx title width: "╭ " (2) + dot (1) + " " + name + " ". Pad fill to inner_w.
    let used = 2 + 1 + 1 + theme.display_name.chars().count() + 1;
    let fill = (inner_w as usize).saturating_sub(used) + 1; // +1 to land on the corner
    let fill_str = "\u{2500}".repeat(fill);
    let suffix = Span::styled(
        format!("{}\u{256e}", fill_str),
        Style::default().fg(primary),
    );
    Line::from(vec![
        title_prefix,
        dot,
        Span::raw(" "),
        name,
        title_gap,
        suffix,
    ])
}

fn bottom_border(primary: Color, inner_w: u16) -> Line<'static> {
    let mid = "\u{2500}".repeat(inner_w as usize);
    Line::from(Span::styled(
        format!("\u{2570}{}\u{256f}", mid),
        Style::default().fg(primary),
    ))
}

fn wrap_line(content: Line<'static>, primary: Color, inner_w: u16) -> Line<'static> {
    let mut spans = Vec::with_capacity(content.spans.len() + 3);
    spans.push(Span::styled(
        "\u{2502}".to_string(),
        Style::default().fg(primary),
    ));
    let visible = visible_width(&content);
    let pad_left = 1usize;
    let pad_right = (inner_w as usize).saturating_sub(visible + pad_left);
    spans.push(Span::raw(" ".repeat(pad_left)));
    spans.extend(content.spans);
    spans.push(Span::raw(" ".repeat(pad_right)));
    spans.push(Span::styled(
        "\u{2502}".to_string(),
        Style::default().fg(primary),
    ));
    Line::from(spans)
}

fn blank_line(primary: Color, inner_w: u16) -> Line<'static> {
    Line::from(vec![
        Span::styled("\u{2502}".to_string(), Style::default().fg(primary)),
        Span::raw(" ".repeat(inner_w as usize)),
        Span::styled("\u{2502}".to_string(), Style::default().fg(primary)),
    ])
}

fn role_line(role: &str, theme: &FamiliarTheme, primary: Color, inner_w: u16) -> Line<'static> {
    let role_trimmed = truncate(role, (inner_w as usize).saturating_sub(2));
    let text = Span::styled(role_trimmed, Style::default().fg(theme.palette.accent));
    let used = visible_str_width(&text.content);
    let pad_right = (inner_w as usize).saturating_sub(used + 2);
    Line::from(vec![
        Span::styled("\u{2502}".to_string(), Style::default().fg(primary)),
        Span::raw("  "),
        text,
        Span::raw(" ".repeat(pad_right)),
        Span::styled("\u{2502}".to_string(), Style::default().fg(primary)),
    ])
}

fn rule_line(primary: Color, inner_w: u16) -> Line<'static> {
    let inner = "  \u{2500}\u{2500}\u{2500}".to_string();
    let used = 5usize;
    let pad_right = (inner_w as usize).saturating_sub(used);
    Line::from(vec![
        Span::styled("\u{2502}".to_string(), Style::default().fg(primary)),
        Span::styled(inner, Style::default().fg(primary)),
        Span::raw(" ".repeat(pad_right)),
        Span::styled("\u{2502}".to_string(), Style::default().fg(primary)),
    ])
}

fn access_line(theme: &FamiliarTheme, primary: Color, inner_w: u16) -> Line<'static> {
    let dot = Span::styled("\u{25cf}", Style::default().fg(theme.access_color()));
    let tier = Span::styled(
        format!(" {}", theme.access),
        Style::default().fg(Color::Rgb(203, 213, 225)),
    );
    let used = 2 + 1 + 1 + theme.access.chars().count();
    let pad_right = (inner_w as usize).saturating_sub(used);
    Line::from(vec![
        Span::styled("\u{2502}".to_string(), Style::default().fg(primary)),
        Span::raw("  "),
        dot,
        tier,
        Span::raw(" ".repeat(pad_right)),
        Span::styled("\u{2502}".to_string(), Style::default().fg(primary)),
    ])
}

// ── Glyph dispatch ───────────────────────────────────────────────────────────

fn glyph_lines(theme: &FamiliarTheme, pose: &RustlePose) -> Vec<Line<'static>> {
    match theme.archetype {
        Archetype::SigilCrystal => sigil_crystal(&theme.palette, &theme.emoji),
        Archetype::SigilHex => sigil_hex(&theme.palette, &theme.emoji),
        Archetype::SigilRune => sigil_rune(&theme.palette, &theme.emoji),
        Archetype::SigilSeal => sigil_seal(&theme.palette, &theme.emoji),
        _ => archetype_lines(theme.archetype, &theme.palette, pose).to_vec(),
    }
}

// ── Procedural sigils for user-defined familiars ─────────────────────────────
//
// Each sigil is 11 visible cells wide × 4 rows so it slots in next to the
// hand-crafted built-ins without changing the bordered layout.  The emoji
// (2 cells wide on most terminals) is rendered as its own span; the
// surrounding frame characters are colored in the resolved palette.

fn sigil_crystal(p: &FamiliarPalette, emoji: &str) -> Vec<Line<'static>> {
    let frame = Style::default().fg(p.primary).add_modifier(Modifier::BOLD);
    let accent = Style::default().fg(p.accent);
    vec![
        Line::from(Span::styled(
            "    \u{2581}\u{2580}\u{2581}    ".to_string(),
            frame,
        )),
        emoji_row(p, emoji, "  \u{25e2}", "\u{25e3}  "),
        Line::from(Span::styled(
            "    \u{2580}\u{2581}\u{2580}    ".to_string(),
            frame,
        )),
        Line::from(Span::styled(
            "     \u{2022} \u{2022}    ".to_string(),
            accent,
        )),
    ]
}

fn sigil_hex(p: &FamiliarPalette, emoji: &str) -> Vec<Line<'static>> {
    let frame = Style::default().fg(p.primary).add_modifier(Modifier::BOLD);
    let accent = Style::default().fg(p.accent);
    vec![
        Line::from(Span::styled(
            "   \u{256d}\u{2500}\u{2500}\u{2500}\u{256e}   ".to_string(),
            frame,
        )),
        emoji_row(p, emoji, "   \u{2502}", "\u{2502}    "),
        Line::from(Span::styled(
            "   \u{2570}\u{2500}\u{2500}\u{2500}\u{256f}   ".to_string(),
            frame,
        )),
        Line::from(Span::styled(
            "     \u{2024}\u{2024}\u{2024}    ".to_string(),
            accent,
        )),
    ]
}

fn sigil_rune(p: &FamiliarPalette, emoji: &str) -> Vec<Line<'static>> {
    let frame = Style::default().fg(p.primary).add_modifier(Modifier::BOLD);
    let accent = Style::default().fg(p.accent);
    vec![
        Line::from(Span::styled(
            "    \u{258e}   \u{258e}   ".to_string(),
            frame,
        )),
        emoji_row(p, emoji, "    \u{258e}", "\u{258e}    "),
        Line::from(Span::styled(
            "    \u{2594}\u{2594}\u{2594}\u{2594}\u{2594}   ".to_string(),
            frame,
        )),
        Line::from(Span::styled(
            "     \u{2500} \u{2500}    ".to_string(),
            accent,
        )),
    ]
}

fn sigil_seal(p: &FamiliarPalette, emoji: &str) -> Vec<Line<'static>> {
    let frame = Style::default().fg(p.primary).add_modifier(Modifier::BOLD);
    let accent = Style::default().fg(p.accent);
    vec![
        Line::from(Span::styled(
            "    \u{2726} \u{2726}    ".to_string(),
            accent,
        )),
        emoji_row(p, emoji, "   \u{2727}", "\u{2727}    "),
        Line::from(Span::styled(
            "    \u{2726} \u{2726}    ".to_string(),
            accent,
        )),
        Line::from(Span::styled(
            "    \u{2500}\u{2500}\u{2500}\u{2500}    ".to_string(),
            frame,
        )),
    ]
}

/// Compose a row with a 2-cell emoji centered between two frame slugs and
/// pad to 11 visible cells.
fn emoji_row(p: &FamiliarPalette, emoji: &str, left: &str, right: &str) -> Line<'static> {
    let frame_style = Style::default().fg(p.primary).add_modifier(Modifier::BOLD);
    // Visible width = left_chars + 2 (emoji) + right_chars. Pad to 11.
    let left_w = left.chars().count();
    let right_w = right.chars().count();
    let used = left_w + 2 + right_w;
    let pad_right = 11usize.saturating_sub(used);
    Line::from(vec![
        Span::styled(left.to_string(), frame_style),
        Span::raw(emoji.to_string()),
        Span::styled(right.to_string(), frame_style),
        Span::raw(" ".repeat(pad_right)),
    ])
}

// ── Width helpers ────────────────────────────────────────────────────────────

fn visible_width(line: &Line<'_>) -> usize {
    line.spans
        .iter()
        .map(|s| visible_str_width(&s.content))
        .sum()
}

/// Approximate display width treating most chars as 1 cell and emoji /
/// East-Asian wide chars as 2. The glyphs in this crate use only
/// Unicode block-drawing chars (1 cell) and the optional emoji (~2 cells),
/// so this approximation is sufficient.
fn visible_str_width(s: &str) -> usize {
    s.chars()
        .map(|c| {
            let cp = c as u32;
            if (0x1F300..=0x1FAFF).contains(&cp) {
                2
            } else if (0x2600..=0x27BF).contains(&cp) {
                // Misc symbols / dingbats, usually 1 cell but emoji-style
                // (✨, ★, ✦, etc.) commonly render as 1 cell in modern terminals.
                1
            } else {
                1
            }
        })
        .sum()
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut out = String::new();
        for (i, c) in s.chars().enumerate() {
            if i + 1 >= max {
                out.push('\u{2026}');
                break;
            }
            out.push(c);
        }
        out
    }
}

fn short_tier(access: &str) -> &str {
    match access {
        "full" => "full",
        "read-only" => "read",
        "search-only" => "search",
        _ => access,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::familiar_theme;

    #[test]
    fn size_thresholds() {
        assert!(matches!(pick_size(10), CardSize::Compact));
        assert!(matches!(pick_size(24), CardSize::Standard));
        assert!(matches!(pick_size(40), CardSize::Large));
    }

    #[test]
    fn render_card_compact_is_glyph_only() {
        let t = familiar_theme::resolve("kitty", &[]);
        let lines = render_card(&t, CardSize::Compact, None);
        // Built-in archetypes return 5 rows (4 content + 1 blank); compact passes through.
        assert!(lines.len() >= 4);
    }

    #[test]
    fn render_card_standard_has_border() {
        let t = familiar_theme::resolve("nova", &[]);
        let lines = render_card(&t, CardSize::Standard, None);
        // Top border + 5 glyph rows + access row + bottom border = at least 8.
        assert!(lines.len() >= 7);
    }

    #[test]
    fn render_card_large_includes_rule_row() {
        let t = familiar_theme::resolve("sage", &[]);
        let lines = render_card(&t, CardSize::Large, None);
        // Large has at least one more row than Standard (the rule + role region).
        let standard = render_card(&t, CardSize::Standard, None).len();
        assert!(lines.len() > standard);
    }

    #[test]
    fn unknown_familiar_falls_back_without_panic() {
        let t = familiar_theme::resolve("definitely-not-real", &[]);
        let _ = render_card(&t, CardSize::Large, None);
    }
}
