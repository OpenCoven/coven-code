//! Next presentation primitives: Brand roles, color modes, glyph fallback,
//! reduced motion and presentation-only settings.
//!
//! Golden snapshots live in `tests/golden/next/`. Regenerate after a
//! deliberate presentation change and review every changed file:
//!
//! ```text
//! UPDATE_GOLDENS=1 cargo test -p claurst-tui --test next_primitives
//! ```

use std::fs;
use std::path::{Path, PathBuf};

use claurst_core::appearance::{ColorChoice, NextAppearanceSettings};
use claurst_core::config::{Config, Settings};
use claurst_core::cost::CostTracker;
use claurst_core::keybindings::{UserBinding, UserKeybindings, KEYBINDINGS_SCHEMA_VERSION};
use claurst_core::types::Message;
use claurst_tui::app::App;
use claurst_tui::next::primitives::{self, Mark, Status};
use claurst_tui::next::style::{
    canvas, contrast_ratio, paint, xterm256_rgb, Appearance, ColorMode, Glyphs, Motion, Role,
    Scheme, ALL_ROLES,
};
use claurst_tui::next::{KeyHints, NextPresentation};
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::style::Color;
use ratatui::text::Line;
use ratatui::widgets::Paragraph;
use ratatui::Terminal;

const MODES: [(ColorMode, &str); 4] = [
    (ColorMode::TrueColor, "truecolor"),
    (ColorMode::Ansi256, "256"),
    (ColorMode::Ansi16, "16"),
    (ColorMode::NoColor, "none"),
];

fn appearance(color: ColorMode, glyphs: Glyphs) -> Appearance {
    Appearance {
        color,
        scheme: Scheme::Dark,
        glyphs,
        motion: Motion::Full,
    }
}

/// Every primitive, in one column.
fn gallery(appearance: &Appearance, width: u16) -> Vec<Line<'static>> {
    let mut lines = vec![
        primitives::header(appearance, "preview"),
        primitives::breadcrumbs(appearance, &["coven-code", "session", "tools"]),
        primitives::transcript_label(appearance, "you"),
        primitives::tool_row(appearance, "Bash", Status::Pending, "running", false),
        primitives::tool_row(appearance, "Read", Status::Evidence, "done", true),
        primitives::list_item(appearance, "Resume last session", true),
        primitives::list_item(appearance, "Start fresh", false),
        primitives::status_message(appearance, Status::Verified, "verified", "receipt 7f3a"),
        primitives::inspector_title(appearance, "Diff"),
        primitives::rule(appearance, width),
        primitives::composer(appearance, "draft text"),
    ];
    lines.extend(primitives::confirmation(
        appearance,
        "Run cargo test?",
        &[('y', "Allow once"), ('n', "Deny")],
        1,
        true,
    ));
    lines.extend(primitives::error_recovery(
        appearance,
        "provider unavailable",
        "Retry with /account status",
    ));
    lines
}

fn render(lines: Vec<Line<'static>>, width: u16, height: u16) -> Buffer {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("terminal");
    terminal
        .draw(|frame| frame.render_widget(Paragraph::new(lines), frame.area()))
        .expect("draw");
    terminal.backend().buffer().clone()
}

fn text_of(buffer: &Buffer) -> String {
    let width = usize::from(buffer.area.width);
    buffer
        .content
        .chunks(width)
        .map(|row| {
            let line: String = row.iter().map(|cell| cell.symbol()).collect();
            format!("{}\n", line.trim_end())
        })
        .collect()
}

fn styles_of(buffer: &Buffer) -> String {
    let mut legend: Vec<String> = Vec::new();
    let mut map = String::new();
    let width = usize::from(buffer.area.width);
    for row in buffer.content.chunks(width) {
        for cell in row {
            let key = format!("fg={:?} bg={:?} mod={:?}", cell.fg, cell.bg, cell.modifier);
            let index = legend.iter().position(|k| *k == key).unwrap_or_else(|| {
                legend.push(key);
                legend.len() - 1
            });
            map.push(char::from(
                b"abcdefghijklmnopqrstuvwxyz0123456789"[index.min(35)],
            ));
        }
        map.push('\n');
    }
    map.push_str("--- legend ---\n");
    for (index, key) in legend.iter().enumerate() {
        map.push_str(&format!(
            "{} {key}\n",
            char::from(b"abcdefghijklmnopqrstuvwxyz0123456789"[index.min(35)])
        ));
    }
    map
}

