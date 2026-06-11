//! Familiar theming — resolves any familiar id (built-in or user-defined from
//! `~/.coven/familiars.toml`) to a stable [`FamiliarTheme`] used by
//! [`crate::familiar_card`] when composing the static themed card shown in the
//! welcome panel, F2 switcher, and `/agents` detail view.
//!
//! Built-in familiars (`kitty`, `nova`, `cody`, `charm`, `sage`, `astra`,
//! `echo`) get hand-tuned palettes + their existing pixel-art archetypes.
//!
//! User-defined familiars get a procedurally derived palette + sigil
//! archetype hashed from the lowercased id so the same familiar always
//! looks the same across sessions and machines without persisting extra
//! state in `familiars.toml`.
//!
//! The access tier flows through verbatim from `coven_shared` — it drives the
//! coloured tier dot drawn on the card.
//!
//! ## Public surface
//!
//! ```ignore
//! let theme = familiar_theme::resolve(id, daemon_familiars);
//! let card  = familiar_card::render_card(&theme, size, pose);
//! ```

use claurst_core::coven_shared::CovenFamiliar;
use ratatui::style::Color;

// ── Palette ──────────────────────────────────────────────────────────────────

/// Four-color palette covering body fill, accent details, eye sockets, and
/// the deep background behind the eyes. The eye socket follows each
/// familiar's hue (a light tint for legibility against the shared dark
/// `eye_bg`), so the eye rendering helpers in [`crate::mascot`] can stay
/// archetype-agnostic while the eyes still read as "this familiar".
#[derive(Debug, Clone, Copy)]
pub struct FamiliarPalette {
    pub primary: Color,
    pub accent: Color,
    pub eye_socket: Color,
    pub eye_bg: Color,
}

impl FamiliarPalette {
    const fn from_rgb(primary: (u8, u8, u8), accent: (u8, u8, u8), eye: (u8, u8, u8)) -> Self {
        Self {
            primary: Color::Rgb(primary.0, primary.1, primary.2),
            accent: Color::Rgb(accent.0, accent.1, accent.2),
            eye_socket: Color::Rgb(eye.0, eye.1, eye.2),
            eye_bg: Color::Rgb(15, 5, 40),
        }
    }
}

// ── Archetype ────────────────────────────────────────────────────────────────

/// Which renderer in [`crate::mascot`] / [`crate::familiar_card`] draws the
/// glyph body. The first seven variants map to the hand-crafted built-ins;
/// `SigilCrystal`/`SigilHex`/`SigilRune`/`SigilSeal` are procedural frames
/// used for any user-defined familiar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Archetype {
    Cat,
    SorceressCrown,
    Robot,
    Heart,
    WizardBook,
    Moon,
    Ghost,
    SigilCrystal,
    SigilHex,
    SigilRune,
    SigilSeal,
}

// ── Theme ────────────────────────────────────────────────────────────────────

/// Everything the card renderer needs to draw one familiar.
///
/// All owned so the struct can be built freely per render frame without
/// borrowing from the [`crate::app::App`] or any daemon-loaded list.
#[derive(Debug, Clone)]
pub struct FamiliarTheme {
    pub id: String,
    pub display_name: String,
    pub emoji: String,
    pub role: Option<String>,
    /// Canonical access tier string from `coven_shared::canonicalize_access_tier`.
    /// One of `"full"`, `"read-only"`, `"search-only"`.
    pub access: String,
    pub palette: FamiliarPalette,
    pub archetype: Archetype,
}

impl FamiliarTheme {
    /// Color the access-tier dot uses on the card.
    pub fn access_color(&self) -> Color {
        match self.access.as_str() {
            "full" => Color::Rgb(34, 197, 94),          // emerald-500
            "read-only" => Color::Rgb(245, 158, 11),    // amber-500
            "search-only" => Color::Rgb(148, 163, 184), // slate-400
            _ => Color::Rgb(148, 163, 184),
        }
    }
}

// ── Built-in palettes ────────────────────────────────────────────────────────

