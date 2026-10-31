//! Presentation-only appearance settings for the Next renderer
//! (`nextAppearance` in settings.json).
//!
//! These values change how the terminal draws, never what the engine may do:
//! no field here affects permissions, providers, identity or stored state.
//! Parsing is lenient so a bad value cannot break settings loading: each
//! field falls back to its default on an invalid value, and both unknown
//! keys and rejected values are written back unchanged on save.

use serde::de::Deserializer;
use serde::ser::{SerializeMap, Serializer};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

macro_rules! choice {
    ($(#[$meta:meta])* $name:ident { $($variant:ident => $text:literal),+ $(,)? }) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
        pub enum $name {
            #[default]
            $($variant),+
        }

        impl $name {
            pub const VALUES: &'static [&'static str] = &[$($text),+];

            pub fn parse(text: &str) -> Option<Self> {
                match text {
                    $($text => Some(Self::$variant),)+
                    _ => None,
                }
            }

            pub fn as_str(self) -> &'static str {
                match self {
                    $(Self::$variant => $text),+
                }
            }
        }
    };
}

choice!(
    /// Light/dark scheme; `auto` follows `COLORFGBG`.
    ThemeChoice { Auto => "auto", Dark => "dark", Light => "light" }
);
choice!(
    /// Color capability override; `auto` detects from the environment.
    ColorChoice { Auto => "auto", TrueColor => "truecolor", Ansi256 => "256", Ansi16 => "16", None => "none" }
);
choice!(
    /// Glyph set; `auto` uses Unicode only under a UTF-8 locale.
    GlyphChoice { Auto => "auto", Unicode => "unicode", Ascii => "ascii" }
);
choice!(
    /// Vertical spacing between transcript entries.
    Density { Comfortable => "comfortable", Compact => "compact" }
);
choice!(
    /// Preferred layout; Work and Inspect fall back to Focus until #410.
    LayoutChoice { Focus => "focus", Work => "work", Inspect => "inspect" }
);

#[derive(Debug, Clone, Default, PartialEq)]
pub struct NextAppearanceSettings {
    pub theme: ThemeChoice,
    pub color: ColorChoice,
    pub glyphs: GlyphChoice,
    pub density: Density,
    pub layout: LayoutChoice,
    /// Show the key-hint row.
    pub hints: bool,
    /// Keys this version does not know, and known keys whose value was
    /// rejected; serialized back verbatim.
    preserved: BTreeMap<String, serde_json::Value>,
    /// Known keys whose value was rejected, in file order.
    rejected: Vec<String>,
    /// Whether `hints` was set explicitly (so defaults are not written out).
    hints_set: bool,
}

impl NextAppearanceSettings {
    /// Keys whose values were rejected and replaced by defaults.
    pub fn rejected_keys(&self) -> &[String] {
        &self.rejected
    }

    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

impl<'de> Deserialize<'de> for NextAppearanceSettings {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(deserializer)?;
        let mut settings = Self::default();
        let serde_json::Value::Object(map) = value else {
            // Not an object: keep nothing typed, but never fail settings load.
            settings.rejected.push(String::new());
            return Ok(settings);
        };
        for (key, value) in map {
            let text = value.as_str();
            let accepted = match key.as_str() {
                "theme" => text
                    .and_then(ThemeChoice::parse)
                    .map(|v| settings.theme = v),
                "colorMode" => text
                    .and_then(ColorChoice::parse)
                    .map(|v| settings.color = v),
                "glyphs" => text
                    .and_then(GlyphChoice::parse)
                    .map(|v| settings.glyphs = v),
                "density" => text.and_then(Density::parse).map(|v| settings.density = v),
                "layout" => text
                    .and_then(LayoutChoice::parse)
                    .map(|v| settings.layout = v),
                "hints" => value.as_bool().map(|v| {
                    settings.hints = v;
                    settings.hints_set = true;
                }),
                _ => {
                    settings.preserved.insert(key, value);
                    continue;
                }
            };
            if accepted.is_none() {
                settings.rejected.push(key.clone());
                settings.preserved.insert(key, value);
            }
        }
        Ok(settings)
    }
}

impl Serialize for NextAppearanceSettings {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(None)?;
        let typed: [(&str, Option<&str>); 5] = [
            (
                "theme",
                Some(self.theme)
                    .filter(|v| *v != ThemeChoice::default())
                    .map(ThemeChoice::as_str),
            ),
            (
                "colorMode",
                Some(self.color)
                    .filter(|v| *v != ColorChoice::default())
                    .map(ColorChoice::as_str),
            ),
            (
                "glyphs",
                Some(self.glyphs)
                    .filter(|v| *v != GlyphChoice::default())
                    .map(GlyphChoice::as_str),
            ),
            (
                "density",
                Some(self.density)
                    .filter(|v| *v != Density::default())
                    .map(Density::as_str),
            ),
            (
                "layout",
                Some(self.layout)
                    .filter(|v| *v != LayoutChoice::default())
                    .map(LayoutChoice::as_str),
            ),
        ];
        for (key, value) in typed {
            if let Some(value) = value {
                map.serialize_entry(key, value)?;
            }
        }
        if self.hints_set {
            map.serialize_entry("hints", &self.hints)?;
        }
        for (key, value) in &self.preserved {
            // A rejected known key keeps the user's original text.
            map.serialize_entry(key, value)?;
        }
        map.end()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_values_round_trip_and_defaults_stay_out_of_the_file() {
        let parsed: NextAppearanceSettings = serde_json::from_str(
            r#"{"theme":"light","colorMode":"256","glyphs":"ascii","density":"compact","layout":"work","hints":true}"#,
        )
        .unwrap();
        assert_eq!(parsed.theme, ThemeChoice::Light);
        assert_eq!(parsed.color, ColorChoice::Ansi256);
        assert_eq!(parsed.glyphs, GlyphChoice::Ascii);
        assert_eq!(parsed.density, Density::Compact);
        assert_eq!(parsed.layout, LayoutChoice::Work);
        assert!(parsed.hints);
        assert!(parsed.rejected_keys().is_empty());
        let again: NextAppearanceSettings =
            serde_json::from_str(&serde_json::to_string(&parsed).unwrap()).unwrap();
        assert_eq!(again, parsed);

        let empty = NextAppearanceSettings::default();
        assert_eq!(serde_json::to_string(&empty).unwrap(), "{}");
        assert!(empty.is_default());
    }

    #[test]
    fn invalid_values_fall_back_and_are_preserved_with_unknown_keys() {
        let parsed: NextAppearanceSettings = serde_json::from_str(
            r#"{"theme":"neon","hints":"yes","futureKey":{"nested":[1,2]},"glyphs":"ascii"}"#,
        )
        .unwrap();
        assert_eq!(parsed.theme, ThemeChoice::Auto);
        assert!(!parsed.hints);
        assert_eq!(parsed.glyphs, GlyphChoice::Ascii);
        let mut rejected = parsed.rejected_keys().to_vec();
        rejected.sort();
        assert_eq!(rejected, ["hints", "theme"]);
        let written: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&parsed).unwrap()).unwrap();
        assert_eq!(
            written,
            serde_json::json!({
                "glyphs": "ascii",
                "theme": "neon",
                "hints": "yes",
                "futureKey": {"nested": [1, 2]}
            })
        );
    }

    #[test]
    fn a_non_object_value_never_fails_settings_load() {
        let parsed: NextAppearanceSettings = serde_json::from_str(r#""dark""#).unwrap();
        assert_eq!(parsed.theme, ThemeChoice::Auto);
        assert_eq!(parsed.rejected_keys().len(), 1);
    }
}
