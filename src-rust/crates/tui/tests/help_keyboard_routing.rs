use claurst_core::{
    config::Config,
    cost::CostTracker,
    keybindings::{KeybindingResolver, UserKeybindings},
};
use claurst_tui::{
    app::App,
    overlays::{render_help_overlay, HelpEntry},
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{backend::TestBackend, Terminal};

fn help(bindings: &str) -> App {
    let mut app = App::new(Config::default(), CostTracker::new());
    app.keybindings = KeybindingResolver::new(&UserKeybindings::from_json_str(bindings));
    app.help_overlay.commands = (0..200)
        .map(|index| HelpEntry {
            name: format!("command-{index}"),
            aliases: String::new(),
            description: "synthetic help description".into(),
            category: "Test".into(),
        })
        .collect();
    app.help_overlay.toggle();
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
    terminal
        .draw(|frame| render_help_overlay(frame, &app.help_overlay, frame.area()))
        .unwrap();
    app
}

#[test]
fn help_page_navigation_uses_rebound_actions_and_explicit_unbinding() {
    let mut app = help(
        r#"{"bindings":[
        {"chord":"pagedown","action":null,"context":"Help"},
        {"chord":"ctrl+n","action":"pageDown","context":"Help"},
        {"chord":"ctrl+p","action":"pageUp","context":"Help"}
    ]}"#,
    );
    app.handle_key_event(KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE));
    assert_eq!(app.help_overlay.scroll_offset, 0);
    app.handle_key_event(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::CONTROL));
    assert!(
        app.help_overlay.scroll_offset > 1,
        "configured page must move by visible rows"
    );
    app.handle_key_event(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL));
    assert_eq!(app.help_overlay.scroll_offset, 0);
    assert!(app.help_overlay.filter.is_empty());
}

#[test]
fn help_chords_do_not_leak_into_filter_and_close_is_remappable() {
    let mut app = help(
        r#"{"bindings":[
        {"chord":"x y","action":"pageDown","context":"Help"},
        {"chord":"ctrl+w","action":"close","context":"Help"}
    ]}"#,
    );
    app.handle_key_event(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE));
    assert!(
        app.help_overlay.filter.is_empty(),
        "pending chord is not filter text"
    );
    app.handle_key_event(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));
    assert!(app.help_overlay.scroll_offset > 1);
    assert!(app.help_overlay.filter.is_empty());
    app.handle_key_event(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE));
    assert_eq!(app.help_overlay.filter, "f");
    app.handle_key_event(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL));
    assert!(!app.help_overlay.visible);
}
