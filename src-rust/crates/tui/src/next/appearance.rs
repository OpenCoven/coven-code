//! `/config appearance` for the Next renderer: preview, save, revert, reset.
//!
//! A change shows on the next frame and stays in memory until it is saved,
//! so trying a value never touches settings.json. Classic never reaches this
//! module: the event loop only routes the command here under Next.

use claurst_core::appearance::{
    choices_for, AppearanceEdits, LayoutChoice, NextAppearanceSettings, EDITABLE_KEYS,
    REDUCE_MOTION_KEY,
};
use claurst_core::config::Settings;
use claurst_core::keybindings::UserKeybindings;
use ratatui::layout::Rect;

use super::{layout, InspectorPage, NextPresentation, ShellState};
use crate::app::App;

const USAGE: &str = "Use /config appearance to list the settings, /config appearance <setting> \
                     <value> to preview one, or add save, revert or reset.";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppearanceCommand {
    Show,
    Set { key: String, value: String },
    Save,
    Revert,
    Reset,
}

/// The arguments after `appearance` when a command addresses these settings.
pub fn appearance_args<'a>(command: &str, args: &'a str) -> Option<&'a str> {
    if !matches!(command, "config" | "settings") {
        return None;
    }
    let rest = args.trim_start().strip_prefix("appearance")?;
    (rest.is_empty() || rest.starts_with(char::is_whitespace)).then(|| rest.trim())
}

/// Read a command, or return the text explaining why it cannot be read.
pub fn parse_appearance_command(args: &str) -> Result<AppearanceCommand, String> {
    let words: Vec<&str> = args.split_whitespace().collect();
    match words.as_slice() {
        [] => Ok(AppearanceCommand::Show),
        ["save"] => Ok(AppearanceCommand::Save),
        ["revert"] => Ok(AppearanceCommand::Revert),
        ["reset"] => Ok(AppearanceCommand::Reset),
        [key, value] => Ok(AppearanceCommand::Set {
            key: (*key).to_string(),
            value: (*value).to_string(),
        }),
        _ => Err(USAGE.to_string()),
    }
}

impl ShellState {
    /// Open the inspector where `choice` asks, and report whether it could.
    /// Work and Inspect wait for room beside the transcript, so a session never
    /// starts with its transcript hidden; the inspector key still opens it at
    /// any size.
    pub fn apply_layout(&mut self, choice: LayoutChoice, terminal: (u16, u16)) -> bool {
        let area = Rect::new(0, 0, terminal.0, terminal.1);
        let page = match choice {
            LayoutChoice::Focus => None,
            _ if layout::SizeClass::of(area) != layout::SizeClass::Split => return false,
            LayoutChoice::Work => Some(InspectorPage::Work),
            LayoutChoice::Inspect => Some(InspectorPage::Activity),
        };
        self.inspector = page;
        self.inspector_scroll = 0;
        true
    }
}

/// `saved` with the session's preview on top.
fn with_preview(saved: &Settings, edits: &AppearanceEdits) -> Settings {
    let mut draft = saved.clone();
    edits.apply_to(&mut draft.next_appearance);
    if let Some(reduce_motion) = edits.reduce_motion() {
        draft.reduce_motion = reduce_motion;
    }
    draft
}

/// Draw what `saved` and the session's preview ask for.
pub fn apply_presentation(
    app: &mut App,
    saved: &Settings,
    keybindings: &UserKeybindings,
    env: impl Fn(&str) -> Option<String>,
) {
    let draft = with_preview(saved, &app.next_shell.appearance_edits);
    app.next_presentation = NextPresentation::from_settings(&draft, keybindings, env);
}

fn key_value(settings: &Settings, key: &str) -> &'static str {
    if key == REDUCE_MOTION_KEY {
        return if settings.reduce_motion {
            "true"
        } else {
            "false"
        };
    }
    settings.next_appearance.value_text(key).unwrap_or_default()
}

