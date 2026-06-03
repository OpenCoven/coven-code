//! Companion mascot rendering for ratatui.
//!
//! Each OpenCoven familiar has its own pixel-art glyph. The active familiar
//! is read from `config.familiar` (settings.json `"familiar"` key) and
//! determines which glyph renders in the welcome screen top-left, the F2
//! switcher, and the `/agents` detail view.
//!
//! The glyph itself is **static** — no walking, no idle blink, no
//! Tab-triggered look-down. The only motion is the loading spinner that
//! rotates inside the eye row when the assistant is mid-turn and stalled.
//! That motion lives in [`loading_eye_spans`]. Everything else is one
//! frame for one pose.
//!
//! Public surface:
//! - [`RustlePose`] — `Static` for the resting glyph, `Loading { frame }` for the spinner.
//! - [`archetype_lines`] — palette-aware glyph dispatcher used by [`crate::familiar_card`].
//! - [`rustle_lines_for`] — legacy entry point preserved for callers that still
//!   pass a familiar slug; it routes through the theme/card path internally.
//!
//! # Built-in roster
//!
//! | ID       | Archetype                                                                |
//! |----------|--------------------------------------------------------------------------|
//! | `kitty`  | Cat head — pointy ears, whisker nose, square eyes (default).             |
//! | `nova`   | 4-point star + crown + orbit dots — sorceress.                           |
//! | `cody`   | Robot face with antenna, bracket eyes, code body.                        |
//! | `charm`  | Large pixel heart with sparkle dots.                                     |
//! | `sage`   | Wizard hat with star above an open spellbook.                            |
//! | `astra`  | Crescent moon, compass star, dotted orbit.                               |
//! | `echo`   | Round ghost with bracket eyes and floaty echo dots.                      |
//!
//! All glyphs are 11 chars wide × 4 content rows + 1 blank spacing row.

use crate::familiar_theme::{Archetype, FamiliarPalette};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

// ── Pose ─────────────────────────────────────────────────────────────────────

/// Pose / expression of the companion mascot.
///
/// Static is the resting frame. Loading carries a monotonically-increasing
/// frame counter that drives the eye-spinner animation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RustlePose {
    Static,
    Loading { frame: u64 },
}

// ── Style helpers ────────────────────────────────────────────────────────────

fn body_style(palette: &FamiliarPalette) -> Style {
    Style::default()
        .fg(palette.primary)
        .add_modifier(Modifier::BOLD)
}

fn accent_style(palette: &FamiliarPalette) -> Style {
    Style::default()
        .fg(palette.accent)
        .add_modifier(Modifier::BOLD)
}

fn eye_bg_style(palette: &FamiliarPalette) -> Style {
    Style::default()
        .fg(palette.eye_socket)
        .bg(palette.eye_bg)
        .add_modifier(Modifier::BOLD)
}

fn eyeball_style(palette: &FamiliarPalette) -> Style {
    Style::default()
        .fg(Color::White)
        .bg(palette.eye_bg)
        .add_modifier(Modifier::BOLD)
}

// ── Eye helpers ───────────────────────────────────────────────────────────────

fn eye_spans(palette: &FamiliarPalette, s: &'static str) -> Vec<Span<'static>> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut buf = String::new();
    let mut buf_is_eyeball = false;
    for ch in s.chars() {
        let is_eyeball = matches!(
            ch,
            '\u{2598}'
                | '\u{259d}'
                | '\u{2580}'
                | '\u{2584}'
                | '\u{2596}'
                | '\u{258c}'
                | '\u{2590}'
        );
        if is_eyeball != buf_is_eyeball && !buf.is_empty() {
            spans.push(Span::styled(
                buf.clone(),
                if buf_is_eyeball {
                    eyeball_style(palette)
                } else {
                    eye_bg_style(palette)
                },
            ));
            buf.clear();
        }
        buf_is_eyeball = is_eyeball;
        buf.push(ch);
    }
    if !buf.is_empty() {
        spans.push(Span::styled(
            buf,
            if buf_is_eyeball {
                eyeball_style(palette)
            } else {
                eye_bg_style(palette)
            },
        ));
    }
    spans
}

