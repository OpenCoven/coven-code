//! Terminal backend that keeps an unchanged frame off the wire.
//!
//! `Terminal::draw` writes on every call, even with nothing to change: the
//! Crossterm backend ends each frame with colour resets, and the cursor is
//! hidden or moved again. Next redraws on every event-loop tick, so an idle
//! session wrote about 58 times a second (#417). In quiet mode this backend
//! drops those writes. Classic leaves quiet mode off and its bytes pass
//! through unchanged.
//!
//! Those writes also repaired the terminal when something else wrote to it:
//! child processes share the terminal, and an MCP server's stderr can move or
//! hide the cursor or leave a colour active. So quiet mode restates the cursor
//! at a low rate, and resets colours before painting changed cells.

use ratatui::backend::{Backend, ClearType, WindowSize};
use ratatui::buffer::Cell;
use ratatui::layout::{Position, Size};
use std::io::{self, Write};
use std::time::{Duration, Instant};

/// How long a cursor changed behind this backend can stay wrong.
const STATE_REFRESH: Duration = Duration::from_secs(5);
/// Select Graphic Rendition reset: the colours a backend draw assumes.
const RESET_STYLE: &[u8] = b"\x1b[0m";

/// Wraps a backend, optionally dropping writes that would change nothing.
#[derive(Debug)]
pub struct QuietBackend<B> {
    inner: B,
    quiet: bool,
    /// What the terminal's cursor is known to be doing; `None` when unknown.
    cursor_visible: Option<bool>,
    cursor_position: Option<Position>,
    state_refresh: Duration,
    state_refreshed: Instant,
}

impl<B> QuietBackend<B> {
    /// With `quiet` off, every call passes straight through.
    pub fn new(inner: B, quiet: bool) -> Self {
        Self::with_state_refresh(inner, quiet, STATE_REFRESH)
    }

    /// As [`Self::new`], restating the cursor at the given interval.
    pub fn with_state_refresh(inner: B, quiet: bool, every: Duration) -> Self {
        Self {
            inner,
            quiet,
            cursor_visible: None,
            cursor_position: None,
            state_refresh: every,
            state_refreshed: Instant::now(),
        }
    }

    /// Stop assuming the terminal's cursor state, so the next frame states it.
    pub fn forget_terminal_state(&mut self) {
        self.cursor_visible = None;
        self.cursor_position = None;
    }
}

impl<B: Backend + Write> Backend for QuietBackend<B> {
    fn draw<'a, I>(&mut self, content: I) -> io::Result<()>
    where
        I: Iterator<Item = (u16, u16, &'a Cell)>,
    {
        if !self.quiet {
            return self.inner.draw(content);
        }
        // `Terminal` handles the cursor after this call, so a refresh here
        // restates it in the same frame.
        if self.state_refreshed.elapsed() >= self.state_refresh {
            self.state_refreshed = Instant::now();
            self.forget_terminal_state();
        }
        let mut content = content.peekable();
        if content.peek().is_none() {
            return Ok(());
        }
        // Writing cells moves the terminal's cursor.
        self.cursor_position = None;
        self.inner.write_all(RESET_STYLE)?;
        self.inner.draw(content)
    }

    fn append_lines(&mut self, n: u16) -> io::Result<()> {
        self.forget_terminal_state();
        self.inner.append_lines(n)
    }

    fn hide_cursor(&mut self) -> io::Result<()> {
        if self.quiet && self.cursor_visible == Some(false) {
            return Ok(());
        }
        self.inner.hide_cursor()?;
        self.cursor_visible = Some(false);
        Ok(())
    }

    fn show_cursor(&mut self) -> io::Result<()> {
        if self.quiet && self.cursor_visible == Some(true) {
            return Ok(());
        }
        self.inner.show_cursor()?;
        self.cursor_visible = Some(true);
        Ok(())
    }

    fn get_cursor_position(&mut self) -> io::Result<Position> {
        self.inner.get_cursor_position()
    }

    fn set_cursor_position<P: Into<Position>>(&mut self, position: P) -> io::Result<()> {
        let position = position.into();
        if self.quiet && self.cursor_position == Some(position) {
            return Ok(());
        }
        self.inner.set_cursor_position(position)?;
        self.cursor_position = Some(position);
        Ok(())
    }

    fn clear(&mut self) -> io::Result<()> {
        self.forget_terminal_state();
        self.inner.clear()
    }

    fn clear_region(&mut self, clear_type: ClearType) -> io::Result<()> {
        self.forget_terminal_state();
        self.inner.clear_region(clear_type)
    }

    fn size(&self) -> io::Result<Size> {
        self.inner.size()
    }

    fn window_size(&mut self) -> io::Result<WindowSize> {
        self.inner.window_size()
    }

    fn flush(&mut self) -> io::Result<()> {
        Backend::flush(&mut self.inner)
    }
}

impl<B: Write> Write for QuietBackend<B> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        // Raw bytes can do anything to the terminal.
        self.forget_terminal_state();
        self.inner.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        Write::flush(&mut self.inner)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::CrosstermBackend;
    use ratatui::layout::Rect;
    use ratatui::widgets::Paragraph;
    use ratatui::{Terminal, TerminalOptions, Viewport};
    use std::cell::RefCell;
    use std::rc::Rc;

