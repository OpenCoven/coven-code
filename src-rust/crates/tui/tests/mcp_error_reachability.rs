use claurst_tui::mcp_view::{
    render_mcp_view, McpServerView, McpToolView, McpViewPane, McpViewState, McpViewStatus,
};
use ratatui::{backend::TestBackend, Terminal};

fn server(error: &str) -> McpServerView {
    McpServerView {
        name: "privacy-fixture".into(),
        transport: "sse".into(),
        status: McpViewStatus::Error,
        tool_count: 1,
        resource_count: 0,
        prompt_count: 0,
        resources: vec![],
        prompts: vec![],
        error_message: Some(error.into()),
        tools: vec![McpToolView {
            name: "retained-tool".into(),
            server: "privacy-fixture".into(),
            description: "Unchanged tool description".into(),
            input_schema: None,
        }],
    }
}

fn reachable(width: u16, height: u16, diagnostic: &str, required: &[&str]) {
    // This envelope is the actual rmcp initialization error shape observed
    // through the installed CLI. The useful stage/category follows its type.
    let error = format!("rmcp sse client 'privacy-fixture' failed to initialize: Send message error Transport [claurst_mcp::rmcp_backend::LegacySseRmcpTransport] error: MCP server 'privacy-fixture': {diagnostic}, when send initialize request");
    let mut state = McpViewState::new();
    state.open(vec![server(&error)]);
    state.toggle_error_detail();
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    for _ in 0..64 {
        terminal
            .draw(|frame| render_mcp_view(&state, frame.area(), frame.buffer_mut()))
            .unwrap();
        let content: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .flat_map(|cell| cell.symbol().chars())
            .filter(|c| !c.is_whitespace() && !"│┃┌┐└┘─━╭╮╯╰".contains(*c))
            .collect();
        if required.iter().all(|value| content.contains(value)) {
            return;
        }
        // Existing configured Select navigation must reach the diagnostic
        // tail when the explicitly expanded error owns the focused detail.
        state.select_next();
    }
    panic!("diagnostic stage/category unreachable at {width}x{height}: {diagnostic}");
}

#[test]
fn compact_post_connection_is_reachable() {
    reachable(
        80,
        24,
        "legacy SSE POST failed (connection)",
        &["POST", "connection"],
    );
}

#[test]
fn compact_post_redirect_is_reachable() {
    reachable(
        80,
        24,
        "legacy SSE POST failed (redirect)",
        &["POST", "redirect"],
    );
}

#[test]
fn compact_post_body_is_reachable() {
    reachable(
        80,
        24,
        "legacy SSE POST response body failed (decode)",
        &["POSTresponsebody", "decode"],
    );
}

#[test]
fn compact_http_status_is_reachable() {
    reachable(
        80,
        24,
        "HTTP 403 Forbidden from legacy SSE transport",
        &["HTTP403"],
    );
}

#[test]
fn large_terminal_remains_readable() {
    reachable(
        120,
        40,
        "legacy SSE POST response body failed (decode)",
        &["POSTresponsebody", "decode"],
    );
}

#[test]
fn closing_error_retains_tool_selection_and_details() {
    let mut state = McpViewState::new();
    state.open(vec![server("safe error")]);
    state.active_pane = McpViewPane::ToolDetail;
    state.toggle_error_detail();
    state.toggle_error_detail();
    assert!(!state.error_expanded);
    assert_eq!(state.active_pane, McpViewPane::ToolDetail);
    assert_eq!(state.filtered_tools()[0].name, "retained-tool");
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
    terminal
        .draw(|frame| render_mcp_view(&state, frame.area(), frame.buffer_mut()))
        .unwrap();
    let content: String = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .flat_map(|cell| cell.symbol().chars())
        .collect();
    assert!(content.contains("Tool Detail"));
}

fn input_case(case: &str) {
    let home = tempfile::tempdir().unwrap();
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "configured_input_worker",
            "--ignored",
            "--nocapture",
        ])
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("HOME", home.path())
        .env("COVEN_HOME", home.path().join("coven"))
        .env("COVEN_CODE_HOME", home.path().join("code"))
        .env("COVEN_CODE_VOICE", "0")
        .env("MCP_INPUT_CASE", case)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn configured_remapped_navigation_reaches_error() {
    input_case("remap");
}

#[test]
fn configured_unbound_navigation_does_not_scroll() {
    input_case("unbind");
}

#[test]
fn configured_page_navigation_uses_actual_viewport() {
    input_case("page");
}

#[test]
fn configured_chord_preserves_pending_and_rejected_input() {
    input_case("chord");
}

#[test]
fn typing_in_error_focus_does_not_filter_hidden_tools() {
    input_case("filter");
}