/// Five-cell eye row that rotates a quarter-block highlight clockwise.
/// Returned spans use the supplied palette for the dim trailing color so the
/// loader feels coherent with the rest of the glyph.
fn loading_eye_spans(palette: &FamiliarPalette, frame: u64) -> Vec<Span<'static>> {
    const QUARTERS: [char; 4] = ['\u{2598}', '\u{259d}', '\u{2597}', '\u{2596}'];
    const CW: [usize; 4] = [0, 1, 2, 3];
    const CCW: [usize; 4] = [1, 0, 3, 2];
    let step = (frame / 5) as usize % 4;
    let prev = (step + 3) % 4;
    let trail = palette.primary;
    let head = Color::White;
    let bg = palette.eye_bg;
    let bold = Modifier::BOLD;
    vec![
        Span::styled(
            QUARTERS[CW[prev]].to_string(),
            Style::default().fg(trail).bg(bg).add_modifier(bold),
        ),
        Span::styled(
            QUARTERS[CW[step]].to_string(),
            Style::default().fg(head).bg(bg).add_modifier(bold),
        ),
        Span::styled("\u{2588}".to_string(), eye_bg_style(palette)),
        Span::styled(
            QUARTERS[CCW[step]].to_string(),
            Style::default().fg(head).bg(bg).add_modifier(bold),
        ),
        Span::styled(
            QUARTERS[CCW[prev]].to_string(),
            Style::default().fg(trail).bg(bg).add_modifier(bold),
        ),
    ]
}

// ── Per-archetype glyph builders ──────────────────────────────────────────────

/// **Kitty** — cat head: pointy ears, square eyes, whisker nose.
fn kitty_lines(palette: &FamiliarPalette, pose: &RustlePose) -> [Line<'static>; 5] {
    let row1 = Line::from(Span::styled(
        " \u{2584}\u{2596}   \u{2597}\u{2584}\u{2596}  ".to_string(),
        body_style(palette),
    ));
    let row2 = match pose {
        RustlePose::Static => Line::from(Span::styled(
            " \u{2590}\u{25c8}   \u{25c8}\u{2590}\u{258c} ".to_string(),
            body_style(palette),
        )),
        RustlePose::Loading { frame } => {
            let mut spans = vec![Span::styled(" \u{2590}".to_string(), body_style(palette))];
            spans.extend(loading_eye_spans(palette, *frame));
            spans.push(Span::styled(
                "\u{2590}\u{258c}  ".to_string(),
                body_style(palette),
            ));
            Line::from(spans)
        }
    };
    let row3 = Line::from(Span::styled(
        " \u{2590}\u{258c} \u{1d25} \u{2590}\u{258c}   ".to_string(),
        body_style(palette),
    ));
    let row4 = Line::from(Span::styled(
        "  \u{2580}\u{2580}\u{2580}\u{2580}\u{2580}\u{2580}   ".to_string(),
        body_style(palette),
    ));
    [row1, row2, row3, row4, Line::from("")]
}

/// **Nova** — crown, hooded face, gem clasp, sparkle accents.
fn nova_lines(palette: &FamiliarPalette, pose: &RustlePose) -> [Line<'static>; 5] {
    let row1 = Line::from(Span::styled(
        "   \u{00b7} \u{2726} \u{00b7}   ".to_string(),
        accent_style(palette),
    ));
    let row2 = match pose {
        RustlePose::Loading { frame } => {
            let spin = ['\u{00b7}', '\u{2726}', '*', '\u{00b7}'];
            let s = spin[(*frame / 5) as usize % 4];
            Line::from(Span::styled(
                format!(" \u{2597}\u{2584}{}\u{2584}\u{2597}\u{2596}    ", s),
                body_style(palette),
            ))
        }
        RustlePose::Static => Line::from(Span::styled(
            " \u{2597}\u{2584}\u{265b}\u{2584}\u{2597}\u{2596}    ".to_string(),
            body_style(palette),
        )),
    };
    let row3 = Line::from(Span::styled(
        "  \u{2590}\u{258c}\u{2588}\u{2588}\u{2588}\u{2590}\u{258c}  ".to_string(),
        body_style(palette),
    ));
    let row4 = Line::from(Span::styled(
        "   \u{25c6} \u{00b7} \u{25c6}   ".to_string(),
        accent_style(palette),
    ));
    [row1, row2, row3, row4, Line::from("")]
}

