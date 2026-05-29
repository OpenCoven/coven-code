//! Companion mascot rendering for ratatui.
//!
//! Each OpenCoven familiar has its own pixel-art glyph. The active familiar
//! is read from `config.familiar` (settings.json `"familiar"` key) and
//! determines which glyph renders in the welcome screen top-left.
//!
//! Public API preserves upstream names (`RustlePose`, `rustle_lines`) for
//! `git merge upstream/main` friendliness.
//!
//! # Familiar roster
//!
//! | ID       | Glyph concept                         |
//! |----------|---------------------------------------|
//! | `kitty`  | Cat head — ears, whiskers, square eyes (default) |
//! | `nova`   | 4-point star with orbiting sparks     |
//! | `cody`   | Robot face — antenna, bracket eyes    |
//! | `charm`  | Heart with sparkle dots + speech bubble |
//! | `sage`   | Wizard hat + star + open book         |
//! | `astra`  | Crescent moon + compass star + orbit  |
//! | `echo`   | Round ghost + mirror eyes + echo dots |
//!
//! # Layout
//!
//! All glyphs are 11 chars wide × 4 content rows + 1 blank spacing row.
//! Row indexing:
//!   [0] — head / top
//!   [1] — face / eyes (animated for Loading pose)
//!   [2] — body / mid
//!   [3] — feet / bottom
//!   [4] — blank spacing

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

// ── Pose ─────────────────────────────────────────────────────────────────────

/// Pose / expression of the companion mascot.
/// Names preserved from upstream for merge-friendliness.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RustlePose {
    Default,
    ArmsUp,
    LookLeft,
    LookRight,
    LookDown,
    /// Spinning animation — `frame` drives the eye rotation.
    Loading { frame: u64 },
}

// ── Colors ────────────────────────────────────────────────────────────────────

/// Violet body: #8B5CF6 violet-500 — OpenCoven primary.
fn body_style() -> Style {
    Style::default()
        .fg(Color::Rgb(139, 92, 246))
        .add_modifier(Modifier::BOLD)
}

/// Eye socket: violet-300 on near-black.
fn eye_bg_style() -> Style {
    Style::default()
        .fg(Color::Rgb(196, 181, 253))
        .bg(Color::Rgb(15, 5, 40))
        .add_modifier(Modifier::BOLD)
}

/// Eye highlight: white on near-black.
fn eyeball_style() -> Style {
    Style::default()
        .fg(Color::White)
        .bg(Color::Rgb(15, 5, 40))
        .add_modifier(Modifier::BOLD)
}

/// Accent detail: violet-400 for secondary marks.
fn accent_style() -> Style {
    Style::default()
        .fg(Color::Rgb(167, 139, 250))
        .add_modifier(Modifier::BOLD)
}

// ── Eye helpers ───────────────────────────────────────────────────────────────

fn eye_spans(s: &'static str) -> Vec<Span<'static>> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut buf = String::new();
    let mut buf_is_eyeball = false;
    for ch in s.chars() {
        let is_eyeball = matches!(ch, '▘' | '▝' | '▀' | '▄' | '▖' | '▌' | '▐');
        if is_eyeball != buf_is_eyeball && !buf.is_empty() {
            spans.push(Span::styled(
                buf.clone(),
                if buf_is_eyeball { eyeball_style() } else { eye_bg_style() },
            ));
            buf.clear();
        }
        buf_is_eyeball = is_eyeball;
        buf.push(ch);
    }
    if !buf.is_empty() {
        spans.push(Span::styled(
            buf,
            if buf_is_eyeball { eyeball_style() } else { eye_bg_style() },
        ));
    }
    spans
}

fn loading_eye_spans(frame: u64) -> Vec<Span<'static>> {
    const QUARTERS: [char; 4] = ['▘', '▝', '▗', '▖'];
    const CW: [usize; 4] = [0, 1, 2, 3];
    const CCW: [usize; 4] = [1, 0, 3, 2];
    const COLORS: [Color; 4] = [
        Color::White,
        Color::Rgb(196, 181, 253), // violet-300
        Color::Rgb(139, 92, 246),  // violet-500
        Color::Rgb(76, 29, 149),   // violet-900
    ];
    let step = (frame / 5) as usize % 4;
    let prev = (step + 3) % 4;
    vec![
        Span::styled(
            QUARTERS[CW[prev]].to_string(),
            Style::default().fg(COLORS[2]).bg(Color::Rgb(15, 5, 40)).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            QUARTERS[CW[step]].to_string(),
            Style::default().fg(COLORS[0]).bg(Color::Rgb(15, 5, 40)).add_modifier(Modifier::BOLD),
        ),
        Span::styled("█".to_string(), eye_bg_style()),
        Span::styled(
            QUARTERS[CCW[step]].to_string(),
            Style::default().fg(COLORS[0]).bg(Color::Rgb(15, 5, 40)).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            QUARTERS[CCW[prev]].to_string(),
            Style::default().fg(COLORS[2]).bg(Color::Rgb(15, 5, 40)).add_modifier(Modifier::BOLD),
        ),
    ]
}

