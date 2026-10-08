//! Classic TUI full-frame baselines.
//!
//! Renders the main screen through Ratatui's `TestBackend` at 40x12, 80x24
//! and 120x40 in four states — idle, streaming, permission prompt, error
//! modal — and compares the text of every cell against a checked-in golden.
//! These are the "before" pictures for the theme-token work: any layout or
//! glyph change in Classic shows up here as a diff a reviewer can read.
//!
//! Determinism: the whole test binary runs against a scratch `HOME` /
//! `COVEN_HOME` / `COVEN_CODE_HOME` (no user keybindings, no familiar
//! roster, no daemon socket, no tip history), a fixed `USER`, a working
//! directory under the scratch home (so the welcome panel prints `~/project`),
//! a pinned model id, and `frame_count = 0`. The only release-coupled text —
//! the version banner and the "What's new" lines — is masked to fixed-width
//! placeholders so a version bump does not invalidate the baselines.
//!
//! Regenerate after a deliberate presentation change and review every
//! changed frame in the PR:
//!
//! ```text
//! UPDATE_GOLDENS=1 cargo test -p claurst-tui --test classic_baselines
//! ```

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use claurst_core::config::Config;
use claurst_core::constants::{APP_VERSION, WHATS_NEW};
use claurst_core::cost::CostTracker;
use claurst_core::types::Message;
use claurst_tui::app::App;
use claurst_tui::dialogs::PermissionRequest;
use claurst_tui::notifications::NotificationKind;
use claurst_tui::render::render_app;
use ratatui::backend::TestBackend;
use ratatui::Terminal;

const SIZES: &[(u16, u16)] = &[(40, 12), (80, 24), (120, 40)];
const PINNED_MODEL: &str = "claude-opus-4-8";

/// Scratch environment shared by every test in this binary. Initialised once,
/// before any `App` is built; the `TempDir` lives for the whole process.
static SCRATCH: OnceLock<tempfile::TempDir> = OnceLock::new();

fn scratch_env() {
    SCRATCH.get_or_init(|| {
        let scratch = tempfile::Builder::new()
            .prefix("coven-tui-baseline-")
            .tempdir()
            .expect("create scratch dir");
        let home = scratch.path().join("home");
        let coven_home = scratch.path().join("coven");
        let project = home.join("project");
        for dir in [&home, &coven_home, &project] {
            fs::create_dir_all(dir).expect("create scratch subdir");
        }
        // The welcome panel abbreviates the working directory against `HOME`
        // by string prefix, and `current_dir()` returns the canonical path
        // (on macOS a temp dir lives under a `/var` → `/private/var` symlink),
        // so both must be canonical for the label to read `~/project`.
        let home = home.canonicalize().expect("canonical scratch home");
        let project = project.canonicalize().expect("canonical scratch project");
        for var in [
            "ANTHROPIC_API_KEY",
            "ANTHROPIC_CONFIG_DIR",
            "COVEN_CODE_CLAUDE_BIN",
            "COVEN_CODE_TEST_HOME",
            "COVEN_PARENT",
            "COVEN_DAEMON_SOCKET",
            "COVENCAVE",
        ] {
            std::env::remove_var(var);
        }
        std::env::set_var("HOME", &home);
        std::env::set_var("USER", "baseline");
        std::env::set_var("USERNAME", "baseline");
        std::env::set_var("COVEN_HOME", &coven_home);
        std::env::set_var("COVEN_CODE_HOME", coven_home.join("code"));
        std::env::set_current_dir(&project).expect("enter scratch project");
        scratch
    });
}

fn fixture_app() -> App {
    scratch_env();
    let config = Config {
        model: Some(PINNED_MODEL.to_string()),
        provider: Some("anthropic".to_string()),
        familiar: None,
        ..Config::default()
    };
    let mut app = App::new(config, CostTracker::new());
    app.has_credentials = true;
    app.frame_count = 0;
    app
}

fn idle() -> App {
    fixture_app()
}

fn streaming() -> App {
    let mut app = fixture_app();
    app.push_message(Message::user(
        "Summarize the engine contract in one sentence.".to_string(),
    ));
    // `push_message` starts the turn clock; clear it so the status row does
    // not carry a wall-clock elapsed value.
    app.turn_start = None;
    app.is_streaming = true;
    app.streaming_text =
        "The engine exposes a small versioned CLI surface that coven drives over".to_string();
    app
}

fn permission_prompt() -> App {
    let mut app = fixture_app();
    app.push_message(Message::user("Run the test suite.".to_string()));
    app.turn_start = None;
    app.is_streaming = true;
    app.permission_request = Some(PermissionRequest::bash(
        "toolu-baseline-1".to_string(),
        "Bash".to_string(),
        "Runs a shell command in the project".to_string(),
        "cargo test --workspace".to_string(),
        Some("cargo test".to_string()),
    ));
    app
}