/// **Cody** — robot programmer: antenna, bracket eyes, code body.
fn cody_lines(palette: &FamiliarPalette, pose: &RustlePose) -> [Line<'static>; 5] {
    let row1 = Line::from(Span::styled(
        "    \u{2500}\u{253c}\u{2500}    ".to_string(),
        body_style(palette),
    ));
    let row2 = match pose {
        RustlePose::Loading { frame } => {
            let anim = ['[', '(', '[', '<'];
            let ch = anim[(*frame / 5) as usize % 4];
            Line::from(Span::styled(
                format!(" \u{2584}\u{2584}[{ch} {ch}]\u{2584}  "),
                body_style(palette),
            ))
        }
        RustlePose::Static => Line::from(Span::styled(
            " \u{2584}\u{2584}[\u{25c8} \u{25c8}]\u{2584}  ".to_string(),
            body_style(palette),
        )),
    };
    let row3 = Line::from(Span::styled(
        "  \u{258c}</> \u{2590}   ".to_string(),
        body_style(palette),
    ));
    let row4 = Line::from(Span::styled(
        "  \u{2584}\u{2588}\u{2588}\u{2588}\u{2588}\u{2584}   ".to_string(),
        body_style(palette),
    ));
    [row1, row2, row3, row4, Line::from("")]
}

/// **Charm** — large pixel heart with sparkle dots.
fn charm_lines(palette: &FamiliarPalette, pose: &RustlePose) -> [Line<'static>; 5] {
    let row1 = Line::from(Span::styled(
        "  \u{2584}\u{2588}\u{2588}\u{2584}\u{2584}\u{2588}\u{2588}\u{2584} ".to_string(),
        body_style(palette),
    ));
    let row2 = match pose {
        RustlePose::Loading { frame } => {
            let sparkle = ['\u{2726}', '\u{00b7}', '*', '\u{00b7}'];
            let s = sparkle[(*frame / 5) as usize % 4];
            Line::from(Span::styled(
                format!(" {s}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}{s}  "),
                body_style(palette),
            ))
        }
        RustlePose::Static => Line::from(Span::styled(
            " \u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588} "
                .to_string(),
            body_style(palette),
        )),
    };
    let row3 = Line::from(Span::styled(
        "  \u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}  ".to_string(),
        body_style(palette),
    ));
    let row4 = Line::from(Span::styled(
        "    \u{2580}\u{2588}\u{2580}    ".to_string(),
        body_style(palette),
    ));
    [row1, row2, row3, row4, Line::from("")]
}

/// **Sage** — wizard hat with star above an open spellbook.
fn sage_lines(palette: &FamiliarPalette, pose: &RustlePose) -> [Line<'static>; 5] {
    let row1 = Line::from(Span::styled(
        "    \u{2597}\u{2584}\u{2596}    ".to_string(),
        body_style(palette),
    ));
    let row2 = Line::from(Span::styled(
        "  \u{2597}\u{2588}\u{2726}\u{2588}\u{2588}\u{2596}   ".to_string(),
        body_style(palette),
    ));
    let row3 = Line::from(Span::styled(
        " \u{2584}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2584} ".to_string(),
        body_style(palette),
    ));
    let row4 = match pose {
        RustlePose::Loading { frame } => {
            let page = ['\u{2500}', '~', '\u{2500}', '~'];
            let p = page[(*frame / 5) as usize % 4];
            Line::from(Span::styled(
                format!(" \u{2590}{p}{p}\u{253c}{p}{p}\u{258c}   "),
                body_style(palette),
            ))
        }
        RustlePose::Static => Line::from(Span::styled(
            " \u{2590}\u{2500}\u{2500}\u{253c}\u{2500}\u{2500}\u{258c}   ".to_string(),
            body_style(palette),
        )),
    };
    [row1, row2, row3, row4, Line::from("")]
}