    const AREA: Rect = Rect::new(0, 0, 20, 4);
    const HIDE: &[u8] = b"\x1b[?25l";

    #[derive(Clone, Default)]
    struct Wire(Rc<RefCell<Vec<u8>>>);

    impl Wire {
        fn take(&self) -> Vec<u8> {
            std::mem::take(&mut self.0.borrow_mut())
        }
    }

    impl Write for Wire {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0.borrow_mut().extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    type Wrapped = QuietBackend<CrosstermBackend<Wire>>;

    fn terminal(quiet: bool) -> (Terminal<Wrapped>, Wire) {
        // Far enough away that a stalled test run cannot reach it.
        terminal_refreshing(quiet, Duration::from_secs(3600))
    }

    fn terminal_refreshing(quiet: bool, every: Duration) -> (Terminal<Wrapped>, Wire) {
        let wire = Wire::default();
        let inner = CrosstermBackend::new(wire.clone());
        let backend = QuietBackend::with_state_refresh(inner, quiet, every);
        let options = TerminalOptions {
            viewport: Viewport::Fixed(AREA),
        };
        (Terminal::with_options(backend, options).unwrap(), wire)
    }

    fn draw<B: Backend>(terminal: &mut Terminal<B>, text: &str, cursor: Option<(u16, u16)>) {
        terminal
            .draw(|frame| {
                frame.render_widget(Paragraph::new(text), frame.area());
                if let Some(position) = cursor {
                    frame.set_cursor_position(position);
                }
            })
            .unwrap();
    }

    fn contains(haystack: &[u8], needle: &[u8]) -> bool {
        haystack.windows(needle.len()).any(|w| w == needle)
    }

    #[test]
    fn an_unchanged_frame_writes_nothing() {
        for cursor in [None, Some((2, 1))] {
            let (mut terminal, wire) = terminal(true);
            draw(&mut terminal, "hello", cursor);
            assert!(!wire.take().is_empty(), "the first frame is written");
            for _ in 0..3 {
                draw(&mut terminal, "hello", cursor);
                assert_eq!(wire.take(), Vec::<u8>::new(), "cursor {cursor:?}");
            }
        }
    }

    #[test]
    fn a_changed_frame_is_written_and_the_cursor_put_back() {
        // Writing cells moves the terminal's cursor, so a position that did
        // not change must still be restated after the cells.
        let (mut terminal, wire) = terminal(true);
        draw(&mut terminal, "hello", Some((2, 1)));
        wire.take();
        draw(&mut terminal, "help!", Some((2, 1)));
        let bytes = wire.take();
        assert!(contains(&bytes, b"p!"), "{bytes:?}");
        assert!(contains(&bytes, b"\x1b[2;3H"), "{bytes:?}");
    }

