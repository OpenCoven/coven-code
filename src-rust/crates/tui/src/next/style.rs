//! Semantic presentation roles for the Next renderer.
//!
//! Role values come from the canonical Brand web profile 1.0.0
//! (OpenCoven/brand `web/profile.json` at
//! `798e2f5f2f1a69f5d156ccc0a9aafc4f7da55fc8`), translated to terminal
//! cells. Widgets ask for a [`Role`]; [`Appearance::style`] resolves it for
//! the terminal's color capability. Classic keeps its own palette in
//! `theme_colors.rs` and is not recolored by this module.
//!
//! Resolution never paints the canvas: the terminal's own background is
//! preserved. Color is never the only cue: roles that carry meaning also set
//! a text attribute, and status primitives add a text/shape label.

use ratatui::style::{Color, Modifier, Style};

/// Presentation meaning of a span.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Role {
    Text,
    TextMuted,
    TextSubtle,
    /// Coven identity accent. Restrained: marks, not body text.
    Identity,
    /// A user-invoked operation.
    Action,
    Focus,
    /// A live/connected embodiment.
    Presence,
    Selection,
    Link,
    Border,
    /// Evidence-backed success.
    Verified,
    /// Waiting, reconciling or stale.
    Pending,
    /// Rejected, failed or unavailable.
    Blocked,
    /// Neutral receipt, provenance or unknown state.
    Evidence,
    DiffAdded,
    DiffRemoved,
    /// An action with side effects that needs deliberate confirmation.
    Sensitive,
}

pub const ALL_ROLES: [Role; 17] = [
    Role::Text,
    Role::TextMuted,
    Role::TextSubtle,
    Role::Identity,
    Role::Action,
    Role::Focus,
    Role::Presence,
    Role::Selection,
    Role::Link,
    Role::Border,
    Role::Verified,
    Role::Pending,
    Role::Blocked,
    Role::Evidence,
    Role::DiffAdded,
    Role::DiffRemoved,
    Role::Sensitive,
];

/// Terminal color capability.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorMode {
    TrueColor,
    /// xterm 256-color palette (approximated from Brand values).
    Ansi256,
    /// The terminal's own 16 named colors; actual values are user-defined.
    Ansi16,
    /// No color at all (`NO_COLOR`, `TERM=dumb`); attributes only.
    NoColor,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scheme {
    Dark,
    Light,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Glyphs {
    Unicode,
    Ascii,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Motion {
    Full,
    Reduced,
}

/// Resolved presentation capabilities for one renderer instance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Appearance {
    pub color: ColorMode,
    pub scheme: Scheme,
    pub glyphs: Glyphs,
    pub motion: Motion,
}

impl Default for Appearance {
    fn default() -> Self {
        Self {
            color: ColorMode::TrueColor,
            scheme: Scheme::Dark,
            glyphs: Glyphs::Unicode,
            motion: Motion::Full,
        }
    }
}

/// Foreground (and optional background) for a role in one scheme, as RGB.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RolePaint {
    pub fg: (u8, u8, u8),
    pub bg: Option<(u8, u8, u8)>,
}

const fn hex(value: u32) -> (u8, u8, u8) {
    (
        ((value >> 16) & 0xFF) as u8,
        ((value >> 8) & 0xFF) as u8,
        (value & 0xFF) as u8,
    )
}

/// Brand canvas (`oc-bg`) for contrast checks; never painted.
pub fn canvas(scheme: Scheme) -> (u8, u8, u8) {
    match scheme {
        Scheme::Dark => hex(0x050409),
        Scheme::Light => hex(0xFCFAFE),
    }
}

/// Brand profile values for a role.
pub fn paint(role: Role, scheme: Scheme) -> RolePaint {
    let fg = |dark: u32, light: u32| RolePaint {
        fg: hex(match scheme {
            Scheme::Dark => dark,
            Scheme::Light => light,
        }),
        bg: None,
    };
    let pair = |dark: (u32, u32), light: (u32, u32)| {
        let (fg, bg) = match scheme {
            Scheme::Dark => dark,
            Scheme::Light => light,
        };
        RolePaint {
            fg: hex(fg),
            bg: Some(hex(bg)),
        }
    };
    match role {
        Role::Text => fg(0xF7F3FA, 0x1A1320),
        Role::TextMuted => fg(0xCBBFD3, 0x5D5265),
        Role::TextSubtle => fg(0xA998B5, 0x74667E),
        // Brand anchor; used for marks, never for body text.
        Role::Identity => fg(0x8E3DFF, 0x7A22EE),
        // oc-on-action on oc-action.
        Role::Action => pair((0xFFFFFF, 0x8E3DFF), (0xFFFFFF, 0x7A22EE)),
        Role::Focus | Role::Link => fg(0xC9A7FF, 0x6E19DA),
        Role::Presence => fg(0xB991FF, 0x5E2A8A),
        // oc-text on oc-surface-3.
        Role::Selection => pair((0xF7F3FA, 0x211A2B), (0x1A1320, 0xECE3F2)),
        Role::Border => fg(0x5B4B69, 0xB9A8C4),
        Role::Verified => pair((0x93E6B4, 0x11281B), (0x17683A, 0xE7F6EC)),
        Role::Pending => pair((0xFFD27A, 0x2B210D), (0x704B00, 0xFFF4D3)),
        Role::Blocked => pair((0xFFB4AC, 0x321517), (0x8A1C1C, 0xFDEBEC)),
        Role::Evidence => pair((0xE8E0F0, 0x201A25), (0x4A3F51, 0xF2EDF5)),
        Role::DiffAdded => fg(0x93E6B4, 0x17683A),
        Role::DiffRemoved | Role::Sensitive => fg(0xFFB4AC, 0x8A1C1C),
    }
}

