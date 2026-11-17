//! Panic restoration probe for `scripts/tui-tests/panic_restore_pty.py` (#417).
//!
//! Takes the terminal with the real `claurst_tui::setup_terminal`, draws a
//! frame, waits for one byte on stdin, then panics. The driver runs it on a
//! PTY and checks what the panic hook gives back. Not shipped.
//!
//! Usage: `panic_restore <classic|next> <main|worker|resume>`
//!
//! - `main` panics on the thread that set the terminal up.
//! - `worker` first panics on a spawned thread, which must leave the terminal
//!   alone, draws again, waits for a second byte, then panics on the main
//!   thread.
//! - `resume` (Unix) first hands the terminal back and takes it again through
//!   the real `suspend_terminal`, the path Ctrl+Z and `fg` take, without
//!   stopping the process; then draws again, waits for a second byte and
//!   panics on the main thread.

use std::io::{self, Read};

use claurst_tui::application::UiRenderer;
use claurst_tui::AppTerminal;
use ratatui::widgets::Paragraph;

enum Mode {
    Main,
    Worker,
    #[cfg(unix)]
    Resume,
}

fn draw(terminal: &mut AppTerminal, text: &str) -> io::Result<()> {
    terminal.draw(|frame| frame.render_widget(Paragraph::new(text), frame.area()))?;
    Ok(())
}

fn wait_for_byte() -> io::Result<()> {
    io::stdin().read_exact(&mut [0u8; 1])
}

fn main() -> io::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [renderer, mode] = args.iter().map(String::as_str).collect::<Vec<_>>()[..] else {
        return Err(usage());
    };
    let renderer = match renderer {
        "classic" => UiRenderer::Classic,
        "next" => UiRenderer::Next,
        _ => return Err(usage()),
    };
    let mode = match mode {
        "main" => Mode::Main,
        "worker" => Mode::Worker,
        #[cfg(unix)]
        "resume" => Mode::Resume,
        _ => return Err(usage()),
    };

    let mut terminal = claurst_tui::setup_terminal(renderer)?;
    draw(&mut terminal, "panic-restore: ready")?;
    wait_for_byte()?;

    match mode {
        Mode::Main => {}
        Mode::Worker => {
            let joined: std::thread::Result<()> =
                std::thread::spawn(|| panic!("panic-restore: deliberate worker-thread panic"))
                    .join();
            let status = if joined.is_err() {
                "panic-restore: worker panicked"
            } else {
                "panic-restore: worker did not panic"
            };
            // Repaint every cell: the worker's panic message was written over
            // the frame behind ratatui's back.
            terminal.clear()?;
            draw(&mut terminal, status)?;
            wait_for_byte()?;
        }
        #[cfg(unix)]
        Mode::Resume => {
            let stopped = claurst_tui::suspend_terminal(&mut terminal, renderer, || {})?;
            let status = if stopped {
                "panic-restore: resumed"
            } else {
                "panic-restore: not handed back"
            };
            draw(&mut terminal, status)?;
            wait_for_byte()?;
        }
    }

    panic!("panic-restore: deliberate main-thread panic");
}

fn usage() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        "usage: panic_restore <classic|next> <main|worker|resume>",
    )
}