    #[test]
    fn moving_or_toggling_the_cursor_alone_is_written() {
        let (mut terminal, wire) = terminal(true);
        draw(&mut terminal, "hello", Some((2, 1)));
        wire.take();
        draw(&mut terminal, "hello", Some((3, 1)));
        assert!(contains(&wire.take(), b"\x1b[2;4H"));
        draw(&mut terminal, "hello", None);
        assert!(contains(&wire.take(), HIDE));
        draw(&mut terminal, "hello", Some((3, 1)));
        assert!(contains(&wire.take(), b"\x1b[?25h"));
    }

    #[test]
    fn clearing_makes_the_next_frame_state_the_cursor_again() {
        let (mut terminal, wire) = terminal(true);
        draw(&mut terminal, "hello", None);
        terminal.clear().unwrap();
        wire.take();
        draw(&mut terminal, "hello", None);
        assert!(contains(&wire.take(), HIDE));
    }

    #[test]
    fn raw_writes_and_forgetting_make_the_next_frame_state_the_cursor_again() {
        let (mut terminal, wire) = terminal(true);
        draw(&mut terminal, "hello", None);
        wire.take();
        terminal.backend_mut().write_all(b"title").unwrap();
        wire.take();
        draw(&mut terminal, "hello", None);
        assert!(contains(&wire.take(), HIDE), "after a raw write");
        draw(&mut terminal, "hello", None);
        assert_eq!(wire.take(), Vec::<u8>::new());
        terminal.backend_mut().forget_terminal_state();
        draw(&mut terminal, "hello", None);
        assert!(contains(&wire.take(), HIDE), "after forgetting");
    }

    #[test]
    fn a_cursor_changed_behind_the_backend_is_restated_after_the_interval() {
        // A child process sharing the terminal, such as an MCP server logging
        // to stderr, moves or hides the cursor without this backend seeing it.
        for (cursor, expected) in [
            (Some((2, 1)), &b"\x1b[?25h"[..]),
            (Some((2, 1)), &b"\x1b[2;3H"[..]),
            (None, HIDE),
        ] {
            let (mut terminal, wire) = terminal_refreshing(true, Duration::ZERO);
            draw(&mut terminal, "hello", cursor);
            wire.take();
            draw(&mut terminal, "hello", cursor);
            let bytes = wire.take();
            assert!(contains(&bytes, expected), "{cursor:?}: {bytes:?}");
        }
    }

    #[test]
    fn restating_the_cursor_paints_no_cells() {
        let (mut terminal, wire) = terminal_refreshing(true, Duration::ZERO);
        draw(&mut terminal, "hello", None);
        wire.take();
        draw(&mut terminal, "hello", None);
        assert_eq!(wire.take(), HIDE.to_vec());
    }

    #[test]
    fn a_changed_frame_resets_colours_before_painting() {
        // The backend assumes default colours when a draw starts, so a colour
        // a child process left active would paint into default-coloured cells.
        let (mut terminal, wire) = terminal(true);
        draw(&mut terminal, "hello", None);
        wire.take();
        draw(&mut terminal, "help!", None);
        let bytes = wire.take();
        assert!(bytes.starts_with(b"\x1b[0m"), "{bytes:?}");
    }

    #[test]
    fn with_quiet_off_every_byte_matches_the_bare_backend() {
        // Classic's output must not change.
        let bare_wire = Wire::default();
        let mut bare = Terminal::with_options(
            CrosstermBackend::new(bare_wire.clone()),
            TerminalOptions {
                viewport: Viewport::Fixed(AREA),
            },
        )
        .unwrap();
        let (mut wrapped, wrapped_wire) = terminal(false);
        let frames = [
            ("hello", None),
            ("hello", None),
            ("hello", Some((2, 1))),
            ("hello", Some((2, 1))),
            ("help!", Some((2, 1))),
            ("help!", None),
        ];
        for (step, (text, cursor)) in frames.into_iter().enumerate() {
            draw(&mut bare, text, cursor);
            draw(&mut wrapped, text, cursor);
            let expected = bare_wire.take();
            assert!(
                !expected.is_empty(),
                "step {step} writes on the bare backend"
            );
            assert_eq!(wrapped_wire.take(), expected, "step {step}");
        }
    }
}