fn golden(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden/next")
        .join(name)
}

fn assert_golden(name: &str, actual: &str) {
    let path = golden(name);
    if std::env::var_os("UPDATE_GOLDENS").is_some() {
        fs::create_dir_all(path.parent().expect("golden dir")).expect("create golden dir");
        fs::write(&path, actual).expect("write golden");
        return;
    }
    let expected = fs::read_to_string(&path).unwrap_or_else(|error| {
        panic!(
            "missing golden {}: {error}\nregenerate with: UPDATE_GOLDENS=1 cargo test -p claurst-tui --test next_primitives",
            path.display()
        )
    });
    assert_eq!(expected, actual, "golden {} drifted", path.display());
}

#[test]
fn primitive_gallery_matches_goldens_across_glyph_sets_sizes_and_color_modes() {
    for (glyphs, glyph_name) in [(Glyphs::Unicode, "unicode"), (Glyphs::Ascii, "ascii")] {
        for (width, height) in [(40u16, 16u16), (80, 16)] {
            let look = appearance(ColorMode::TrueColor, glyphs);
            let buffer = render(gallery(&look, width), width, height);
            assert_golden(
                &format!("gallery_{glyph_name}_{width}x{height}.txt"),
                &text_of(&buffer),
            );
        }
    }
    for (mode, name) in MODES {
        let look = appearance(mode, Glyphs::Unicode);
        let buffer = render(gallery(&look, 80), 80, 16);
        assert_golden(
            &format!("gallery_{name}_80x16.styles.txt"),
            &styles_of(&buffer),
        );
    }
}

#[test]
fn brand_contrast_pairs_meet_their_profile_minimums() {
    for scheme in [Scheme::Dark, Scheme::Light] {
        let bg = canvas(scheme);
        // oc-text 7:1; muted and subtle 4.5:1 on oc-bg.
        for (role, minimum) in [
            (Role::Text, 7.0),
            (Role::TextMuted, 4.5),
            (Role::TextSubtle, 4.5),
        ] {
            let ratio = contrast_ratio(paint(role, scheme).fg, bg);
            assert!(
                ratio >= minimum,
                "{role:?} {scheme:?}: {ratio:.2} < {minimum}"
            );
        }
        // Filled pairs: on-action/action, status text/bg, selection.
        for role in [
            Role::Action,
            Role::Selection,
            Role::Verified,
            Role::Pending,
            Role::Blocked,
            Role::Evidence,
        ] {
            let pair = paint(role, scheme);
            let ratio = contrast_ratio(pair.fg, pair.bg.expect("filled role"));
            assert!(ratio >= 4.5, "{role:?} {scheme:?}: {ratio:.2}");
        }
        // Text-bearing foreground roles drawn on the canvas.
        for role in [
            Role::Focus,
            Role::Link,
            Role::Presence,
            Role::DiffAdded,
            Role::DiffRemoved,
            Role::Sensitive,
        ] {
            let ratio = contrast_ratio(paint(role, scheme).fg, bg);
            assert!(ratio >= 4.5, "{role:?} {scheme:?}: {ratio:.2}");
        }
        // The identity accent marks, it never carries body text: 3:1.
        let identity = contrast_ratio(paint(Role::Identity, scheme).fg, bg);
        assert!(identity >= 3.0, "Identity {scheme:?}: {identity:.2}");
    }
}

