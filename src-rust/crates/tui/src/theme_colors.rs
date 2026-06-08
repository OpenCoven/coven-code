// theme_colors.rs — Color palette management for accessibility-friendly themes.
//
// Provides color definitions for different themes, with special support for
// Deuteranopia (red-green color blindness) using blue, yellow, and gray palettes.

use ratatui::style::Color;

/// Color palette for a specific theme.
pub struct ColorPalette {
    /// Error messages and alerts (normally red, but color-blind friendly)
    pub error: Color,
    /// Success indicators (normally green, but color-blind friendly)
    pub success: Color,
    /// Warning/caution messages
    pub warning: Color,
    /// Information messages
    pub info: Color,
    /// Action buttons and interactive elements
    pub action: Color,
    /// Disabled or dimmed states
    pub disabled: Color,
    /// Primary accent color
    pub accent: Color,
    /// Secondary accent
    pub secondary_accent: Color,
    /// Text on dark backgrounds
    pub text_light: Color,
    /// Text on light backgrounds
    pub text_dark: Color,
    /// Borders and dividers
    pub border: Color,
}

impl ColorPalette {
    /// Get the color palette for a given theme name.
    pub fn for_theme(theme_name: &str) -> Self {
        match theme_name {
            "coven" | "coven-code" => Self::default_theme(),
            "coven-dark" => Self::coven_dark(),
            "deuteranopia" => Self::deuteranopia(),
            "dark" => Self::dark(),
            "light" => Self::light(),
            "solarized" => Self::solarized(),
            "nord" => Self::nord(),
            "dracula" => Self::dracula(),
            "monokai" => Self::monokai(),
            _ => Self::default_theme(),
        }
    }

    /// Default Coven Code theme — OpenCoven brand palette
    ///
    /// Violet-first dark theme matching the OpenCoven brand:
    ///   primary/accent:   #8B5CF6  violet-500
    ///   secondary_accent: #EC4899  pink-500
    ///   action:           #A78BFA  violet-400 (slightly lighter for interactive)
    ///   info:             #C4B5FD  violet-300 (readable on dark)
    ///   border:           #4C1D95  violet-900 (dark border, not grey)
    ///   success:          #34D399  emerald-400 (distinct from violet)
    ///   warning:          #FBBF24  amber-400
    ///   error:            #F87171  red-400
    fn default_theme() -> Self {
        Self {
            // Violet/pink brand palette
            accent: Color::Rgb(139, 92, 246), // #8B5CF6 violet-500
            secondary_accent: Color::Rgb(236, 72, 153), // #EC4899 pink-500
            action: Color::Rgb(167, 139, 250), // #A78BFA violet-400
            info: Color::Rgb(196, 181, 253),  // #C4B5FD violet-300
            border: Color::Rgb(76, 29, 149),  // #4C1D95 violet-900
            // Semantic states — distinct from violet family
            success: Color::Rgb(52, 211, 153), // #34D399 emerald-400
            warning: Color::Rgb(251, 191, 36), // #FBBF24 amber-400
            error: Color::Rgb(248, 113, 113),  // #F87171 red-400
            // Text / disabled
            disabled: Color::Rgb(109, 40, 217), // #6D28D9 violet-700 (muted)
            text_light: Color::White,
            text_dark: Color::Black,
        }
    }

    /// Coven Code dark variant — violet palette on a true dark background
    fn coven_dark() -> Self {
        Self {
            accent: Color::Rgb(139, 92, 246),           // #8B5CF6 violet-500
            secondary_accent: Color::Rgb(236, 72, 153), // #EC4899 pink-500
            action: Color::Rgb(167, 139, 250),          // #A78BFA violet-400
            info: Color::Rgb(196, 181, 253),            // #C4B5FD violet-300
            border: Color::Rgb(55, 14, 110),            // #37006E violet-950
            success: Color::Rgb(52, 211, 153),          // #34D399 emerald-400
            warning: Color::Rgb(251, 191, 36),          // #FBBF24 amber-400
            error: Color::Rgb(248, 113, 113),           // #F87171 red-400
            disabled: Color::Rgb(76, 29, 149),          // #4C1D95 violet-900 (dimmer)
            text_light: Color::Rgb(237, 233, 254),      // #EDE9FE violet-100
            text_dark: Color::Rgb(15, 5, 40),           // near-black with violet tint
        }
    }