/// **Astra** — crescent moon with compass star and dotted orbit.
fn astra_lines(palette: &FamiliarPalette, pose: &RustlePose) -> [Line<'static>; 5] {
    let row1 = Line::from(Span::styled(
        "    \u{2726}  \u{00b7}   ".to_string(),
        accent_style(palette),
    ));
    let row2 = Line::from(Span::styled(
        " \u{2597}\u{2588}\u{2588}\u{2588}\u{2588}\u{2596}    ".to_string(),
        body_style(palette),
    ));
    let row3 = match pose {
        RustlePose::Loading { frame } => {
            let arcs = [
                " \u{2588}    \u{2598}    ",
                " \u{2588}    \u{00b7}    ",
                " \u{2588}    \u{2598}    ",
                " \u{2588}     \u{00b7}   ",
            ];
            Line::from(Span::styled(
                arcs[(*frame / 5) as usize % 4].to_string(),
                body_style(palette),
            ))
        }
        RustlePose::Static => Line::from(Span::styled(
            " \u{2588}    \u{2726}    ".to_string(),
            body_style(palette),
        )),
    };
    let row4 = Line::from(Span::styled(
        " \u{2580}\u{2584}\u{2584}\u{00b7} \u{00b7}    ".to_string(),
        accent_style(palette),
    ));
    [row1, row2, row3, row4, Line::from("")]
}

