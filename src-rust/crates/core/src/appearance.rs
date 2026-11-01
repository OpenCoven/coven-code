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
use std::collections::{BTreeMap, BTreeSet};

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

/// Keys this version reads.
const KEYS: &[&str] = &["theme", "colorMode", "glyphs", "density", "layout", "hints"];

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
    /// Keys this layer set to a valid value, so merging knows which win.
    set: BTreeSet<&'static str>,
    /// A non-object value, kept so saving settings never rewrites it.
    unparsed: Option<serde_json::Value>,
}

impl NextAppearanceSettings {
    /// Keys whose values were rejected and replaced by defaults.
    pub fn rejected_keys(&self) -> &[String] {
        &self.rejected
    }

    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }

    /// Combine two settings layers key by key: each key `over` set to a
    /// valid value wins; everything else, including unknown keys, comes
    /// from `base` unless `over` carries it. A layer that could not be read
    /// as an object changes nothing.
    pub fn merge(base: Self, over: Self) -> Self {
        if over.unparsed.is_some() {
            return base;
        }
        let mut merged = base;
        for key in &over.set {
            match *key {
                "theme" => merged.theme = over.theme,
                "colorMode" => merged.color = over.color,
                "glyphs" => merged.glyphs = over.glyphs,
                "density" => merged.density = over.density,
                "layout" => merged.layout = over.layout,
                "hints" => merged.hints = over.hints,
                _ => {}
            }
            merged.set.insert(*key);
            merged.preserved.remove(*key);
            merged.rejected.retain(|rejected| rejected.as_str() != *key);
        }
        for (key, value) in over.preserved {
            // A rejected higher value never hides a valid lower one.
            if !merged.set.contains(key.as_str()) {
                merged.preserved.insert(key, value);
            }
        }
        for key in over.rejected {
            if !merged.set.contains(key.as_str()) && !merged.rejected.contains(&key) {
                merged.rejected.push(key);
            }
        }
        merged.unparsed = None;
        merged
    }
}

impl<'de> Deserialize<'de> for NextAppearanceSettings {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(deserializer)?;
        let mut settings = Self::default();
        let serde_json::Value::Object(map) = value else {
            // Not an object: keep nothing typed, but never fail settings load.
            settings.rejected.push(String::new());
            settings.unparsed = Some(value);
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
                }),
                _ => {
                    settings.preserved.insert(key, value);
                    continue;
                }
            };
            if accepted.is_some() {
                if let Some(known) = KEYS.iter().find(|known| **known == key) {
                    settings.set.insert(known);
                }
            } else {
                settings.rejected.push(key.clone());
                settings.preserved.insert(key, value);
            }
        }
        Ok(settings)
    }
}

impl Serialize for NextAppearanceSettings {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        if let Some(value) = &self.unparsed {
            return value.serialize(serializer);
        }
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
        if self.set.contains("hints") {
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
    fn layers_merge_per_field_and_the_higher_layer_wins_each_key() {
        let user: NextAppearanceSettings =
            serde_json::from_str(r#"{"colorMode":"256","theme":"dark","oldKey":1}"#).unwrap();
        let project: NextAppearanceSettings =
            serde_json::from_str(r#"{"hints":true,"theme":"light","newKey":2}"#).unwrap();
        let merged = NextAppearanceSettings::merge(user.clone(), project);
        assert_eq!(merged.color, ColorChoice::Ansi256, "user key survives");
        assert_eq!(merged.theme, ThemeChoice::Light, "project key wins");
        assert!(merged.hints);
        assert_eq!(
            serde_json::to_value(&merged).unwrap(),
            serde_json::json!({"colorMode":"256","theme":"light","hints":true,"oldKey":1,"newKey":2})
        );

        // An invalid higher value keeps the lower layer's valid one, once.
        let invalid: NextAppearanceSettings = serde_json::from_str(r#"{"theme":"plaid"}"#).unwrap();
        let merged = NextAppearanceSettings::merge(user.clone(), invalid);
        assert_eq!(merged.theme, ThemeChoice::Dark);
        assert_eq!(serde_json::to_value(&merged).unwrap()["theme"], "dark");

        // A layer that does not set the key, or cannot be read, changes nothing.
        let empty = NextAppearanceSettings::default();
        assert_eq!(NextAppearanceSettings::merge(user.clone(), empty), user);
        let unreadable: NextAppearanceSettings = serde_json::from_str(r#""dark""#).unwrap();
        assert_eq!(
            NextAppearanceSettings::merge(user.clone(), unreadable),
            user
        );
    }

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
