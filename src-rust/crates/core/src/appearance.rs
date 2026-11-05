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
    /// Where the inspector starts: closed, on Work, or on Activity.
    LayoutChoice { Focus => "focus", Work => "work", Inspect => "inspect" }
);

/// Keys this version reads.
const KEYS: &[&str] = &["theme", "colorMode", "glyphs", "density", "layout", "hints"];

/// The top-level setting that sits beside `nextAppearance` and is edited with it.
pub const REDUCE_MOTION_KEY: &str = "reduceMotion";

/// Everything `/config appearance` can change, in display order.
pub const EDITABLE_KEYS: [&str; 7] = [
    "theme",
    "colorMode",
    "glyphs",
    "density",
    "layout",
    "hints",
    REDUCE_MOTION_KEY,
];

const FLAG_VALUES: &[&str] = &["true", "false"];

/// Values `key` accepts, or `None` when `key` is not an appearance setting.
pub fn choices_for(key: &str) -> Option<&'static [&'static str]> {
    Some(match key {
        "theme" => ThemeChoice::VALUES,
        "colorMode" => ColorChoice::VALUES,
        "glyphs" => GlyphChoice::VALUES,
        "density" => Density::VALUES,
        "layout" => LayoutChoice::VALUES,
        "hints" | REDUCE_MOTION_KEY => FLAG_VALUES,
        _ => return None,
    })
}

/// Pending changes to the user's appearance settings: each named key set to
/// one of its accepted values, or cleared back to its default. Only presentation keys can
/// get in here, so applying these can never reach permissions, providers or
/// identity.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AppearanceEdits {
    changes: BTreeMap<&'static str, Option<&'static str>>,
}

impl AppearanceEdits {
    /// Record `key = value`, or say why it cannot be set.
    pub fn set(&mut self, key: &str, value: &str) -> Result<(), String> {
        let Some(name) = EDITABLE_KEYS.iter().find(|known| **known == key) else {
            return Err(format!(
                "{key} is not an appearance setting. These are: {}.",
                EDITABLE_KEYS.join(", ")
            ));
        };
        let choices = choices_for(name).unwrap_or_default();
        let Some(choice) = choices.iter().find(|choice| **choice == value) else {
            return Err(format!(
                "{key} cannot be {value}. Choose one of: {}.",
                choices.join(", ")
            ));
        };
        self.changes.insert(name, Some(choice));
        Ok(())
    }

    /// Clear every appearance setting back to its default.
    pub fn reset_all(&mut self) {
        self.changes = EDITABLE_KEYS.iter().map(|key| (*key, None)).collect();
    }

    pub fn is_empty(&self) -> bool {
        self.changes.is_empty()
    }

    /// The keys these edits change.
    pub fn keys(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.changes.keys().copied()
    }

    /// The `reduceMotion` these edits ask for, when they change it.
    pub fn reduce_motion(&self) -> Option<bool> {
        self.changes
            .get(REDUCE_MOTION_KEY)
            .map(|value| *value == Some("true"))
    }