#[test]
#[ignore = "isolated environment worker invoked by parent tests"]
fn configured_input_worker() {
    use claurst_core::{
        config::Config,
        keybindings::{KeybindingResolver, UserKeybindings},
        CostTracker,
    };
    use claurst_tui::{application::UiRenderer, App};
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    let case = std::env::var("MCP_INPUT_CASE").unwrap();
    for renderer in [UiRenderer::Classic, UiRenderer::Next] {
        let mut app = App::new(Config::default(), CostTracker::new());
        app.select_renderer(renderer);
        app.mcp_view
            .open(vec![server(&"safe diagnostic line\n".repeat(40))]);
        app.mcp_view.toggle_error_detail();
        let overrides = UserKeybindings::from_json_str(
            r#"{"schema_version":1,"bindings":[{"chord":"ctrl+n","action":"next","context":"Select"},{"chord":"ctrl+p","action":"prev","context":"Select"},{"chord":"down","action":null,"context":"Select"},{"chord":"ctrl+x ctrl+n","action":"next","context":"Select"}]}"#,
        );
        if matches!(case.as_str(), "remap" | "unbind" | "chord") {
            app.keybindings = KeybindingResolver::new(&overrides);
        }
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        terminal
            .draw(|frame| render_mcp_view(&app.mcp_view, frame.area(), frame.buffer_mut()))
            .unwrap();
        let press = |code, modifiers| KeyEvent::new(code, modifiers);
        match case.as_str() {
            "remap" => {
                app.handle_key_event(press(KeyCode::Char('n'), KeyModifiers::CONTROL));
                assert_eq!(app.mcp_view.error_scroll.get(), 1, "{renderer:?}");
                app.handle_key_event(press(KeyCode::Char('p'), KeyModifiers::CONTROL));
                assert_eq!(app.mcp_view.error_scroll.get(), 0, "{renderer:?}");
            }
            "unbind" => {
                app.handle_key_event(press(KeyCode::Down, KeyModifiers::NONE));
                assert_eq!(
                    app.mcp_view.error_scroll.get(),
                    0,
                    "unbound key scrolled {renderer:?}"
                );
            }
            "page" => {
                app.handle_key_event(press(KeyCode::PageDown, KeyModifiers::NONE));
                assert_eq!(
                    app.mcp_view.error_scroll.get(),
                    app.mcp_view.error_viewport_rows.get(),
                    "{renderer:?}"
                );
                app.handle_key_event(press(KeyCode::PageUp, KeyModifiers::NONE));
                assert_eq!(app.mcp_view.error_scroll.get(), 0, "{renderer:?}");
            }
            "chord" => {
                app.handle_key_event(press(KeyCode::Char('x'), KeyModifiers::CONTROL));
                app.handle_key_event(press(KeyCode::Char('e'), KeyModifiers::NONE));
                assert!(
                    app.mcp_view.error_expanded,
                    "rejected chord toggled error {renderer:?}"
                );
                assert!(app.mcp_view.tool_search.is_empty());
                app.handle_key_event(press(KeyCode::Char('x'), KeyModifiers::CONTROL));
                app.handle_key_event(press(KeyCode::Char('n'), KeyModifiers::CONTROL));
                assert_eq!(app.mcp_view.error_scroll.get(), 1, "{renderer:?}");
            }
            "filter" => {
                app.handle_key_event(press(KeyCode::Char('z'), KeyModifiers::NONE));
                assert!(
                    app.mcp_view.tool_search.is_empty(),
                    "error input changed tool filter {renderer:?}"
                );
            }
            _ => panic!("unknown isolated case"),
        }
    }
}

#[test]
fn resizing_clamps_error_scroll_and_keeps_compact_tail_reachable() {
    let mut state = McpViewState::new();
    state.open(vec![server(&"safe diagnostic line\n".repeat(40))]);
    state.toggle_error_detail();
    for (width, height) in [(80, 24), (0, 0), (1, 1), (200, 160)] {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        state.error_scroll.set(usize::MAX);
        terminal
            .draw(|frame| render_mcp_view(&state, frame.area(), frame.buffer_mut()))
            .unwrap();
        if width >= 80 {
            assert!(state.error_scroll.get() <= state.error_scroll_max.get());
        }
        if width == 200 {
            assert_eq!(state.error_scroll.get(), 0);
        }
    }
}

#[test]
fn tall_narrow_error_view_is_bounded() {
    let mut state = McpViewState::new();
    state.open(vec![server("safe diagnostic at a narrow width")]);
    state.toggle_error_detail();
    for width in [0, 1, 2, 3, 4, 8, 20] {
        let mut terminal = Terminal::new(TestBackend::new(width, 24)).unwrap();
        terminal
            .draw(|frame| render_mcp_view(&state, frame.area(), frame.buffer_mut()))
            .unwrap();
    }
}