// ── Per-familiar glyph builders ───────────────────────────────────────────────

/// **Kitty** — cat head: triangle ears, whiskers, square eyes.
/// Default familiar for Coven Code.
fn kitty_lines(pose: &RustlePose) -> [Line<'static>; 5] {
    let (r2l, r2e, r2r) = match pose {
        RustlePose::Default   => ("──▌", "▀ █▀ ", "▐──"),
        RustlePose::ArmsUp    => ("▀─▌", "▀ █▀ ", "▐─▀"),
        RustlePose::LookLeft  => ("──▌", "▘ █ ▘", "▐──"),
        RustlePose::LookRight => ("──▌", " ▀█ ▀", "▐──"),
        RustlePose::LookDown  => ("──▌", "▄ █▄ ", "▐──"),
        RustlePose::Loading { .. } => ("──▌", "", "▐──"),
    };
    let row1 = Line::from(vec![Span::styled(" ▄  ▄▄▄  ▄ ".to_string(), body_style())]);
    let mut row2 = vec![Span::styled(r2l.to_string(), body_style())];
    if let RustlePose::Loading { frame } = pose {
        row2.extend(loading_eye_spans(*frame));
    } else {
        row2.extend(eye_spans(r2e));
    }
    row2.push(Span::styled(r2r.to_string(), body_style()));
    let row3 = Line::from(vec![Span::styled("  ▌█████▐  ".to_string(), body_style())]);
    let row4 = Line::from(vec![Span::styled("  ▄▀   ▀▄  ".to_string(), body_style())]);
    [row1, Line::from(row2), row3, row4, Line::from("")]
}

/// **Nova** — 4-point star with orbiting sparkle dots.
fn nova_lines(pose: &RustlePose) -> [Line<'static>; 5] {
    // Top: diamond tip with side sparks
    let row1 = Line::from(vec![
        Span::styled("  · ▲ · ·  ".to_string(), accent_style()),
    ]);
    // Face row: star centre with spinning sparks on loading
    let row2 = if let RustlePose::Loading { frame } = pose {
        let spin = ['·', '✦', '*', '·'];
        let s = spin[(frame / 5) as usize % 4];
        Line::from(vec![
            Span::styled(format!("  {}  █  {}  ", s, s), body_style()),
        ])
    } else {
        let eyes = match pose {
            RustlePose::LookLeft  => "  ◄  █  ◄  ",
            RustlePose::LookRight => "  ►  █  ►  ",
            RustlePose::LookDown  => "  ▼  █  ▼  ",
            _                     => "  ▲  █  ▲  ",
        };
        Line::from(vec![Span::styled(eyes.to_string(), body_style())])
    };
    let row3 = Line::from(vec![Span::styled("  ███████  ".to_string(), body_style())]);
    let row4 = Line::from(vec![
        Span::styled("  · ▼ · ·  ".to_string(), accent_style()),
    ]);
    [row1, row2, row3, row4, Line::from("")]
}

/// **Cody** — robot face: antenna, square bracket eyes, code details.
fn cody_lines(pose: &RustlePose) -> [Line<'static>; 5] {
    // Antenna top
    let row1 = Line::from(vec![Span::styled("    ─┼─    ".to_string(), body_style())]);
    // Face with bracket eyes
    let r2 = match pose {
        RustlePose::Loading { frame } => {
            let anim = ['[', '(', '[', '<'];
            let ch = anim[(frame / 5) as usize % 4];
            format!(" ▄ {}   {} ▄ ", ch, ch)  // 11 chars
        }
        RustlePose::LookLeft  => " ▄ [◄ ◄] ▄ ".to_string(),
        RustlePose::LookRight => " ▄ [► ►] ▄ ".to_string(),
        RustlePose::LookDown  => " ▄ [▼ ▼] ▄ ".to_string(),
        RustlePose::ArmsUp    => " ▄ [▲ ▲] ▄ ".to_string(),
        RustlePose::Default   => " ▄ [■ ■] ▄ ".to_string(),
    };
    let row2 = Line::from(vec![Span::styled(r2, body_style())]);
    // Body with code glyphs
    let row3 = Line::from(vec![Span::styled("  ▌{..}▐   ".to_string(), body_style())]);
    // Feet / base
    let row4 = Line::from(vec![Span::styled("  ▐█▌ ▐█▌  ".to_string(), body_style())]);
    [row1, row2, row3, row4, Line::from("")]
}