    /// Dark theme
    fn dark() -> Self {
        Self {
            error: Color::Rgb(239, 83, 80),     // Light red
            success: Color::Rgb(129, 199, 132), // Light green
            warning: Color::Rgb(255, 171, 64),  // Light orange
            info: Color::Rgb(100, 181, 246),    // Light blue
            action: Color::Rgb(100, 181, 246),
            disabled: Color::Rgb(97, 97, 97),
            accent: Color::Rgb(100, 181, 246),
            secondary_accent: Color::Rgb(229, 57, 53),
            text_light: Color::Rgb(229, 229, 229),
            text_dark: Color::Rgb(33, 33, 33),
            border: Color::Rgb(66, 66, 66),
        }
    }

    /// Light theme
    fn light() -> Self {
        Self {
            error: Color::Rgb(211, 47, 47),    // Dark red
            success: Color::Rgb(27, 94, 32),   // Dark green
            warning: Color::Rgb(230, 124, 13), // Dark orange
            info: Color::Rgb(13, 71, 161),     // Dark blue
            action: Color::Blue,
            disabled: Color::Rgb(189, 189, 189),
            accent: Color::Blue,
            secondary_accent: Color::Rgb(194, 24, 91),
            text_light: Color::White,
            text_dark: Color::Black,
            border: Color::Rgb(189, 189, 189),
        }
    }

    /// Solarized Dark theme
    fn solarized() -> Self {
        Self {
            error: Color::Rgb(220, 50, 47),   // Solarized red
            success: Color::Rgb(133, 153, 0), // Solarized green
            warning: Color::Rgb(181, 137, 0), // Solarized yellow
            info: Color::Rgb(38, 139, 210),   // Solarized blue
            action: Color::Rgb(38, 139, 210),
            disabled: Color::Rgb(88, 110, 117),
            accent: Color::Rgb(38, 139, 210),
            secondary_accent: Color::Rgb(108, 113, 196),
            text_light: Color::Rgb(131, 148, 150),
            text_dark: Color::Rgb(0, 43, 54),
            border: Color::Rgb(7, 54, 66),
        }
    }

    /// Nord theme
    fn nord() -> Self {
        Self {
            error: Color::Rgb(191, 97, 106),    // Nord red
            success: Color::Rgb(163, 190, 140), // Nord green
            warning: Color::Rgb(235, 203, 139), // Nord yellow
            info: Color::Rgb(136, 192, 208),    // Nord blue
            action: Color::Rgb(136, 192, 208),
            disabled: Color::Rgb(76, 86, 106),
            accent: Color::Rgb(136, 192, 208),
            secondary_accent: Color::Rgb(191, 97, 106),
            text_light: Color::Rgb(236, 239, 244),
            text_dark: Color::Rgb(46, 52, 64),
            border: Color::Rgb(67, 76, 94),
        }
    }

    /// Dracula theme
    fn dracula() -> Self {
        Self {
            error: Color::Rgb(255, 85, 85),     // Dracula red
            success: Color::Rgb(80, 250, 123),  // Dracula green
            warning: Color::Rgb(241, 250, 140), // Dracula yellow
            info: Color::Rgb(139, 233, 253),    // Dracula blue
            action: Color::Rgb(139, 233, 253),
            disabled: Color::Rgb(98, 114, 164),
            accent: Color::Rgb(139, 233, 253),
            secondary_accent: Color::Rgb(189, 147, 249),
            text_light: Color::Rgb(248, 248, 242),
            text_dark: Color::Rgb(40, 42, 54),
            border: Color::Rgb(68, 71, 90),
        }
    }