#[test]
fn xterm256_approximations_keep_text_roles_legible() {
    for scheme in [Scheme::Dark, Scheme::Light] {
        let look = Appearance {
            color: ColorMode::Ansi256,
            scheme,
            glyphs: Glyphs::Unicode,
            motion: Motion::Full,
        };
        let bg = xterm256_rgb(claurst_tui::next::style::nearest_xterm256(canvas(scheme)));
        for role in [
            Role::Text,
            Role::TextMuted,
            Role::TextSubtle,
            Role::Focus,
            Role::Presence,
        ] {
            let Some(Color::Indexed(index)) = look.style(role).fg else {
                panic!("{role:?} should resolve to an indexed color");
            };
            let ratio = contrast_ratio(xterm256_rgb(index), bg);
            assert!(
                ratio >= 4.5,
                "{role:?} {scheme:?} index {index}: {ratio:.2}"
            );
        }
    }
}

#[test]
fn no_color_mode_emits_no_color_and_keeps_meaning_in_text_and_attributes() {
    let look = appearance(ColorMode::NoColor, Glyphs::Unicode);
    for role in ALL_ROLES {
        let style = look.style(role);
        assert_eq!(style.fg, None, "{role:?}");
        assert_eq!(style.bg, None, "{role:?}");
    }
    let buffer = render(gallery(&look, 80), 80, 16);
    for cell in &buffer.content {
        assert_eq!((cell.fg, cell.bg), (Color::Reset, Color::Reset));
    }
    let text = text_of(&buffer);
    for word in ["running", "done", "verified", "error"] {
        assert!(text.contains(word), "{word} missing:\n{text}");
    }
    // Selection survives without color: marker and reverse video.
    assert!(text.contains("› Resume last session"), "{text}");
}

#[test]
fn ascii_glyphs_never_emit_non_ascii_cells() {
    for (mode, _) in MODES {
        let look = appearance(mode, Glyphs::Ascii);
        let text = text_of(&render(gallery(&look, 80), 80, 16));
        assert!(text.is_ascii(), "{mode:?}:\n{text}");
        assert!(text.contains("> Resume last session"), "{text}");
        assert!(text.contains("[ok] verified"), "{text}");
    }
    assert!(primitives::activity(&appearance(ColorMode::NoColor, Glyphs::Ascii), 3).is_ascii());
}

#[test]
fn status_is_never_conveyed_by_color_alone() {
    for glyphs in [Glyphs::Unicode, Glyphs::Ascii] {
        for status in [
            Status::Verified,
            Status::Pending,
            Status::Blocked,
            Status::Evidence,
        ] {
            let look = appearance(ColorMode::NoColor, glyphs);
            let text: String = primitives::status_label(&look, status, "word")
                .iter()
                .map(|span| span.content.as_ref())
                .collect();
            let mark = match status {
                Status::Verified => Mark::Verified,
                Status::Pending => Mark::Pending,
                Status::Blocked => Mark::Blocked,
                Status::Evidence => Mark::Evidence,
            };
            assert_eq!(text, format!("{} word", mark.glyph(glyphs)));
        }
    }
}

#[test]
fn reduced_motion_activity_is_static_and_full_motion_is_bounded() {
    let mut reduced = appearance(ColorMode::TrueColor, Glyphs::Unicode);
    reduced.motion = Motion::Reduced;
    let first = primitives::activity(&reduced, 0);
    assert!((0..200).all(|frame| primitives::activity(&reduced, frame) == first));

    let full = appearance(ColorMode::TrueColor, Glyphs::Unicode);
    let frames: Vec<&str> = (0..64)
        .map(|frame| primitives::activity(&full, frame))
        .collect();
    let changes = frames.windows(2).filter(|pair| pair[0] != pair[1]).count();
    assert!(
        changes <= 64 / 8,
        "indicator changed {changes} times in 64 frames"
    );
    let distinct: std::collections::HashSet<_> = frames.iter().collect();
    assert!(distinct.len() <= 4);
}

fn env<'a>(vars: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
    move |name| {
        vars.iter()
            .find(|(key, _)| *key == name)
            .map(|(_, value)| value.to_string())
    }
}