fn error_modal() -> App {
    let mut app = fixture_app();
    app.push_notification(
        NotificationKind::Error,
        "Baseline fixture: provider returned HTTP 500 (synthetic error for the golden)".to_string(),
        None,
    );
    app
}

fn render_text(app: &App, width: u16, height: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("test terminal");
    terminal
        .draw(|frame| render_app(frame, app))
        .expect("draw frame");
    let buffer = terminal.backend().buffer();
    let width = usize::from(buffer.area.width);
    let mut text = String::new();
    for row in buffer.content.chunks(width) {
        let line: String = row.iter().map(|cell| cell.symbol()).collect();
        text.push_str(line.trim_end());
        text.push('\n');
    }
    mask_release_text(&text)
}

/// Replace `v<APP_VERSION>` and each "What's new" line with placeholders of
/// the same character width, so a release does not move the baseline.
fn mask_release_text(text: &str) -> String {
    let version = format!("v{APP_VERSION}");
    let version_mask = format!("{:<width$}", "vX.Y.Z", width = version.chars().count());
    text.lines()
        .map(|line| {
            let mut line = line.replace(&version, &version_mask);
            for (index, item) in WHATS_NEW.iter().enumerate() {
                line = mask_prefix_of(&line, item, &format!("<whats-new-{index}>"));
            }
            line
        })
        .map(|line| format!("{}\n", line.trim_end()))
        .collect()
}

/// If `line` renders `item` — either in full or as a prefix cut with `…` —
/// replace that span with `label`, padded or cut to the same width. A bare
/// prefix without the ellipsis is left alone, so ordinary text that happens
/// to share an opening word with a release note is never masked.
fn mask_prefix_of(line: &str, item: &str, label: &str) -> String {
    let item: Vec<char> = item.chars().collect();
    let line: Vec<char> = line.chars().collect();
    let probe = &item[..item.len().min(8)];
    if probe.len() < 8 {
        return line.iter().collect();
    }
    let Some(start) = line.windows(probe.len()).position(|w| w == probe) else {
        return line.iter().collect();
    };
    let mut end = start;
    while end < line.len() && end - start < item.len() && line[end] == item[end - start] {
        end += 1;
    }
    let full_item = end - start == item.len();
    let truncated = end < line.len() && line[end] == '\u{2026}';
    if !full_item && !truncated {
        return line.iter().collect();
    }
    if truncated {
        end += 1;
    }
    let width = end - start;
    let mut mask: String = label.chars().take(width).collect();
    while mask.chars().count() < width {
        mask.push(' ');
    }
    let mut out: String = line[..start].iter().collect();
    out.push_str(&mask);
    out.extend(&line[end..]);
    out
}

fn golden_path(state: &str, width: u16, height: u16) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden/classic")
        .join(format!("{state}_{width}x{height}.txt"))
}

fn assert_golden(path: &Path, actual: &str) {
    if std::env::var_os("UPDATE_GOLDENS").is_some() {
        fs::write(path, actual).expect("write golden");
        return;
    }
    let expected = fs::read_to_string(path).unwrap_or_else(|e| {
        panic!(
            "missing golden {}: {e}\nregenerate with: UPDATE_GOLDENS=1 cargo test -p claurst-tui --test classic_baselines",
            path.display()
        )
    });
    if expected != actual {
        let mut report = String::new();
        for (n, (want, got)) in expected.lines().zip(actual.lines()).enumerate() {
            if want != got {
                report.push_str(&format!("row {}:\n  want: {want}\n  got:  {got}\n", n + 1));
            }
        }
        panic!(
            "Classic baseline drifted from {} (expected {} rows, got {}). \
             If the change is intentional, regenerate with UPDATE_GOLDENS=1 and review the frame diff.\n{report}--- actual frame ---\n{actual}",
            path.display(),
            expected.lines().count(),
            actual.lines().count(),
        );
    }
}

fn check_state(state: &str, build: fn() -> App) {
    for &(width, height) in SIZES {
        let app = build();
        let text = render_text(&app, width, height);
        assert_golden(&golden_path(state, width, height), &text);
    }
}

#[test]
fn classic_idle_matches_baseline() {
    check_state("idle", idle);
}

#[test]
fn classic_streaming_matches_baseline() {
    check_state("streaming", streaming);
}

#[test]
fn classic_permission_prompt_matches_baseline() {
    check_state("permission", permission_prompt);
}

#[test]
fn classic_error_modal_matches_baseline() {
    check_state("error", error_modal);
}