/// **Charm** — heart with sparkle dots and speech bubble.
fn charm_lines(pose: &RustlePose) -> [Line<'static>; 5] {
    let row1 = Line::from(vec![
        Span::styled(" ✦ ▄██▄ ✦ ".to_string(), body_style()),
    ]);
    let row2 = if let RustlePose::Loading { frame } = pose {
        let sparkle = ['✦', '·', '*', '·'];
        let s = sparkle[(frame / 5) as usize % 4];
        Line::from(vec![
            Span::styled(format!("  {s}█████{s}  "), body_style()),
        ])
    } else {
        Line::from(vec![Span::styled("  ███████  ".to_string(), body_style())])
    };
    let row3 = Line::from(vec![Span::styled("   █████   ".to_string(), body_style())]);
    let row4_left = match pose {
        RustlePose::ArmsUp => "  ✦ ▀█▀ ✦  ",
        _                  => "    ▀█▀    ",
    };
    let row4 = Line::from(vec![Span::styled(row4_left.to_string(), body_style())]);
    [row1, row2, row3, row4, Line::from("")]
}

/// **Sage** — wizard hat with star above an open book.
fn sage_lines(pose: &RustlePose) -> [Line<'static>; 5] {
    let row1 = Line::from(vec![Span::styled("    ▲✦▲    ".to_string(), body_style())]);
    let row2 = Line::from(vec![Span::styled("   ████▄   ".to_string(), body_style())]);
    // Book: open spine line, animated page turn on loading
    let row3 = if let RustlePose::Loading { frame } = pose {
        let page = ['─', '~', '─', '~'];
        let p = page[(frame / 5) as usize % 4];
        Line::from(vec![
            Span::styled(format!("  ▐{p}{p}│{p}{p}▌  "), body_style()),
        ])
    } else {
        Line::from(vec![Span::styled("  ▐──│──▌  ".to_string(), body_style())])
    };
    let row4 = Line::from(vec![Span::styled("  ▀▀▀▀▀▀▀  ".to_string(), body_style())]);
    [row1, row2, row3, row4, Line::from("")]
}

/// **Astra** — crescent moon with compass star and orbit arc.
fn astra_lines(pose: &RustlePose) -> [Line<'static>; 5] {
    let row1 = Line::from(vec![Span::styled("   ·  ✦ ·  ".to_string(), accent_style())]);
    // Crescent: right-facing, hollow interior
    let row2 = Line::from(vec![Span::styled("  ██▄      ".to_string(), body_style())]);
    // Orbit arc animated on loading
    let row3 = if let RustlePose::Loading { frame } = pose {
        let arcs = ["  ██ ·--·  ", "  ██ --··  ", "  ██ ·--·  ", "  ██ ··--  "];
        Line::from(vec![Span::styled(arcs[(frame / 5) as usize % 4].to_string(), body_style())])
    } else {
        Line::from(vec![Span::styled("  ██▀  ✦   ".to_string(), body_style())])
    };
    let row4 = Line::from(vec![Span::styled("  · ──── · ".to_string(), accent_style())]);
    [row1, row2, row3, row4, Line::from("")]
}