    /// Monokai theme
    fn monokai() -> Self {
        Self {
            error: Color::Rgb(249, 38, 114), // Monokai magenta (used for errors)
            success: Color::Rgb(166, 226, 46), // Monokai green
            warning: Color::Rgb(253, 151, 31), // Monokai orange
            info: Color::Rgb(102, 217, 239), // Monokai cyan
            action: Color::Rgb(102, 217, 239),
            disabled: Color::Rgb(117, 113, 94),
            accent: Color::Rgb(102, 217, 239),
            secondary_accent: Color::Rgb(249, 38, 114),
            text_light: Color::Rgb(248, 248, 242),
            text_dark: Color::Rgb(39, 40, 34),
            border: Color::Rgb(75, 75, 75),
        }
    }

    /// Deuteranopia (red-green color blind) theme
    /// Uses blue, yellow, and gray to avoid red/green distinction
    fn deuteranopia() -> Self {
        Self {
            error: Color::Rgb(255, 140, 0),   // Orange (not red)
            success: Color::Rgb(0, 150, 200), // Blue (not green)
            warning: Color::Rgb(255, 180, 0), // Gold/Yellow
            info: Color::Cyan,
            action: Color::Rgb(0, 150, 200), // Blue action buttons
            disabled: Color::Rgb(120, 120, 120), // Neutral gray
            accent: Color::Rgb(0, 150, 200), // Blue accent
            secondary_accent: Color::Rgb(180, 140, 255), // Purple accent
            text_light: Color::Rgb(220, 220, 220),
            text_dark: Color::Rgb(40, 40, 40),
            border: Color::Rgb(100, 100, 100),
        }
    }
}

/// Get appropriate color for a given theme based on message type/role.
pub fn get_message_indicator_color(theme_name: &str, role: &str) -> Color {
    let palette = ColorPalette::for_theme(theme_name);
    match role {
        "user" => palette.accent,
        "assistant" => palette.secondary_accent,
        "system" => palette.disabled,
        "tool" => palette.action,
        _ => palette.text_light,
    }
}

/// Get error indicator color for given theme (always prominent, never red in deuteranopia).
pub fn get_error_color(theme_name: &str) -> Color {
    ColorPalette::for_theme(theme_name).error
}

/// Get success indicator color for given theme (blue instead of green in deuteranopia).
pub fn get_success_color(theme_name: &str) -> Color {
    ColorPalette::for_theme(theme_name).success
}

/// Get warning indicator color for given theme (yellow/gold instead of orange in deuteranopia).
pub fn get_warning_color(theme_name: &str) -> Color {
    ColorPalette::for_theme(theme_name).warning
}

// ---------------------------------------------------------------------------
// Diff viewer palette
// ---------------------------------------------------------------------------

/// Theme-aware colors for the unified diff viewer.
///
/// Built from a [`ColorPalette`] so each theme gets sensible diff colors.
/// In particular the deuteranopia theme replaces the red/green removed/added
/// distinction with orange/blue — matching how the rest of the palette
/// handles error/success indicators — so users with red-green colour
/// blindness can still read diffs.
pub struct DiffPalette {
    /// Dim background tint for removed (`-`) lines.
    pub bg_removed: Color,
    /// Dim background tint for added (`+`) lines.
    pub bg_added: Color,
    /// Bright background highlight for word-level deletions inside a row.
    pub bg_word_del: Color,
    /// Bright background highlight for word-level insertions inside a row.
    pub bg_word_ins: Color,
    /// Soft foreground for removed text and the leading `-` marker.
    pub fg_removed: Color,
    /// Soft foreground for added text and the leading `+` marker.
    pub fg_added: Color,
    /// Dim line-number gutter colour.
    pub fg_gutter: Color,
    /// Accent colour for hunk header lines (`@@ ... @@`).
    pub fg_header: Color,
    /// Subtle background band behind hunk headers.
    pub bg_header: Color,
}

