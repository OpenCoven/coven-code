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
    pixel_avatar(avatar_grid(theme.archetype), &theme.palette, pose)
}

/// Public entry for surfaces that want the bare 8-bit avatar rows without the
/// bordered card chrome. The welcome panel centers these directly.
pub fn render_avatar(theme: &FamiliarTheme, pose: &CompanionPose) -> Vec<Line<'static>> {
    glyph_lines(theme, pose)
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

// ── 8-bit pixel-art avatars ──────────────────────────────────────────────────
//
// Each avatar is an 11-wide × 8-tall pixel grid drawn with Unicode half-block
// cells (`▀`/`▄`) so eight pixel rows pack into the four glyph rows the card
// layout reserves. There is a small, curated, fully-functional set — four
// distinct creatures — rather than an open-ended procedural space, and every
// familiar's palette recolors whichever avatar its archetype maps to. Pixels
// are addressed by a single ASCII key per cell:
//
//   ' ' transparent   'P' body (primary)   'A' accent (pulsed)
//   'E' eye            'S' eye highlight    'O' outline (dimmed body)

/// Map an archetype to its 11×8 pixel grid. Reuses the four stable archetype
/// slots so [`crate::familiar_theme`] keeps hashing ids across the same four
/// buckets — only the art behind each slot changed.
fn avatar_grid(archetype: Archetype) -> &'static [&'static str; 8] {
    match archetype {
        Archetype::SigilCrystal => &AVATAR_CRITTER,
        Archetype::SigilHex => &AVATAR_CAT,
        Archetype::SigilRune => &AVATAR_OWL,
        Archetype::SigilSeal => &AVATAR_GOLEM,
    }
}

// A round-bodied critter with a little run of legs.
static AVATAR_CRITTER: [&str; 8] = [
    "   PPPPP   ",
    "  PPAAAPP  ",
    " PPPPPPPPP ",
    " PEPPPPPEP ",
    " PPPPPPPPP ",
    " OPPPPPPPO ",
    "  PP P PP  ",
    "  P  P  P  ",
];

// Pointed ears, wide face, accent cheeks.
static AVATAR_CAT: [&str; 8] = [
    " PP     PP ",
    " PPP   PPP ",
    " PPPPPPPPP ",
    " PEPPPPPEP ",
    " PPPAPPAPP ",
    " PPPPPPPPP ",
    "  PPPPPPP  ",
    "   P   P   ",
];

// Owl with ear tufts, big socketed eyes, a centered beak.
static AVATAR_OWL: [&str; 8] = [
    "  PP   PP  ",
    " PPPPPPPPP ",
    " PSEPPPESP ",
    " PSEPPPESP ",
    " PPPAPPP P ",
    " PPPPPPPPP ",
    "  PPPPPPP  ",
    "  A     A  ",
];

// Blocky golem with rectangular eyes and accent rivets.
static AVATAR_GOLEM: [&str; 8] = [
    "  PPPPPPP  ",
    " PPPPPPPPP ",
    " PEEPPPEEP ",
    " PEEPPPEEP ",
    " PPPPPPPPP ",
    " PAPPPPPAP ",
    " PPPPPPPPP ",
    "  P P P P  ",
];

/// What the eyes are doing this frame, derived from the companion pose.
#[derive(Clone, Copy)]
enum Eyes {
    Open,
    Closed,
    Alert,
}

fn eye_state(pose: &CompanionPose) -> Eyes {
    match pose {
        // A short blink near the end of each idle cycle.
        CompanionPose::Idle { frame } if frame % 150 >= 145 => Eyes::Closed,
        CompanionPose::Loading { .. } => Eyes::Alert,
        _ => Eyes::Open,
    }
}

/// Dim an RGB color toward black by `factor` (0.0 = black, 1.0 = unchanged).
/// Non-RGB colors pass through unchanged.
fn dim(c: Color, factor: f32) -> Color {
    match c {
        Color::Rgb(r, g, b) => Color::Rgb(
            (r as f32 * factor) as u8,
            (g as f32 * factor) as u8,
            (b as f32 * factor) as u8,
        ),
        other => other,
    }
}

/// Resolve one pixel key to its color, or `None` for a transparent cell.
fn pixel_color(key: u8, p: &FamiliarPalette, pose: &CompanionPose) -> Option<Color> {
    let eyes = eye_state(pose);
    match key {
        b' ' => None,
        b'P' => Some(p.primary),
        b'A' => Some(pulse_accent(p, pose)),
        b'O' => Some(dim(p.primary, 0.45)),
        b'E' => Some(match eyes {
            Eyes::Open => p.eye_bg,
            Eyes::Closed => p.primary, // lid down
            Eyes::Alert => p.accent,
        }),
        b'S' => Some(match eyes {
            Eyes::Closed => p.primary,
            _ => p.eye_socket,
        }),
        _ => Some(p.primary),
    }
}

/// Render an 11×8 pixel grid into four half-block glyph rows. Each output cell
/// stacks two vertical pixels: the upper pixel is the foreground of `▀`, the
/// lower pixel its background (or `▄` / blank when one side is transparent), so
/// the avatar stays exactly 11 cells wide — matching the card layout math.
fn pixel_avatar(
    grid: &[&str; 8],
    palette: &FamiliarPalette,
    pose: &CompanionPose,
) -> Vec<Line<'static>> {
    let mut rows = Vec::with_capacity(4);
    for r in 0..4 {
        let top = grid[r * 2].as_bytes();
        let bot = grid[r * 2 + 1].as_bytes();
        let mut spans = Vec::with_capacity(11);
        for c in 0..11 {
            let t = pixel_color(top.get(c).copied().unwrap_or(b' '), palette, pose);
            let b = pixel_color(bot.get(c).copied().unwrap_or(b' '), palette, pose);
            let span = match (t, b) {
                (None, None) => Span::raw(" "),
                (Some(t), None) => Span::styled("\u{2580}".to_string(), Style::default().fg(t)),
                (None, Some(b)) => Span::styled("\u{2584}".to_string(), Style::default().fg(b)),
                (Some(t), Some(b)) => {
                    Span::styled("\u{2580}".to_string(), Style::default().fg(t).bg(b))
                }
            };
            spans.push(span);
        }
        rows.push(Line::from(spans));
    }
    rows
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

    #[test]
    fn avatar_is_four_rows_eleven_cells_wide() {
        let t = familiar_theme::resolve("avatar-test", &[]);
        let rows = render_avatar(&t, &CompanionPose::Static);
        assert_eq!(rows.len(), 4, "avatar must pack 8 pixel rows into 4 glyphs");
        for row in &rows {
            assert_eq!(
                visible_width(row),
                11,
                "every avatar row must stay 11 cells wide for the card layout"
            );
        }
    }

    #[test]
    fn every_archetype_grid_is_well_formed() {
        for grid in [&AVATAR_CRITTER, &AVATAR_CAT, &AVATAR_OWL, &AVATAR_GOLEM] {
            assert_eq!(grid.len(), 8);
            for row in grid.iter() {
                assert_eq!(row.len(), 11, "grid row {row:?} must be 11 cells");
            }
        }
    }

    #[test]
    fn blink_closes_eyes_without_changing_width() {
        let t = familiar_theme::resolve("blinker", &[]);
        let open = render_avatar(&t, &CompanionPose::Idle { frame: 0 });
        let closed = render_avatar(&t, &CompanionPose::Idle { frame: 149 });
        // A blink must not resize the avatar.
        for (a, b) in open.iter().zip(closed.iter()) {
            assert_eq!(visible_width(a), visible_width(b));
        }
    }
}
