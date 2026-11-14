//! Active frame measurements, separate from the raw-PTY paint-gap proxy.
//! Run serially with --release --ignored --nocapture. JSON samples include
//! actual allocations inside draw; mutation and initial cache builds are timed
//! separately. No wall-clock ratio is used as a flaky correctness assertion.
use claurst_core::{
    config::Config,
    cost::CostTracker,
    types::{Message, MessageContent},
};
use claurst_tui::{
    app::{App, ToolStatus, ToolUseBlock},
    application::UiRenderer,
};
use ratatui::{backend::TestBackend, Terminal};
use std::{
    alloc::{GlobalAlloc, Layout, System},
    sync::atomic::{AtomicUsize, Ordering},
    time::Instant,
};

struct Counted;
static ALLOCATIONS: AtomicUsize = AtomicUsize::new(0);
static BYTES: AtomicUsize = AtomicUsize::new(0);
// SAFETY: All requests and deallocations are forwarded unchanged to System;
// atomics record counts without allocating or accessing allocation contents.
unsafe impl GlobalAlloc for Counted {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        BYTES.fetch_add(layout.size(), Ordering::Relaxed);
        // SAFETY: Forward the caller's valid allocation layout unchanged.
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: This pointer and layout came from the forwarded System allocation.
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        BYTES.fetch_add(size, Ordering::Relaxed);
        // SAFETY: Forward the live allocation, original layout and requested size.
        unsafe { System.realloc(ptr, layout, size) }
    }
}
#[global_allocator]
static ALLOCATOR: Counted = Counted;

fn app(lines: usize, tools: bool) -> App {
    let mut app = App::new(
        Config {
            project_dir: Some("/synthetic/perf".into()),
            ..Config::default()
        },
        CostTracker::new(),
    );
    app.has_credentials = true;
    app.renderer = UiRenderer::Next;
    app.turn_start = None;
    for start in (0..lines).step_by(100) {
        app.messages.push(Message::user("synthetic question"));
        app.messages.push(Message::assistant(
            (start..(start + 100).min(lines))
                .map(|n| format!("retained line {n:06}\n"))
                .collect::<String>(),
        ));
        if tools {
            app.tool_use_blocks.push(ToolUseBlock {
                id: start.to_string(),
                name: "Read".into(),
                turn_index: None,
                status: ToolStatus::Done,
                output_preview: Some("safe output".repeat(10)),
                input_json: "{\"path\":\"/synthetic/file\"}".into(),
            });
        }
    }
    // Fill the viewport with the same kind of tool rows at every history size;
    // comparing a mixed text/tool viewport with an all-tool viewport counts
    // different visible content rather than history traversal.
    if tools {
        while app.tool_use_blocks.len() < 40 {
            let tool = app.tool_use_blocks[0].clone();
            app.tool_use_blocks.push(tool);
        }
    }
    app
}
fn draw(app: &App, terminal: &mut Terminal<TestBackend>) -> serde_json::Value {
    let allocations = ALLOCATIONS.load(Ordering::Relaxed);
    let bytes = BYTES.load(Ordering::Relaxed);
    let start = Instant::now();
    terminal
        .draw(|frame| claurst_tui::next::render_next(frame, app))
        .unwrap();
    let ns = start.elapsed().as_nanos();
    let count = ALLOCATIONS.load(Ordering::Relaxed) - allocations;
    let bytes = BYTES.load(Ordering::Relaxed) - bytes;
    serde_json::json!({"ns": ns, "allocations": count, "allocated_bytes": bytes})
}