#[test]
fn appearance_resolution_follows_settings_then_environment() {
    let auto = NextAppearanceSettings::default();
    let resolve = |vars: &[(&str, &str)]| Appearance::resolve(&auto, false, env(vars));

    assert_eq!(
        resolve(&[("NO_COLOR", "1"), ("COLORTERM", "truecolor")]).color,
        ColorMode::NoColor
    );
    // An empty NO_COLOR does not count as set.
    assert_eq!(
        resolve(&[("NO_COLOR", ""), ("COLORTERM", "truecolor")]).color,
        ColorMode::TrueColor
    );
    assert_eq!(resolve(&[("TERM", "dumb")]).color, ColorMode::NoColor);
    assert_eq!(
        resolve(&[("TERM", "xterm-256color")]).color,
        ColorMode::Ansi256
    );
    assert_eq!(resolve(&[("TERM", "xterm")]).color, ColorMode::Ansi16);
    assert_eq!(resolve(&[("COLORFGBG", "0;15")]).scheme, Scheme::Light);
    assert_eq!(resolve(&[("COLORFGBG", "15;0")]).scheme, Scheme::Dark);
    assert_eq!(resolve(&[("LANG", "en_US.UTF-8")]).glyphs, Glyphs::Unicode);
    assert_eq!(resolve(&[("LANG", "C")]).glyphs, Glyphs::Ascii);
    assert_eq!(resolve(&[]).motion, Motion::Full);
    assert_eq!(
        Appearance::resolve(&auto, true, env(&[])).motion,
        Motion::Reduced
    );

    let explicit: NextAppearanceSettings =
        serde_json::from_str(r#"{"colorMode":"16","glyphs":"unicode","theme":"light"}"#).unwrap();
    assert_eq!(explicit.color, ColorChoice::Ansi16);
    let resolved = Appearance::resolve(&explicit, false, env(&[("NO_COLOR", "1"), ("LANG", "C")]));
    assert_eq!(
        resolved.color,
        ColorMode::Ansi16,
        "explicit app setting beats NO_COLOR"
    );
    assert_eq!(resolved.glyphs, Glyphs::Unicode);
    assert_eq!(resolved.scheme, Scheme::Light);
}

#[test]
fn appearance_settings_live_inside_settings_without_breaking_or_dropping_anything() {
    let parsed: Settings = serde_json::from_str(
        r#"{"reduceMotion":true,"nextAppearance":{"density":"compact","laterKey":42,"theme":"plaid"}}"#,
    )
    .expect("settings parse");
    assert!(parsed.reduce_motion);
    assert_eq!(
        parsed.next_appearance.rejected_keys(),
        ["theme".to_string()]
    );
    let written: serde_json::Value = serde_json::to_value(&parsed).expect("serialize");
    assert_eq!(
        written["nextAppearance"],
        serde_json::json!({"density": "compact", "laterKey": 42, "theme": "plaid"})
    );

    let wrong_type: Settings = serde_json::from_str(r#"{"nextAppearance":"dark","showCwd":false}"#)
        .expect("settings parse");
    assert!(!wrong_type.show_cwd, "other settings still load");

    let untouched: serde_json::Value =
        serde_json::to_value(Settings::default()).expect("serialize defaults");
    assert!(
        untouched.get("nextAppearance").is_none(),
        "Classic users never get the key"
    );
}

fn bindings(entries: &[(&str, Option<&str>, Option<&str>)]) -> UserKeybindings {
    UserKeybindings {
        schema_version: KEYBINDINGS_SCHEMA_VERSION,
        bindings: entries
            .iter()
            .map(|(chord, action, context)| UserBinding {
                chord: chord.to_string(),
                action: action.map(str::to_string),
                context: context.map(str::to_string),
            })
            .collect(),
    }
}

#[test]
fn key_hints_come_from_the_configurable_bindings_and_surface_conflicts() {
    let KeyHints::Hints(defaults) = KeyHints::resolve(&UserKeybindings::default()) else {
        panic!("defaults should produce plain hints");
    };
    let defaults = defaults.join(" ");
    assert!(
        defaults.contains("send") && defaults.contains("commands"),
        "{defaults}"
    );

    let rebound = bindings(&[("ctrl+g", Some("openHelp"), None)]);
    let KeyHints::Hints(text) = KeyHints::resolve(&rebound) else {
        panic!("a new chord is not a conflict");
    };
    let text = text.join(" ");
    assert!(text.contains("help"), "{text}");
    assert_ne!(text, defaults, "hint follows the user's binding");

    assert_eq!(
        KeyHints::resolve(&bindings(&[("enter", Some("newline"), Some("Chat"))])),
        KeyHints::Unreachable(vec!["submit"])
    );
    assert!(matches!(
        KeyHints::resolve(&bindings(&[
            ("enter", Some("newline"), Some("Chat")),
            ("ctrl+s", Some("submit"), Some("Chat")),
        ])),
        KeyHints::Conflict { .. }
    ));
}

fn next_app(presentation: NextPresentation, messages: &[&str]) -> App {
    let mut app = App::new(Config::default(), CostTracker::new());
    app.has_credentials = true;
    app.next_presentation = presentation;
    for text in messages {
        app.push_message(Message::user(text.to_string()));
    }
    app.turn_start = None;
    app
}

fn draw_next(app: &App, width: u16, height: u16) -> Buffer {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("terminal");
    terminal
        .draw(|frame| claurst_tui::next::render_next(frame, app))
        .expect("draw");
    terminal.backend().buffer().clone()
}

#[test]
fn next_renders_with_brand_roles_and_honors_presentation_settings() {
    let truecolor = NextPresentation {
        appearance: appearance(ColorMode::TrueColor, Glyphs::Unicode),
        ..NextPresentation::default()
    };
    let app = next_app(truecolor, &["hello"]);
    let buffer = draw_next(&app, 80, 24);
    // The identity mark uses the Brand anchor.
    assert!(
        buffer
            .content
            .iter()
            .any(|cell| cell.fg == Color::Rgb(0x8E, 0x3D, 0xFF)),
        "Brand identity accent missing"
    );
    assert!(text_of(&buffer).contains("Coven · preview"));

    let plain = NextPresentation {
        appearance: appearance(ColorMode::NoColor, Glyphs::Ascii),
        hints: Some(KeyHints::resolve(&UserKeybindings::default())),
        ..NextPresentation::default()
    };
    let app = next_app(plain, &["hello"]);
    let buffer = draw_next(&app, 80, 24);
    let text = text_of(&buffer);
    assert!(text.is_ascii(), "{text}");
    assert!(text.contains("send"), "hint row missing:\n{text}");
    assert!(buffer
        .content
        .iter()
        .all(|cell| cell.fg == Color::Reset && cell.bg == Color::Reset));
}

#[test]
fn next_wraps_wide_and_combining_text_by_cell_width_without_loss() {
    let samples = [
        "漢字かな交じり文の折り返しを確認する長い行です漢字かな交じり文",
        "e\u{301}e\u{301}e\u{301} combining marks stay attached to their base",
        "emoji 🧪🧪🧪 occupy two cells each and must not overflow the pane",
        "/Users/someone/projects/a/very/long/path/that/does/not/contain/any/spaces/at/all.rs",
    ];
    let presentation = NextPresentation {
        appearance: appearance(ColorMode::NoColor, Glyphs::Unicode),
        ..NextPresentation::default()
    };
    for sample in samples {
        let app = next_app(presentation.clone(), &[sample]);
        for (width, height) in [(40u16, 24u16), (23, 30)] {
            let buffer = draw_next(&app, width, height);
            let rows: Vec<String> = text_of(&buffer).lines().map(str::to_string).collect();
            // Reassemble the wrapped transcript rows between the label and
            // the status row; nothing may be cut.
            let start = rows.iter().position(|row| row == "you").expect("label") + 1;
            let joined: String = rows[start..]
                .iter()
                .take_while(|row| !row.is_empty() && !row.starts_with("ready"))
                .map(String::as_str)
                .collect();
            let compact: String = sample.chars().filter(|c| *c != ' ').collect();
            let rendered: String = joined.chars().filter(|c| *c != ' ').collect();
            assert_eq!(rendered, compact, "{width}x{height}: {sample}");
        }
    }
}
