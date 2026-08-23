#!/usr/bin/env python3
"""Apply the reviewed paste-burst release-event correction to app.rs.

This is a one-shot, branch-scoped migration helper. It asserts the exact source
blob and every replacement target so repository drift fails closed rather than
partially rewriting the TUI event loop.
"""

from __future__ import annotations

import hashlib
from pathlib import Path


APP_PATH = Path("src-rust/crates/tui/src/app.rs")
EXPECTED_BLOB = "4a573d0777372610cd01f4eae18b6478d4fa17fe"


def git_blob_sha(data: bytes) -> str:
    header = f"blob {len(data)}\0".encode("ascii")
    return hashlib.sha1(header + data).hexdigest()


def replace_once(text: str, old: str, new: str, label: str) -> str:
    count = text.count(old)
    if count != 1:
        raise SystemExit(f"{label}: expected exactly one match, found {count}")
    return text.replace(old, new, 1)


def replace_between(text: str, start: str, end: str, replacement: str, label: str) -> str:
    start_count = text.count(start)
    end_count = text.count(end)
    if start_count != 1 or end_count != 1:
        raise SystemExit(
            f"{label}: expected one start/end marker, found {start_count}/{end_count}"
        )
    start_index = text.index(start)
    end_index = text.index(end, start_index)
    return text[:start_index] + replacement + text[end_index:]