impl DiffPalette {
    /// Get the diff palette for a given theme name.
    pub fn for_theme(theme_name: &str) -> Self {
        match theme_name {
            "deuteranopia" => Self::deuteranopia(),
            "light" => Self::light(),
            _ => Self::default_dark(),
        }
    }

    /// Default dark-theme diff palette — classic dim-red / dim-green tint.
    /// Used by the OpenCoven default + coven-dark / dark / solarized / nord /
    /// dracula / monokai themes.
    fn default_dark() -> Self {
        Self {
            bg_removed: Color::Rgb(52, 18, 24),
            bg_added: Color::Rgb(14, 44, 22),
            bg_word_del: Color::Rgb(150, 38, 52),
            bg_word_ins: Color::Rgb(34, 120, 52),
            fg_removed: Color::Rgb(255, 168, 178),
            fg_added: Color::Rgb(168, 240, 184),
            fg_gutter: Color::Rgb(108, 108, 122),
            fg_header: Color::Rgb(167, 139, 250),
            bg_header: Color::Rgb(18, 18, 28),
        }
    }

    /// Light-theme variant — lighter tints with darker foreground.
    fn light() -> Self {
        Self {
            bg_removed: Color::Rgb(255, 224, 224),
            bg_added: Color::Rgb(220, 255, 220),
            bg_word_del: Color::Rgb(255, 170, 170),
            bg_word_ins: Color::Rgb(170, 240, 170),
            fg_removed: Color::Rgb(180, 30, 30),
            fg_added: Color::Rgb(30, 110, 30),
            fg_gutter: Color::Rgb(140, 140, 140),
            fg_header: Color::Rgb(90, 60, 200),
            bg_header: Color::Rgb(232, 232, 245),
        }
    }

    /// Deuteranopia variant — orange/blue instead of red/green so red-green
    /// colour-blind users can still distinguish removed and added rows.
    fn deuteranopia() -> Self {
        Self {
            // Dim row tints
            bg_removed: Color::Rgb(72, 36, 0), // dim orange
            bg_added: Color::Rgb(0, 36, 68),   // dim blue
            // Bright word highlights
            bg_word_del: Color::Rgb(200, 110, 0), // saturated orange
            bg_word_ins: Color::Rgb(0, 120, 200), // saturated blue
            // Foreground markers
            fg_removed: Color::Rgb(255, 196, 128), // soft orange text
            fg_added: Color::Rgb(168, 218, 255),   // soft blue text
            // Neutral gutter + violet header (shared)
            fg_gutter: Color::Rgb(140, 140, 140),
            fg_header: Color::Rgb(180, 140, 255),
            bg_header: Color::Rgb(20, 18, 28),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deuteranopia_diff_palette_avoids_red_and_green() {
        // The whole reason DiffPalette::deuteranopia exists is to drop the
        // red/green encoding. The removed row background must not be a
        // "more red than green" colour and the added row background must
        // not be a "more green than red" colour — otherwise the user with
        // red-green colour blindness cannot tell removed from added.
        let p = DiffPalette::deuteranopia();
        fn rgb(c: Color) -> (u8, u8, u8) {
            match c {
                Color::Rgb(r, g, b) => (r, g, b),
                _ => panic!("expected Rgb"),
            }
        }
        let (rr, rg, _) = rgb(p.bg_removed);
        let (ar, ag, _) = rgb(p.bg_added);
        assert!(
            rr > rg,
            "removed bg in deuteranopia should lean orange (R>G), got R={rr} G={rg}"
        );
        assert!(
            ag.saturating_sub(ar) > 10 || rgb(p.bg_added).2 > ar.max(ag),
            "added bg in deuteranopia should lean blue, got {:?}",
            p.bg_added
        );
    }

    #[test]
    fn for_theme_falls_back_to_default_dark_for_unknown() {
        let unknown = DiffPalette::for_theme("totally-not-a-theme");
        let default = DiffPalette::default_dark();
        assert_eq!(
            format!("{:?}", unknown.bg_removed),
            format!("{:?}", default.bg_removed)
        );
    }
}