/// Palettes for the seven hand-crafted built-ins. Each one breaks the old
/// uniform violet so familiars are visually distinct at a glance.
const BUILTIN_PALETTES: &[(&str, FamiliarPalette, Archetype, &str, &str)] = &[
    (
        "kitty",
        FamiliarPalette::from_rgb((139, 92, 246), (167, 139, 250), (196, 181, 253)),
        Archetype::Cat,
        "Kitty",
        "\u{1f431}",
    ),
    (
        "nova",
        FamiliarPalette::from_rgb((245, 197, 24), (253, 230, 138), (254, 240, 138)),
        Archetype::SorceressCrown,
        "Nova",
        "\u{1f451}",
    ),
    (
        "cody",
        FamiliarPalette::from_rgb((34, 211, 238), (165, 243, 252), (165, 243, 252)),
        Archetype::Robot,
        "Cody",
        "\u{1f4bb}",
    ),
    (
        "charm",
        FamiliarPalette::from_rgb((236, 72, 153), (251, 207, 232), (251, 207, 232)),
        Archetype::Heart,
        "Charm",
        "\u{2728}",
    ),
    (
        "sage",
        FamiliarPalette::from_rgb((16, 185, 129), (167, 243, 208), (167, 243, 208)),
        Archetype::WizardBook,
        "Sage",
        "\u{1f33f}",
    ),
    (
        "astra",
        FamiliarPalette::from_rgb((99, 102, 241), (199, 210, 254), (199, 210, 254)),
        Archetype::Moon,
        "Astra",
        "\u{1f319}",
    ),
    (
        "echo",
        FamiliarPalette::from_rgb((20, 184, 166), (153, 246, 228), (153, 246, 228)),
        Archetype::Ghost,
        "Echo",
        "\u{1f47b}",
    ),
];

/// Eight-color palette table used to pick a deterministic accent for any
/// user-defined familiar by hashing its id.
const PROCEDURAL_PALETTES: &[FamiliarPalette] = &[
    FamiliarPalette::from_rgb((139, 92, 246), (167, 139, 250), (196, 181, 253)), // violet
    FamiliarPalette::from_rgb((245, 197, 24), (253, 230, 138), (254, 240, 138)), // gold
    FamiliarPalette::from_rgb((34, 211, 238), (165, 243, 252), (165, 243, 252)), // cyan
    FamiliarPalette::from_rgb((236, 72, 153), (251, 207, 232), (251, 207, 232)), // pink
    FamiliarPalette::from_rgb((16, 185, 129), (167, 243, 208), (167, 243, 208)), // emerald
    FamiliarPalette::from_rgb((99, 102, 241), (199, 210, 254), (199, 210, 254)), // indigo
    FamiliarPalette::from_rgb((251, 113, 133), (254, 205, 211), (254, 205, 211)), // rose
    FamiliarPalette::from_rgb((250, 204, 21), (253, 224, 71), (254, 240, 138)),  // amber
];

const PROCEDURAL_ARCHETYPES: &[Archetype] = &[
    Archetype::SigilCrystal,
    Archetype::SigilHex,
    Archetype::SigilRune,
    Archetype::SigilSeal,
];

/// Resolve a familiar id to its theme.
///
/// Built-in ids win first. Anything else is matched against the supplied
/// `daemon_familiars` (callers pass [`coven_shared::load_familiars`] output).
/// Unknown ids fall back to the `kitty` theme so the welcome panel never
/// renders blank.
pub fn resolve(id: &str, daemon_familiars: &[CovenFamiliar]) -> FamiliarTheme {
    let lc = id.to_lowercase();
    if let Some(theme) = builtin(&lc) {
        return theme;
    }
    if let Some(def) = daemon_familiars.iter().find(|f| f.id.to_lowercase() == lc) {
        return procedural(def);
    }
    builtin("kitty").expect("kitty is always present in BUILTIN_PALETTES")
}