/// Text attribute carried by a role in every color mode, so meaning
/// survives monochrome terminals.
fn attributes(role: Role) -> Modifier {
    match role {
        Role::Identity | Role::Blocked | Role::Sensitive => Modifier::BOLD,
        Role::Focus => Modifier::BOLD | Modifier::UNDERLINED,
        Role::Link => Modifier::UNDERLINED,
        Role::Action => Modifier::BOLD,
        _ => Modifier::empty(),
    }
}

/// Named-color mapping for terminals whose palette the user controls.
fn named(role: Role) -> (Color, Option<Color>) {
    match role {
        Role::Text | Role::Evidence => (Color::Reset, None),
        // Gray and DarkGray vanish on light or Solarized palettes; muted
        // tiers use the terminal's own text color, dimmed (see `style`).
        Role::TextMuted | Role::TextSubtle | Role::Border => (Color::Reset, None),
        Role::Identity | Role::Presence => (Color::Magenta, None),
        Role::Action => (Color::White, Some(Color::Magenta)),
        Role::Focus | Role::Link => (Color::LightMagenta, None),
        Role::Selection => (Color::Reset, None),
        Role::Verified | Role::DiffAdded => (Color::Green, None),
        Role::Pending => (Color::Yellow, None),
        Role::Blocked | Role::DiffRemoved | Role::Sensitive => (Color::Red, None),
    }
}

/// The xterm 256-color palette entry nearest to `rgb` (cube and gray ramp;
/// the 16 system colors are user-defined and skipped).
pub fn nearest_xterm256(rgb: (u8, u8, u8)) -> u8 {
    let distance = |a: (u8, u8, u8), b: (u8, u8, u8)| {
        let d = |x: u8, y: u8| (i32::from(x) - i32::from(y)).pow(2);
        d(a.0, b.0) + d(a.1, b.1) + d(a.2, b.2)
    };
    (16u8..=255)
        .min_by_key(|&index| distance(rgb, xterm256_rgb(index)))
        .unwrap_or(16)
}

impl Appearance {
    pub fn style(&self, role: Role) -> Style {
        let mut style = Style::default().add_modifier(attributes(role));
        match self.color {
            ColorMode::NoColor => {
                if role == Role::Selection || role == Role::Action {
                    style = style.add_modifier(Modifier::REVERSED);
                }
                if matches!(role, Role::TextMuted | Role::TextSubtle) {
                    style = style.add_modifier(Modifier::DIM);
                }
            }
            ColorMode::Ansi16 => {
                let (fg, bg) = named(role);
                style = style.fg(fg);
                if let Some(bg) = bg {
                    style = style.bg(bg);
                }
                if role == Role::Selection {
                    style = style.add_modifier(Modifier::REVERSED);
                }
                if matches!(role, Role::TextMuted | Role::TextSubtle | Role::Border) {
                    style = style.add_modifier(Modifier::DIM);
                }
            }
            ColorMode::TrueColor | ColorMode::Ansi256 => {
                let paint = paint(role, self.scheme);
                let color = |(r, g, b): (u8, u8, u8)| match self.color {
                    ColorMode::Ansi256 => Color::Indexed(nearest_xterm256((r, g, b))),
                    _ => Color::Rgb(r, g, b),
                };
                style = style.fg(color(paint.fg));
                if let Some(bg) = paint.bg {
                    style = style.bg(color(bg));
                }
                if self.color == ColorMode::Ansi256 {
                    if let Some(bg) = xterm256_fill_override(role, self.scheme) {
                        style = style.bg(Color::Indexed(bg));
                    }
                }
            }
        }
        style
    }
}

/// Fills whose nearest xterm index loses the pair's contrast; each value is
/// the nearest index that keeps 4.5:1 with the role's approximated text.
fn xterm256_fill_override(role: Role, scheme: Scheme) -> Option<u8> {
    match (role, scheme) {
        // #8E3DFF maps to 99, where white text drops to 4.1:1; 98 keeps 4.5:1.
        (Role::Action, Scheme::Dark) => Some(98),
        _ => None,
    }
}