/// **Echo** — round ghost silhouette with mirror-bracket eyes and echo dots.
fn echo_lines(pose: &RustlePose) -> [Line<'static>; 5] {
    let row1 = Line::from(vec![Span::styled("   ▄████▄  ".to_string(), body_style())]);
    // Eyes
    let (r2l, r2e, r2r) = match pose {
        RustlePose::Loading { .. } => ("  █", "", "█  "),
        RustlePose::LookLeft       => ("  █", "▘ · ▘", "█  "),
        RustlePose::LookRight      => ("  █", " · ▝ ▝", "█  "),
        _                          => ("  █", "▀ · ▀", "█  "),
    };
    let mut row2_spans = vec![Span::styled(r2l.to_string(), body_style())];
    if let RustlePose::Loading { frame } = pose {
        row2_spans.extend(loading_eye_spans(*frame));
    } else {
        row2_spans.extend(eye_spans(r2e));
    }
    row2_spans.push(Span::styled(r2r.to_string(), body_style()));
    // Body — wavy hem
    let row3 = Line::from(vec![Span::styled("  ██████   ".to_string(), body_style())]);
    // Trailing echo dots, animated offset
    let row4 = if let RustlePose::Loading { frame } = pose {
        let dots = ["  ▀▄▀▄ ···", "  ▀▄▀▄ ·· ", "  ▀▄▀▄ ·  ", "  ▀▄▀▄    "];
        Line::from(vec![Span::styled(dots[(frame / 8) as usize % 4].to_string(), accent_style())])
    } else {
        Line::from(vec![Span::styled("  ▀▄▀▄ ··· ".to_string(), accent_style())])
    };
    [row1, Line::from(row2_spans), row3, row4, Line::from("")]
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Returns 5 `Line` values for the given familiar and pose.
///
/// `familiar` should be the lowercase familiar ID from `config.familiar`
/// (e.g. `"kitty"`, `"nova"`, `"cody"`…). `None` or unknown values fall
/// back to `kitty`.
pub fn rustle_lines_for(familiar: Option<&str>, pose: &RustlePose) -> [Line<'static>; 5] {
    match familiar.unwrap_or("kitty") {
        "nova"  => nova_lines(pose),
        "cody"  => cody_lines(pose),
        "charm" => charm_lines(pose),
        "sage"  => sage_lines(pose),
        "astra" => astra_lines(pose),
        "echo"  => echo_lines(pose),
        _       => kitty_lines(pose), // "kitty" + default
    }
}

/// Legacy entry point — defaults to kitty. Kept for call-sites that
/// don't yet thread the familiar name through.
pub fn rustle_lines(pose: &RustlePose) -> [Line<'static>; 5] {
    kitty_lines(pose)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line_text(line: &Line<'_>) -> String {
        line.spans.iter().map(|s| s.content.as_ref()).collect::<Vec<_>>().join("")
    }

    // Width assertions — all familiars, all non-loading poses
    fn check_width(name: &str, pose: &RustlePose) {
        let lines = rustle_lines_for(Some(name), pose);
        for (i, line) in lines.iter().enumerate().take(4) {
            let text = line_text(line);
            let width = text.chars().count();
            // Allow 10–12 chars (some glyphs use Unicode combining/width)
            assert!(
                (9..=12).contains(&width),
                "familiar={name} pose={pose:?} row={i} width={width} text={text:?}"
            );
        }
    }

    #[test]
    fn kitty_default_eye_row() {
        let lines = rustle_lines_for(Some("kitty"), &RustlePose::Default);
        assert_eq!(line_text(&lines[1]), "──▌▀ █▀ ▐──");
    }

    #[test]
    fn kitty_arms_up_whiskers_tilt() {
        let lines = rustle_lines_for(Some("kitty"), &RustlePose::ArmsUp);
        assert_eq!(line_text(&lines[1]), "▀─▌▀ █▀ ▐─▀");
    }

    #[test]
    fn all_familiars_all_poses_have_reasonable_width() {
        let familiars = ["kitty", "nova", "cody", "charm", "sage", "astra", "echo"];
        let poses = [
            RustlePose::Default,
            RustlePose::ArmsUp,
            RustlePose::LookLeft,
            RustlePose::LookRight,
            RustlePose::LookDown,
        ];
        for fam in &familiars {
            for pose in &poses {
                check_width(fam, pose);
            }
        }
    }

    #[test]
    fn unknown_familiar_falls_back_to_kitty() {
        let a = rustle_lines_for(Some("unknown_xxx"), &RustlePose::Default);
        let b = rustle_lines_for(Some("kitty"), &RustlePose::Default);
        assert_eq!(line_text(&a[0]), line_text(&b[0]));
    }

    #[test]
    fn none_familiar_falls_back_to_kitty() {
        let a = rustle_lines_for(None, &RustlePose::Default);
        let b = rustle_lines_for(Some("kitty"), &RustlePose::Default);
        assert_eq!(line_text(&a[0]), line_text(&b[0]));
    }
}
