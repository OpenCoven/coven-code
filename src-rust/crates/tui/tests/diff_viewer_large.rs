//! A large diff stays responsive: a frame costs what is on screen, not what is
//! in the diff.

use claurst_core::config::Config;
use claurst_core::cost::CostTracker;
use claurst_tui::app::App;
use claurst_tui::application::UiRenderer;
use claurst_tui::diff_viewer::{parse_unified_diff, DiffPane, FileDiffStats};
use ratatui::backend::TestBackend;
use ratatui::Terminal;
use std::time::{Duration, Instant};

/// Far above a frame that formats only what is visible, and far below one that
/// formats every line of [`LARGE_PAIRS`] pairs, even in a debug build on a busy
/// runner.
const FRAME_BUDGET: Duration = Duration::from_secs(1);
const LARGE_PAIRS: usize = 500_000;

fn app_showing(files: Vec<FileDiffStats>, renderer: UiRenderer) -> App {
    let mut app = App::new(Config::default(), CostTracker::new());
    app.has_credentials = true;
    app.renderer = renderer;
    app.diff_viewer.collapsed = vec![false; files.len()];
    app.diff_viewer.files = files;
    app.diff_viewer.visible = true;
    app
}

fn slowest_frame(app: &mut App, frames: usize) -> Duration {
    let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
    let mut slowest = Duration::ZERO;
    for _ in 0..frames {
        let started = Instant::now();
        terminal
            .draw(|frame| match app.renderer {
                UiRenderer::Classic => claurst_tui::render::render_app(frame, app),
                UiRenderer::Next => claurst_tui::next::render_next(frame, app),
            })
            .unwrap();
        slowest = slowest.max(started.elapsed());
        app.diff_viewer.scroll_detail_down();
    }
    slowest
}

fn large_diff() -> String {
    let mut diff = String::from("diff --git a/big.rs b/big.rs\n--- a/big.rs\n+++ b/big.rs\n");
    diff.push_str(&format!("@@ -1,{LARGE_PAIRS} +1,{LARGE_PAIRS} @@\n"));
    for line in 0..LARGE_PAIRS {
        diff.push_str(&format!("-let value = {line};\n+let value = {line} + 1;\n"));
    }
    diff
}

#[test]
fn a_frame_of_a_large_diff_formats_only_what_is_visible() {
    warm_up_highlighter();
    let files = parse_unified_diff(&large_diff());
    for renderer in [UiRenderer::Classic, UiRenderer::Next] {
        let mut app = app_showing(files.clone(), renderer);
        app.diff_viewer.active_pane = DiffPane::Detail;
        let slowest = slowest_frame(&mut app, 3);
        assert!(
            slowest < FRAME_BUDGET,
            "{renderer:?}: slowest frame took {slowest:?}"
        );
    }
}

/// A pair of changed lines with no word in common, the worst case for a word diff.
fn changed_pair(words: usize) -> String {
    let old: String = (0..words).map(|word| format!("old{word} ")).collect();
    let new: String = (0..words).map(|word| format!("new{word} ")).collect();
    format!("-{old}\n+{new}\n")
}

/// The highlighter loads its syntax set once; this keeps that out of the timing.
fn warm_up_highlighter() {
    let warm = "diff --git a/w.rs b/w.rs\n--- a/w.rs\n+++ b/w.rs\n@@ -1 +1 @@\n-a\n+b\n";
    slowest_frame(
        &mut app_showing(parse_unified_diff(warm), UiRenderer::Classic),
        1,
    );
}

#[test]
fn a_frame_of_one_enormous_changed_line_stays_quick() {
    warm_up_highlighter();
    let diff = format!(
        "diff --git a/min.js b/min.js\n--- a/min.js\n+++ b/min.js\n@@ -1 +1 @@\n{}",
        changed_pair(8_000)
    );
    let mut app = app_showing(parse_unified_diff(&diff), UiRenderer::Classic);
    let slowest = slowest_frame(&mut app, 1);
    assert!(slowest < FRAME_BUDGET, "slowest frame took {slowest:?}");
}

#[test]
fn a_screen_full_of_long_changed_lines_stays_quick() {
    warm_up_highlighter();
    let mut diff = String::from("diff --git a/min.js b/min.js\n--- a/min.js\n+++ b/min.js\n");
    diff.push_str("@@ -1,40 +1,40 @@\n");
    for _ in 0..40 {
        diff.push_str(&changed_pair(8_000));
    }
    let mut app = app_showing(parse_unified_diff(&diff), UiRenderer::Classic);
    let slowest = slowest_frame(&mut app, 1);
    assert!(slowest < FRAME_BUDGET, "slowest frame took {slowest:?}");
}