fn listing(saved: &Settings, edits: &AppearanceEdits) -> String {
    let draft = with_preview(saved, edits);
    let mut lines =
        vec!["Appearance for Next. These change how it draws and nothing else.".to_string()];
    let mut previewing = false;
    for key in EDITABLE_KEYS {
        let (value, stored) = (key_value(&draft, key), key_value(saved, key));
        let mark = if value == stored {
            ""
        } else {
            previewing = true;
            "*"
        };
        let choices = choices_for(key).unwrap_or_default().join(", ");
        lines.push(format!(
            "  {key:<13}{:<13}{choices}",
            format!("{value}{mark}")
        ));
    }
    if previewing {
        lines.push("* previewing, not saved.".to_string());
    }
    let ignored: Vec<&str> = saved
        .next_appearance
        .rejected_keys()
        .iter()
        .map(String::as_str)
        .filter(|key| !key.is_empty())
        .collect();
    if !ignored.is_empty() {
        lines.push(format!(
            "Ignored in your settings, using the default: {}.",
            ignored.join(", ")
        ));
    }
    lines.push("Preview one: /config appearance density compact".to_string());
    lines.push(
        "/config appearance save keeps a preview, revert drops it, reset previews the defaults."
            .to_string(),
    );
    lines.join("\n")
}

/// Keys the edits name whose value still differs after saving: a layer above
/// the user's file sets them.
fn still_overridden(edits: &AppearanceEdits, saved: &Settings) -> Vec<&'static str> {
    let wanted = with_preview(
        &Settings {
            next_appearance: NextAppearanceSettings::default(),
            reduce_motion: false,
            ..Settings::default()
        },
        edits,
    );
    edits
        .keys()
        .filter(|key| key_value(&wanted, key) != key_value(saved, key))
        .collect()
}

/// Carry out a command and return the reply. `saved` is the settings as now
/// on disk; for [`AppearanceCommand::Save`] the caller writes first and passes
/// any failure in `save_failure`.
pub fn run_appearance_command(
    app: &mut App,
    command: AppearanceCommand,
    saved: &Settings,
    save_failure: Option<String>,
    keybindings: &UserKeybindings,
    env: impl Fn(&str) -> Option<String>,
    terminal: (u16, u16),
) -> String {
    let edits = &app.next_shell.appearance_edits;
    let layout_before = with_preview(saved, edits).next_appearance.layout;
    let mut layout_named = false;
    let reply = match command {
        AppearanceCommand::Show => return listing(saved, edits),
        AppearanceCommand::Set { key, value } => {
            match app.next_shell.appearance_edits.set(&key, &value) {
                Err(reason) => return reason,
                Ok(()) => {
                    layout_named = key == "layout";
                    format!(
                        "Previewing {key} = {value}. Not saved: /config appearance save keeps \
                         it, /config appearance revert drops it."
                    )
                }
            }
        }
        AppearanceCommand::Save if edits.is_empty() => {
            return "Nothing to save: no preview is active.".to_string()
        }
        AppearanceCommand::Save => match save_failure {
            Some(failure) => {
                return format!(
                    "Not saved, and your settings file was left as it was: {failure}. The \
                     preview stays for this session."
                )
            }
            None => {
                let done = std::mem::take(&mut app.next_shell.appearance_edits);
                let overridden = still_overridden(&done, saved);
                if overridden.is_empty() {
                    "Saved.".to_string()
                } else {
                    format!(
                        "Saved, but settings for this folder also set {}, and those win here.",
                        overridden.join(", ")
                    )
                }
            }
        },
        AppearanceCommand::Revert if edits.is_empty() => {
            return "Nothing to revert: no preview is active.".to_string()
        }
        AppearanceCommand::Revert => {
            app.next_shell.appearance_edits = AppearanceEdits::default();
            "Preview dropped. Showing your saved appearance.".to_string()
        }
        AppearanceCommand::Reset => {
            app.next_shell.appearance_edits.reset_all();
            "Previewing the defaults. Not saved: /config appearance save keeps them, and keeps \
             any setting this version does not know. /config appearance revert drops the preview."
                .to_string()
        }
    };
    apply_presentation(app, saved, keybindings, env);
    let layout_after = with_preview(saved, &app.next_shell.appearance_edits)
        .next_appearance
        .layout;
    if (layout_named || layout_after != layout_before)
        && !app.next_shell.apply_layout(layout_after, terminal)
    {
        return format!(
            "{reply} The {} layout opens at 120x32 or larger; this terminal is {}x{}.",
            layout_after.as_str(),
            terminal.0,
            terminal.1
        );
    }
    reply
}