#[test]
#[ignore = "optimized serial active-frame qualification; emits raw JSON samples"]
fn measure_active_frames() {
    for lines in [1_000, 10_000, 100_000] {
        for tools in [false, true] {
            for (width, height) in [(80, 24), (120, 40)] {
                let mut app = app(lines, tools);
                let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
                let cold = draw(&app, &mut terminal);
                let idle: Vec<_> = (0..31).map(|_| draw(&app, &mut terminal)).collect();
                let mut streaming = Vec::new();
                app.is_streaming = true;
                // Include a long live prefix, then measure appends + active frames.
                app.streaming_text.push_str(
                    &(0..lines)
                        .map(|n| format!("live line {n:06}\n"))
                        .collect::<String>(),
                );
                let live_build = draw(&app, &mut terminal);
                for _ in 0..31 {
                    app.streaming_text.push_str("new chunk\npartial");
                    app.prompt_input.text.push('q');
                    app.prompt_input.cursor = app.prompt_input.text.len();
                    streaming.push(draw(&app, &mut terminal));
                    app.streaming_text.push('\n');
                }
                // Destructive mutations must retain exactly the fresh semantics.
                if let MessageContent::Text(text) = &mut app.messages[1].content {
                    text.replace_range(..8, "REPLACED");
                }
                let replacement = draw(&app, &mut terminal);
                let keep = app.messages.len() / 2;
                app.messages.truncate(keep);
                app.streaming_text.clear();
                app.streaming_text.push_str("retry\nunfinished");
                let retry = draw(&app, &mut terminal);
                terminal.backend_mut().resize(width - 10, height);
                terminal
                    .resize(ratatui::layout::Rect::new(0, 0, width - 10, height))
                    .unwrap();
                let resize = draw(&app, &mut terminal);
                app.next_presentation.density = claurst_core::appearance::Density::Compact;
                let appearance = draw(&app, &mut terminal);
                app.auto_scroll = false;
                app.next_shell.apply_action("jumpToStart");
                let jump = draw(&app, &mut terminal);
                let text = terminal
                    .backend()
                    .buffer()
                    .content
                    .iter()
                    .map(|cell| cell.symbol())
                    .collect::<String>();
                assert!(text.contains("synthetic question"));
                eprintln!(
                    "PERF_JSON {}",
                    serde_json::json!({"lines":lines,"tools":tools,"width":width,"height":height,"cold":cold,"idle":idle,"live_build":live_build,"streaming":streaming,"replacement":replacement,"retry":retry,"resize":resize,"appearance":appearance,"jump":jump})
                );
            }
        }
    }
}

#[test]
#[ignore = "serial allocation regression, independent of wall-clock noise"]
fn unchanged_frame_allocations_are_independent_of_history() {
    let mut counts = Vec::new();
    for lines in [1_000, 100_000] {
        let app = app(lines, true);
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        draw(&app, &mut terminal);
        let sample = draw(&app, &mut terminal);
        counts.push(sample["allocations"].as_u64().unwrap());
    }
    assert!(
        counts[1] <= counts[0] + 20,
        "unchanged frame walked history: {counts:?}"
    );
}

#[test]
#[ignore = "serial allocation regression with Activity inspector open"]
fn unchanged_activity_frame_does_not_rebuild_hidden_tools() {
    let mut counts = Vec::new();
    for lines in [1_000, 100_000] {
        let mut app = app(lines, true);
        app.next_shell.inspector = Some(claurst_tui::next::InspectorPage::Activity);
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        draw(&app, &mut terminal);
        counts.push(draw(&app, &mut terminal)["allocations"].as_u64().unwrap());
    }
    assert!(
        counts[1] <= counts[0] + 20,
        "Activity frame rebuilt hidden tools: {counts:?}"
    );
}

#[test]
#[ignore = "serial allocation regression for newline-free streaming"]
fn newline_free_append_does_not_rebuild_completed_visual_rows() {
    for source in ["data ", "漢字 \u{1f469}\u{200d}\u{1f4bb} "] {
        let mut counts = Vec::new();
        for lines in [1_000, 100_000] {
            let mut app = app(1, false);
            app.streaming_text.push_str(&source.repeat(lines));
            let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
            draw(&app, &mut terminal);
            app.streaming_text.push_str("tail");
            counts.push(draw(&app, &mut terminal)["allocations"].as_u64().unwrap());
        }
        assert!(
            counts[1] <= counts[0] + 20,
            "append rebuilt completed visual rows: {counts:?}"
        );
    }
}