fn builtin(id: &str) -> Option<FamiliarTheme> {
    BUILTIN_PALETTES
        .iter()
        .find(|(slug, _, _, _, _)| *slug == id)
        .map(|(slug, palette, arch, name, emoji)| FamiliarTheme {
            id: (*slug).to_string(),
            display_name: (*name).to_string(),
            emoji: (*emoji).to_string(),
            role: None,
            access: builtin_access(slug).to_string(),
            palette: *palette,
            archetype: *arch,
        })
}

/// Built-in tier defaults match the recommendation table in `docs/familiars.md`:
/// `cody`, `nova`, `kitty` get `full`; the research-leaning rest stay read-only.
fn builtin_access(id: &str) -> &'static str {
    match id {
        "kitty" | "cody" | "nova" => "full",
        _ => "read-only",
    }
}

fn procedural(def: &CovenFamiliar) -> FamiliarTheme {
    let h = hash_id(&def.id);
    let palette = PROCEDURAL_PALETTES[(h % PROCEDURAL_PALETTES.len() as u64) as usize];
    let archetype = PROCEDURAL_ARCHETYPES[((h / 8) % PROCEDURAL_ARCHETYPES.len() as u64) as usize];

    FamiliarTheme {
        id: def.id.clone(),
        display_name: def.display_name.clone().unwrap_or_else(|| def.id.clone()),
        emoji: def.emoji.clone().unwrap_or_else(|| "\u{2728}".to_string()),
        role: def.role.clone(),
        access: def.resolved_access().to_string(),
        palette,
        archetype,
    }
}

/// FNV-1a 64-bit hash. Stable across machines/architectures so the same
/// familiar id always produces the same palette + archetype.
fn hash_id(id: &str) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in id.to_lowercase().as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_familiar(id: &str) -> CovenFamiliar {
        CovenFamiliar {
            id: id.to_string(),
            display_name: None,
            emoji: None,
            role: None,
            description: None,
            pronouns: None,
            access: None,
        }
    }

    #[test]
    fn builtin_resolution() {
        let t = resolve("kitty", &[]);
        assert_eq!(t.id, "kitty");
        assert!(matches!(t.archetype, Archetype::Cat));
    }

    #[test]
    fn case_insensitive_lookup() {
        let t = resolve("KITTY", &[]);
        assert_eq!(t.id, "kitty");
    }

    #[test]
    fn unknown_falls_back_to_kitty() {
        let t = resolve("does-not-exist", &[]);
        assert_eq!(t.id, "kitty");
    }

    #[test]
    fn user_defined_is_stable() {
        let f = vec![fake_familiar("qa")];
        let a = resolve("qa", &f);
        let b = resolve("qa", &f);
        // Same id must hash to same palette + archetype.
        assert_eq!(format!("{:?}", a.archetype), format!("{:?}", b.archetype));
        assert_eq!(
            format!("{:?}", a.palette.primary),
            format!("{:?}", b.palette.primary)
        );
    }

    #[test]
    fn eye_socket_palette_varies_by_familiar() {
        let kitty = resolve("kitty", &[]);
        let cody = resolve("cody", &[]);

        assert_ne!(
            kitty.palette.eye_socket, cody.palette.eye_socket,
            "eye sockets should use each familiar palette instead of one hardcoded violet"
        );
    }

    #[test]
    fn different_user_familiars_differ() {
        // Two distinct ids should map to either different palette or
        // different archetype most of the time. This isn't a strict
        // collision-resistance claim; we just want the hashing to spread.
        let f = vec![fake_familiar("qa"), fake_familiar("planner-bot")];
        let a = resolve("qa", &f);
        let b = resolve("planner-bot", &f);
        assert!(
            format!("{:?}", a.archetype) != format!("{:?}", b.archetype)
                || format!("{:?}", a.palette.primary) != format!("{:?}", b.palette.primary),
            "qa and planner-bot collided on both palette and archetype"
        );
    }

    #[test]
    fn user_emoji_passthrough() {
        let mut f = fake_familiar("qa");
        f.emoji = Some("\u{1f9ea}".to_string()); // 🧪
        let t = resolve("qa", &[f.clone()]);
        assert_eq!(t.emoji, "\u{1f9ea}");
    }
}
