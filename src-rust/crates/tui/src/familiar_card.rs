//! Static themed familiar card composer.
//!
//! Given a [`crate::familiar_theme::FamiliarTheme`] and a [`CardSize`], produces
//! the lines that render the active familiar in the welcome panel, the F2
//! switcher, and the `/agents` detail view. The welcome panel passes the live
//! [`CompanionPose`] so the glyph blinks and sways while idle and spins its
//! eye row while the assistant is in the `Loading` state; other surfaces pass
//! `CompanionPose::Static` and stay still.
//!
//! Every archetype ([`Archetype::SigilCrystal`] etc.) draws a colored frame
//! around the familiar's emoji, so any entry from `~/.coven/familiars.toml`
//! gets first-class visual identity and nothing inherits a built-in persona.

use crate::familiar_theme::{Archetype, FamiliarPalette, FamiliarTheme};
use crate::mascot::CompanionPose;
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

/// Render the full card with the given pose: `Static` for surfaces that
/// never animate, `Idle { frame }` for the blink/sway loop, or
/// `Loading { frame }` to spin the eyes.
pub fn render_card(
    theme: &FamiliarTheme,
    size: CardSize,
    pose: &CompanionPose,
) -> Vec<Line<'static>> {
    let glyph = glyph_lines(theme, pose);

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

fn glyph_lines(theme: &FamiliarTheme, pose: &CompanionPose) -> Vec<Line<'static>> {
    match theme.archetype {
        Archetype::SigilCrystal => sigil_crystal(&theme.palette, &theme.emoji, pose),
        Archetype::SigilHex => sigil_hex(&theme.palette, &theme.emoji, pose),
        Archetype::SigilRune => sigil_rune(&theme.palette, &theme.emoji, pose),
        Archetype::SigilSeal => sigil_seal(&theme.palette, &theme.emoji, pose),
    }
}

/// Accent color for a sigil's decorative row, pulsed by the companion pose.
///
/// The glyph text never changes width — only the accent row's color shifts
/// between the bright accent and the dimmer primary — so an `Idle` card
/// breathes softly and a stalled `Loading` card pulses faster, while a
/// `Static` card stays bright. This is the only animation; it is identical for
/// every familiar and tied to no named persona.
fn pulse_accent(p: &FamiliarPalette, pose: &CompanionPose) -> Color {
    match pose {
        CompanionPose::Idle { frame } if matches!(frame % 120, 90..=95) => p.primary,
        CompanionPose::Loading { frame } if (frame / 5) % 2 == 1 => p.primary,
        _ => p.accent,
    }
}

// ── Procedural sigils ────────────────────────────────────────────────────────
//
// Each sigil is 11 visible cells wide × 4 rows so it slots into the bordered
// layout consistently.  The emoji (2 cells wide on most terminals) is rendered
// as its own span; the surrounding frame characters are colored in the
// resolved palette.

fn sigil_crystal(p: &FamiliarPalette, emoji: &str, pose: &CompanionPose) -> Vec<Line<'static>> {
    let frame = Style::default().fg(p.primary).add_modifier(Modifier::BOLD);
    let accent = Style::default().fg(pulse_accent(p, pose));
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

fn sigil_hex(p: &FamiliarPalette, emoji: &str, pose: &CompanionPose) -> Vec<Line<'static>> {
    let frame = Style::default().fg(p.primary).add_modifier(Modifier::BOLD);
    let accent = Style::default().fg(pulse_accent(p, pose));
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

fn sigil_rune(p: &FamiliarPalette, emoji: &str, pose: &CompanionPose) -> Vec<Line<'static>> {
    let frame = Style::default().fg(p.primary).add_modifier(Modifier::BOLD);
    let accent = Style::default().fg(pulse_accent(p, pose));
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

fn sigil_seal(p: &FamiliarPalette, emoji: &str, pose: &CompanionPose) -> Vec<Line<'static>> {
    let frame = Style::default().fg(p.primary).add_modifier(Modifier::BOLD);
    let accent = Style::default().fg(pulse_accent(p, pose));
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
        let t = familiar_theme::resolve("alpha", &[]);
        let lines = render_card(&t, CardSize::Compact, &CompanionPose::Static);
        // Sigil archetypes return 4 content rows; compact passes through.
        assert!(lines.len() >= 4);
    }

    #[test]
    fn render_card_standard_has_border() {
        let t = familiar_theme::resolve("beta", &[]);
        let lines = render_card(&t, CardSize::Standard, &CompanionPose::Static);
        // Top border + glyph rows + access row + bottom border = at least 7.
        assert!(lines.len() >= 7);
    }

    #[test]
    fn render_card_large_includes_rule_row() {
        let t = familiar_theme::resolve("gamma", &[]);
        let lines = render_card(&t, CardSize::Large, &CompanionPose::Static);
        // Large has at least one more row than Standard (the rule + role region).
        let standard = render_card(&t, CardSize::Standard, &CompanionPose::Static).len();
        assert!(lines.len() > standard);
    }

    #[test]
    fn unknown_familiar_falls_back_without_panic() {
        let t = familiar_theme::resolve("definitely-not-real", &[]);
        let _ = render_card(&t, CardSize::Large, &CompanionPose::Static);
    }
}
