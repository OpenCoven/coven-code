use claurst_core::{config::Config, cost::CostTracker, types::Message};
use claurst_tui::{
    app::App,
    render::render_app,
    virtual_list::{VirtualItem, VirtualList},
};
use ratatui::{backend::TestBackend, buffer::Buffer, layout::Rect, Terminal};

struct Row(usize);
impl VirtualItem for Row {
    fn measure_height(&self, _: u16) -> u16 {
        1
    }
    fn search_text(&self) -> String {
        format!("row-{}", self.0)
    }
    fn render(&self, area: Rect, buf: &mut Buffer, _: bool) {
        buf.set_string(
            area.x,
            area.y,
            self.search_text(),
            ratatui::style::Style::default(),
        );
    }
}

fn text(buffer: &Buffer) -> String {
    buffer.content.iter().map(|cell| cell.symbol()).collect()
}

#[test]
fn virtual_rows_do_not_wrap_at_u16_boundary() {
    for count in [65_534, 65_535, 65_536, 100_000] {
        let mut list = VirtualList::new();
        list.set_items((0..count).map(Row).collect());
        assert_eq!(list.total_height(80), count);
        let area = Rect::new(0, 0, 80, 24);
        let mut buffer = Buffer::empty(area);
        list.render(area, &mut buffer);
        assert!(
            text(&buffer).contains(&format!("row-{}", count - 1)),
            "tail for {count}"
        );
        list.scroll_to_index(65_530, 80);
        list.render(area, &mut buffer);
        assert!(text(&buffer).contains("row-65530"));
        assert_eq!(list.items.len(), count);
    }
}

#[test]
fn classic_live_tail_and_full_scroll_position_survive_large_history() {
    for (width, height) in [(80, 24), (120, 40)] {
        let mut app = App::new(Config::default(), CostTracker::new());
        let history = (0..100_000)
            .map(|i| format!("history-line-{i:06}\n"))
            .collect::<String>();
        app.replace_messages(vec![
            Message::user("synthetic history"),
            Message::assistant(history),
        ]);
        app.is_streaming = true;
        app.streaming_text = "LIVE-TAIL-441".into();
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| render_app(frame, &app)).unwrap();
        assert!(
            text(terminal.backend().buffer()).contains("LIVE-TAIL-441"),
            "live tail at {width}x{height}"
        );
        assert!(app.last_render_scroll_offset.get() > 65_535);
        app.auto_scroll = false;
        app.scroll_offset = 80_000;
        terminal.draw(|frame| render_app(frame, &app)).unwrap();
        let scrolled = text(terminal.backend().buffer());
        assert!(!scrolled.contains("LIVE-TAIL-441"));
        assert!(
            scrolled.contains("history-line-02"),
            "old history remains reachable at {width}x{height}"
        );
        assert_eq!(app.messages[1].get_all_text().lines().count(), 100_000);
    }
}

#[test]
fn resumed_tail_remains_visible_with_startup_diagnostics_at_compact_size() {
    let mut app = App::new(Config::default(), CostTracker::new());
    app.replace_messages(vec![
        Message::user("synthetic resumed question"),
        Message::assistant("Earlier answer\nHISTORY-TAIL-441"),
    ]);
    app.push_system_message(
        "Diagnostics: synthetic local path".into(),
        claurst_tui::app::SystemMessageStyle::Info,
    );
    for (width, height) in [(80, 24), (120, 40)] {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| render_app(frame, &app)).unwrap();
        assert!(
            text(terminal.backend().buffer()).contains("HISTORY-TAIL-441"),
            "resumed tail at {width}x{height}"
        );
    }
    app.replace_messages(Vec::new());
    app.system_annotations.clear();
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
    terminal.draw(|frame| render_app(frame, &app)).unwrap();
    assert!(
        text(terminal.backend().buffer()).contains("Coven v"),
        "empty sessions keep their welcome screen"
    );
}