    /// Apply to a `nextAppearance` layer; `reduceMotion` is not in that
    /// object, see [`Self::reduce_motion`].
    pub fn apply_to(&self, settings: &mut NextAppearanceSettings) {
        for (key, value) in &self.changes {
            match value {
                Some(value) => settings.set_text(key, value),
                None => settings.clear_key(key),
            }
        }
    }
}

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

    /// Whether this layer held something other than an object.
    pub fn is_unreadable(&self) -> bool {
        self.unparsed.is_some()
    }

    /// A key's current value as settings.json spells it.
    pub fn value_text(&self, key: &str) -> Option<&'static str> {
        Some(match key {
            "theme" => self.theme.as_str(),
            "colorMode" => self.color.as_str(),
            "glyphs" => self.glyphs.as_str(),
            "density" => self.density.as_str(),
            "layout" => self.layout.as_str(),
            "hints" if self.hints => "true",
            "hints" => "false",
            _ => return None,
        })
    }

    fn set_text(&mut self, key: &str, value: &str) {
        let accepted = match key {
            "theme" => ThemeChoice::parse(value).map(|v| self.theme = v),
            "colorMode" => ColorChoice::parse(value).map(|v| self.color = v),
            "glyphs" => GlyphChoice::parse(value).map(|v| self.glyphs = v),
            "density" => Density::parse(value).map(|v| self.density = v),
            "layout" => LayoutChoice::parse(value).map(|v| self.layout = v),
            "hints" => {
                self.hints = value == "true";
                Some(())
            }
            _ => None,
        };
        if accepted.is_some() {
            if let Some(known) = KEYS.iter().find(|known| **known == key) {
                self.set.insert(known);
            }
            self.forget_stored_value(key);
        }
    }

    fn clear_key(&mut self, key: &str) {
        match key {
            "theme" => self.theme = ThemeChoice::default(),
            "colorMode" => self.color = ColorChoice::default(),
            "glyphs" => self.glyphs = GlyphChoice::default(),
            "density" => self.density = Density::default(),
            "layout" => self.layout = LayoutChoice::default(),
            "hints" => self.hints = false,
            _ => return,
        }
        self.set.remove(key);
        self.forget_stored_value(key);
    }

    /// Drop a value kept because it was rejected: the user just decided.
    fn forget_stored_value(&mut self, key: &str) {
        self.preserved.remove(key);
        self.rejected.retain(|rejected| rejected != key);
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
    fn only_presentation_keys_and_their_values_can_be_edited() {
        let mut edits = AppearanceEdits::default();
        for key in [
            "model",
            "provider",
            "permissionMode",
            "permission_mode",
            "permissionRules",
            "apiKey",
            "familiar",
            "",
        ] {
            let error = edits.set(key, "anything").unwrap_err();
            assert!(error.contains("not an appearance setting"), "{error}");
        }
        let error = edits.set("theme", "plaid").unwrap_err();
        assert!(error.contains("auto, dark, light"), "{error}");
        assert!(edits.set("hints", "maybe").is_err());
        assert!(edits.is_empty(), "a refused edit changes nothing");

        // Every key in the list has choices, and each one takes effect.
        for key in EDITABLE_KEYS {
            for value in choices_for(key).expect("listed keys have choices") {
                let mut edits = AppearanceEdits::default();
                edits.set(key, value).expect("a listed value is accepted");
                let mut layer = NextAppearanceSettings::default();
                edits.apply_to(&mut layer);
                if key == REDUCE_MOTION_KEY {
                    assert_eq!(edits.reduce_motion(), Some(*value == "true"));
                } else {
                    assert_eq!(layer.value_text(key), Some(*value), "{key}");
                }
            }
        }
    }

    #[test]
    fn setting_a_key_replaces_a_rejected_value_and_leaves_the_rest() {
        let mut layer: NextAppearanceSettings =
            serde_json::from_str(r#"{"theme":"plaid","density":"compact","laterKey":42}"#).unwrap();
        let mut edits = AppearanceEdits::default();
        edits.set("theme", "dark").unwrap();
        edits.apply_to(&mut layer);
        assert!(layer.rejected_keys().is_empty());
        assert_eq!(
            serde_json::to_value(&layer).unwrap(),
            serde_json::json!({"theme":"dark","density":"compact","laterKey":42})
        );
    }

    #[test]
    fn a_reset_clears_known_keys_and_keeps_unknown_ones() {
        let mut layer: NextAppearanceSettings = serde_json::from_str(
            r#"{"theme":"plaid","density":"compact","hints":true,"layout":"work","laterKey":42}"#,
        )
        .unwrap();
        let mut edits = AppearanceEdits::default();
        edits.reset_all();
        edits.apply_to(&mut layer);
        assert_eq!(edits.reduce_motion(), Some(false));
        assert_eq!(layer.density, Density::Comfortable);
        assert_eq!(layer.layout, LayoutChoice::Focus);
        assert!(!layer.hints);
        assert!(layer.rejected_keys().is_empty());
        assert_eq!(
            serde_json::to_value(&layer).unwrap(),
            serde_json::json!({"laterKey":42})
        );
    }

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