def main() -> None:
    source = APP_PATH.read_bytes()
    actual_blob = git_blob_sha(source)
    if actual_blob != EXPECTED_BLOB:
        raise SystemExit(
            f"source drift: expected app.rs blob {EXPECTED_BLOB}, found {actual_blob}"
        )

    text = source.decode("utf-8")

    text = replace_once(
        text,
        """    /// A single key event that was drained from the queue during paste-burst
    /// detection but wasn't part of the burst (e.g. a modifier key that stopped
    /// the burst). Replayed at the top of the next loop iteration.
    pending_key: Option<crossterm::event::KeyEvent>,
""",
        """    /// Key events drained from the queue during paste-burst detection that were
    /// not part of the burst. Replayed in FIFO order at the top of later event
    /// loop iterations so lookahead never drops submission or navigation keys.
    pending_keys: std::collections::VecDeque<crossterm::event::KeyEvent>,
""",
        "pending key field",
    )

    text = replace_once(
        text,
        "            pending_key: None,\n",
        "            pending_keys: std::collections::VecDeque::new(),\n",
        "pending key initializer",
    )

    replacement = """    /// Take the next key event saved by `try_detect_paste_burst` when
    /// lookahead reached a submission or non-text key. Events are replayed in
    /// FIFO order at the top of later event-loop iterations.
    pub fn take_pending_key(&mut self) -> Option<crossterm::event::KeyEvent> {
        self.pending_keys.pop_front()
    }

    fn is_paste_text_key(key: &crossterm::event::KeyEvent) -> bool {
        use crossterm::event::{KeyCode, KeyEventKind, KeyModifiers};

        if key.kind != KeyEventKind::Press {
            return false;
        }

        let text_modifiers =
            key.modifiers == KeyModifiers::NONE || key.modifiers == KeyModifiers::SHIFT;
        text_modifiers && matches!(key.code, KeyCode::Char(_) | KeyCode::Enter)
    }

    /// Classify already-drained key events without depending on the terminal
    /// event queue. Release/repeat events are ignored because both interactive
    /// loops process press events only. Any meaningful key that does not belong
    /// to the paste is retained for ordered replay.
    fn classify_paste_burst_events(
        first: char,
        events: impl IntoIterator<Item = crossterm::event::KeyEvent>,
    ) -> (
        String,
        std::collections::VecDeque<crossterm::event::KeyEvent>,
    ) {
        use crossterm::event::{KeyCode, KeyEventKind};
        use std::collections::VecDeque;

        let mut remaining = events
            .into_iter()
            .filter(|key| key.kind == KeyEventKind::Press)
            .collect::<VecDeque<_>>();
        let mut pending = VecDeque::new();
        let mut buffer = String::new();
        buffer.push(first);

        while let Some(key) = remaining.pop_front() {
            if !Self::is_paste_text_key(&key) {
                pending.push_back(key);
                pending.extend(remaining);
                break;
            }

            match key.code.clone() {
                KeyCode::Char(character) => buffer.push(character),
                KeyCode::Enter => {
                    // Enter is an interior line break only when the next
                    // meaningful press is also paste text. Terminal keyboard
                    // protocols commonly place a Release event immediately
                    // after Enter; those releases were filtered above and must
                    // not make a final submit key look like interior text.
                    let has_following_text = remaining
                        .front()
                        .map(Self::is_paste_text_key)
                        .unwrap_or(false);
                    if has_following_text {
                        buffer.push('\n');
                    } else {
                        pending.push_back(key);
                        pending.extend(remaining);
                        break;
                    }
                }
                _ => unreachable!("paste text predicate admitted a non-text key"),
            }
        }

        (buffer, pending)
    }

    /// Drain any immediately-available key events from the crossterm event
    /// queue (zero-timeout poll) and return them alongside `first` as a single
    /// pasted string if the burst is large enough to be a paste.
    ///
    /// On Windows Terminal, Ctrl+V causes the terminal emulator to write the
    /// clipboard content directly to stdin as raw character events — every
    /// newline becomes an Enter keypress and stray `v` characters trigger
    /// voice PTT. Because a paste dumps its events into the queue together, a
    /// zero-timeout drain immediately after the first character reliably finds
    /// the rest of a non-trivial paste, while normal keyboard typing almost
    /// never queues a second press in the same drain.
    ///
    /// Returns `Some(text)` when a paste burst is detected (caller should route
    /// through `handle_paste_data`). Returns `None` for a normal single
    /// keystroke. Submission and non-text press events reached during lookahead
    /// are retained in `self.pending_keys` and replayed in FIFO order.
    pub fn try_detect_paste_burst(&mut self, first: char) -> Option<String> {
        use crossterm::event::Event;

        // Minimum number of chars (including `first`) to classify as a paste.
        // Two or more is enough: at 120 WPM the inter-key interval is ~60 ms,
        // so a second char in the same zero-timeout drain is extremely unlikely
        // from a human typist but guaranteed from a clipboard paste.
        const BURST_THRESHOLD: usize = 2;

        // Quick exit: don't bother if nothing is queued immediately.
        if !crossterm::event::poll(std::time::Duration::ZERO).unwrap_or(false) {
            return None;
        }

        let mut drained = Vec::new();
        while let Ok(true) = crossterm::event::poll(std::time::Duration::ZERO) {
            match crossterm::event::read() {
                Ok(Event::Key(key)) => drained.push(key),
                // Preserve the existing contract for mouse/resize events: the
                // first non-key event ends paste detection after being consumed.
                _ => break,
            }
        }

        let (buffer, pending) = Self::classify_paste_burst_events(first, drained);
        self.pending_keys.extend(pending);

        if buffer.chars().count() >= BURST_THRESHOLD {
            Some(buffer)
        } else {
            None
        }
    }

"""
    text = replace_between(
        text,
        "    /// Take any key event saved by `try_detect_paste_burst` when a non-character\n",
        "    /// Process mouse events (trackpad scroll, text selection, etc.).\n",
        replacement,
        "paste burst implementation",
    )

    text = replace_once(
        text,
        """            // Replay a key that was saved by try_detect_paste_burst in a
            // previous iteration (e.g. a modifier key that terminated a burst).
            let pending = self.pending_key.take();
""",
        """            // Replay the next key saved by try_detect_paste_burst. A FIFO is
            // required because lookahead can encounter a final Enter followed
            // by another meaningful key after terminal release events.
            let pending = self.take_pending_key();
""",
        "event-loop pending-key replay",
    )

    test_anchor = """    fn press_key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent {
            code,
            modifiers,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }
    }

"""
    test_block = test_anchor + """    fn release_key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent {
            code,
            modifiers,
            kind: KeyEventKind::Release,
            state: KeyEventState::NONE,
        }
    }

    #[test]
    fn paste_burst_release_after_interior_enter_does_not_stop_multiline_text() {
        let (text, pending) = App::classify_paste_burst_events(
            'a',
            [
                press_key(KeyCode::Enter, KeyModifiers::NONE),
                release_key(KeyCode::Enter, KeyModifiers::NONE),
                press_key(KeyCode::Char('b'), KeyModifiers::NONE),
                release_key(KeyCode::Char('b'), KeyModifiers::NONE),
                press_key(KeyCode::Enter, KeyModifiers::NONE),
                release_key(KeyCode::Enter, KeyModifiers::NONE),
            ],
        );

        assert_eq!(text, "a\\nb");
        assert_eq!(
            pending.into_iter().map(|key| key.code).collect::<Vec<_>>(),
            vec![KeyCode::Enter]
        );
    }

    #[test]
    fn paste_burst_final_enter_survives_kitty_release_event() {
        let (text, pending) = App::classify_paste_burst_events(
            'a',
            [
                press_key(KeyCode::Char('b'), KeyModifiers::NONE),
                release_key(KeyCode::Char('b'), KeyModifiers::NONE),
                press_key(KeyCode::Enter, KeyModifiers::NONE),
                release_key(KeyCode::Enter, KeyModifiers::NONE),
            ],
        );

        assert_eq!(text, "ab");
        assert_eq!(
            pending.into_iter().map(|key| key.code).collect::<Vec<_>>(),
            vec![KeyCode::Enter]
        );
    }

    #[test]
    fn paste_burst_replays_submit_before_following_non_text_key() {
        let (text, pending) = App::classify_paste_burst_events(
            'a',
            [
                press_key(KeyCode::Char('b'), KeyModifiers::NONE),
                press_key(KeyCode::Enter, KeyModifiers::NONE),
                release_key(KeyCode::Enter, KeyModifiers::NONE),
                press_key(KeyCode::Left, KeyModifiers::NONE),
                release_key(KeyCode::Left, KeyModifiers::NONE),
            ],
        );

        assert_eq!(text, "ab");
        assert_eq!(
            pending.into_iter().map(|key| key.code).collect::<Vec<_>>(),
            vec![KeyCode::Enter, KeyCode::Left]
        );
    }

    #[test]
    fn paste_burst_release_events_do_not_count_as_text() {
        let (text, pending) = App::classify_paste_burst_events(
            'a',
            [release_key(KeyCode::Char('a'), KeyModifiers::NONE)],
        );

        assert_eq!(text, "a");
        assert!(pending.is_empty());
    }

"""
    text = replace_once(text, test_anchor, test_block, "paste burst regression tests")

    if "pending_key" in text:
        # Only the public method name and documentation reference should remain.
        leftovers = [line for line in text.splitlines() if "pending_key" in line]
        allowed = (
            "take_pending_key",
            "pending_keys",
        )
        unexpected = [line for line in leftovers if not any(item in line for item in allowed)]
        if unexpected:
            raise SystemExit(f"unexpected stale pending_key references: {unexpected}")

    APP_PATH.write_text(text, encoding="utf-8")
    print("patched", APP_PATH)


if __name__ == "__main__":
    main()