/// WCAG relative luminance contrast ratio between two RGB colors.
pub fn contrast_ratio(a: (u8, u8, u8), b: (u8, u8, u8)) -> f64 {
    fn channel(value: u8) -> f64 {
        let value = f64::from(value) / 255.0;
        if value <= 0.03928 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    }
    let luminance =
        |(r, g, b): (u8, u8, u8)| 0.2126 * channel(r) + 0.7152 * channel(g) + 0.0722 * channel(b);
    let (la, lb) = (luminance(a), luminance(b));
    let (hi, lo) = if la >= lb { (la, lb) } else { (lb, la) };
    (hi + 0.05) / (lo + 0.05)
}

/// RGB value of an xterm 256-color cube/gray index (16..=255).
pub fn xterm256_rgb(index: u8) -> (u8, u8, u8) {
    const LEVELS: [u8; 6] = [0, 95, 135, 175, 215, 255];
    if index >= 232 {
        let gray = 8 + (index - 232) * 10;
        return (gray, gray, gray);
    }
    let n = index.saturating_sub(16);
    (
        LEVELS[usize::from(n / 36)],
        LEVELS[usize::from((n / 6) % 6)],
        LEVELS[usize::from(n % 6)],
    )
}

impl Appearance {
    /// Resolve capabilities from presentation settings, the existing
    /// `reduceMotion` setting and the environment (`env` returns a variable's
    /// value). Explicit settings win; `auto` consults the environment.
    pub fn resolve(
        settings: &claurst_core::appearance::NextAppearanceSettings,
        reduce_motion: bool,
        env: impl Fn(&str) -> Option<String>,
    ) -> Self {
        use claurst_core::appearance::{ColorChoice, GlyphChoice, ThemeChoice};
        let set = |name: &str| env(name).filter(|value| !value.is_empty());
        let term = set("TERM").unwrap_or_default();
        let color = match settings.color {
            // NO_COLOR is an explicit user request and beats `auto` only;
            // an explicit colorMode is a deliberate per-app override.
            ColorChoice::Auto if set("NO_COLOR").is_some() || term == "dumb" => ColorMode::NoColor,
            ColorChoice::Auto => match set("COLORTERM").as_deref() {
                Some("truecolor" | "24bit") => ColorMode::TrueColor,
                _ if term.contains("256color") => ColorMode::Ansi256,
                _ => ColorMode::Ansi16,
            },
            ColorChoice::TrueColor => ColorMode::TrueColor,
            ColorChoice::Ansi256 => ColorMode::Ansi256,
            ColorChoice::Ansi16 => ColorMode::Ansi16,
            ColorChoice::None => ColorMode::NoColor,
        };
        // `None` when the theme is `auto` and the terminal does not report
        // its background.
        let scheme = match settings.theme {
            ThemeChoice::Dark => Some(Scheme::Dark),
            ThemeChoice::Light => Some(Scheme::Light),
            // COLORFGBG is "fg;bg" (sometimes "fg;x;bg"); 7 and 15 are light.
            ThemeChoice::Auto => set("COLORFGBG")
                .and_then(|value| value.rsplit(';').next().map(str::to_string))
                .map(|bg| match bg.as_str() {
                    "7" | "15" => Scheme::Light,
                    _ => Scheme::Dark,
                }),
        };
        // Brand colors assume a known canvas. On an unknown background, use
        // the terminal's own palette instead (text tiers become dimmed
        // default text); an explicit theme restores the Brand palette.
        let color = match (color, scheme) {
            (ColorMode::TrueColor | ColorMode::Ansi256, None) => ColorMode::Ansi16,
            (color, _) => color,
        };
        let scheme = scheme.unwrap_or(Scheme::Dark);
        let glyphs = match settings.glyphs {
            GlyphChoice::Unicode => Glyphs::Unicode,
            GlyphChoice::Ascii => Glyphs::Ascii,
            GlyphChoice::Auto => {
                let locale = set("LC_ALL")
                    .or_else(|| set("LC_CTYPE"))
                    .or_else(|| set("LANG"))
                    .unwrap_or_default()
                    .to_ascii_lowercase();
                // CJK locales draw East Asian ambiguous-width marks (›, …,
                // ─) two cells wide, which breaks alignment.
                let ambiguous_wide = ["ja", "ko", "zh"]
                    .iter()
                    .any(|prefix| locale.starts_with(prefix));
                if term != "dumb"
                    && !ambiguous_wide
                    && (locale.contains("utf-8") || locale.contains("utf8"))
                {
                    Glyphs::Unicode
                } else {
                    Glyphs::Ascii
                }
            }
        };
        Self {
            color,
            scheme,
            glyphs,
            motion: if reduce_motion {
                Motion::Reduced
            } else {
                Motion::Full
            },
        }
    }
}