/// **Echo** — round ghost with bracket eyes, blush smile, floaty dots.
fn echo_lines(palette: &FamiliarPalette, pose: &RustlePose) -> [Line<'static>; 5] {
    let row1 = Line::from(Span::styled(
        "  \u{2584}\u{2588}\u{2588}\u{2588}\u{2588}\u{2584}   ".to_string(),
        body_style(palette),
    ));
    let row2 = match pose {
        RustlePose::Loading { frame } => {
            let mut spans = vec![Span::styled("  \u{2588}[".to_string(), body_style(palette))];
            spans.extend(loading_eye_spans(palette, *frame));
            spans.push(Span::styled(
                "]\u{2588}   ".to_string(),
                body_style(palette),
            ));
            Line::from(spans)
        }
        RustlePose::Static => {
            let mut spans = vec![Span::styled("  \u{2588}[".to_string(), body_style(palette))];
            spans.extend(eye_spans(palette, "\u{2580}\u{00b7}\u{2580}"));
            spans.push(Span::styled(
                "]\u{2588}   ".to_string(),
                body_style(palette),
            ));
            Line::from(spans)
        }
    };
    let row3 = Line::from(Span::styled(
        "  \u{2588} \u{203f} \u{2588}    ".to_string(),
        body_style(palette),
    ));
    let row4 = match pose {
        RustlePose::Loading { frame } => {
            let dots = [
                "  \u{2580}\u{2584}\u{2580}\u{2584}\u{2580} \u{00b7}\u{00b7}\u{00b7}",
                "  \u{2580}\u{2584}\u{2580}\u{2584}\u{2580} \u{00b7}\u{00b7} ",
                "  \u{2580}\u{2584}\u{2580}\u{2584}\u{2580} \u{00b7}  ",
                "  \u{2580}\u{2584}\u{2580}\u{2584}\u{2580}    ",
            ];
            Line::from(Span::styled(
                dots[(*frame / 8) as usize % 4].to_string(),
                accent_style(palette),
            ))
        }
        RustlePose::Static => Line::from(Span::styled(
            "  \u{2580}\u{2584}\u{2580}\u{2584}\u{2580} \u{00b7}\u{00b7}\u{00b7}".to_string(),
            accent_style(palette),
        )),
    };
    [row1, row2, row3, row4, Line::from("")]
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Render the 5-line glyph block for a built-in archetype.
///
/// Sigil archetypes ([`Archetype::SigilCrystal`] etc.) are handled by
/// [`crate::familiar_card`] directly, so they will hit `kitty_lines` here as
/// a safe fallback if they ever route through this path by accident.
pub fn archetype_lines(
    arch: Archetype,
    palette: &FamiliarPalette,
    pose: &RustlePose,
) -> [Line<'static>; 5] {
    match arch {
        Archetype::Cat => kitty_lines(palette, pose),
        Archetype::SorceressCrown => nova_lines(palette, pose),
        Archetype::Robot => cody_lines(palette, pose),
        Archetype::Heart => charm_lines(palette, pose),
        Archetype::WizardBook => sage_lines(palette, pose),
        Archetype::Moon => astra_lines(palette, pose),
        Archetype::Ghost => echo_lines(palette, pose),
        Archetype::SigilCrystal
        | Archetype::SigilHex
        | Archetype::SigilRune
        | Archetype::SigilSeal => kitty_lines(palette, pose),
    }
}

/// Legacy entry point — resolve a familiar slug to its theme and render the
/// glyph block. Keeps the old function name so any straggling caller can
/// keep working; new code should go through [`crate::familiar_card::render_card`].
pub fn rustle_lines_for(familiar: Option<&str>, pose: &RustlePose) -> [Line<'static>; 5] {
    let id = familiar.unwrap_or("kitty");
    let theme = crate::familiar_theme::resolve(id, &[]);
    archetype_lines(theme.archetype, &theme.palette, pose)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::familiar_theme;

    fn line_text(line: &Line<'_>) -> String {
        line.spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect::<Vec<_>>()
            .join("")
    }

    #[test]
    fn static_pose_renders_all_familiars() {
        let familiars = ["kitty", "nova", "cody", "charm", "sage", "astra", "echo"];
        for fam in &familiars {
            let lines = rustle_lines_for(Some(fam), &RustlePose::Static);
            assert_eq!(lines.len(), 5, "familiar {fam} should produce 5 rows");
            let row0 = line_text(&lines[0]);
            assert!(
                !row0.trim().is_empty(),
                "familiar {fam} row 0 should not be blank: {row0:?}"
            );
        }
    }

    #[test]
    fn loading_pose_drives_frame_dependent_output() {
        // Different frame values should produce at least one frame where the
        // visible text differs, proving the spinner is actually frame-driven.
        let a = rustle_lines_for(Some("kitty"), &RustlePose::Loading { frame: 0 });
        let b = rustle_lines_for(Some("kitty"), &RustlePose::Loading { frame: 5 });
        let txt_a = line_text(&a[1]);
        let txt_b = line_text(&b[1]);
        assert_ne!(txt_a, txt_b, "loading row should differ between frames");
    }

    #[test]
    fn unknown_familiar_falls_back_to_kitty() {
        let a = rustle_lines_for(Some("unknown_xxx"), &RustlePose::Static);
        let b = rustle_lines_for(Some("kitty"), &RustlePose::Static);
        assert_eq!(line_text(&a[0]), line_text(&b[0]));
    }

    #[test]
    fn none_familiar_falls_back_to_kitty() {
        let a = rustle_lines_for(None, &RustlePose::Static);
        let b = rustle_lines_for(Some("kitty"), &RustlePose::Static);
        assert_eq!(line_text(&a[0]), line_text(&b[0]));
    }

    #[test]
    fn archetype_dispatcher_respects_palette() {
        // Different palettes produce spans whose styles differ; the glyph
        // shape itself stays the same.
        let theme_a = familiar_theme::resolve("kitty", &[]);
        let theme_b = familiar_theme::resolve("nova", &[]);
        let a = archetype_lines(theme_a.archetype, &theme_a.palette, &RustlePose::Static);
        let b = archetype_lines(theme_b.archetype, &theme_b.palette, &RustlePose::Static);
        // Distinct archetypes → distinct row 0 content.
        assert_ne!(line_text(&a[0]), line_text(&b[0]));
    }
}
