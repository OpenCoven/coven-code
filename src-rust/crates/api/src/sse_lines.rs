/// Assemble complete LF-terminated lines before decoding UTF-8. HTTP chunk
/// boundaries are neither character boundaries nor SSE frame separators.
#[derive(Default)]
pub(crate) struct SseLines {
    pending: Vec<u8>,
}

impl SseLines {
    pub(crate) fn feed(&mut self, chunk: &[u8]) -> Vec<String> {
        let mut lines = Vec::new();
        for part in chunk.split_inclusive(|byte| *byte == b'\n') {
            self.pending.extend_from_slice(part);
            if self.pending.last() == Some(&b'\n') {
                self.pending.pop();
                lines.push(String::from_utf8_lossy(&self.pending).into_owned());
                self.pending.clear();
            }
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn complete_lines_are_independent_of_byte_partition() {
        let bytes = "é中\u{10000}\r\n\r\nlast\nunfinished".as_bytes();
        for split in 0..=bytes.len() {
            let mut lines = SseLines::default();
            let mut actual = lines.feed(&bytes[..split]);
            actual.extend(lines.feed(&bytes[split..]));
            assert_eq!(actual, ["é中\u{10000}\r", "\r", "last"], "split {split}");
            assert!(lines.feed(&[]).is_empty());
        }
    }

    #[test]
    fn malformed_utf8_remains_lossy_after_line_assembly() {
        let bytes = b"valid\xff\xf0\x90tail\n";
        let expected = String::from_utf8_lossy(&bytes[..bytes.len() - 1]);
        for split in 0..=bytes.len() {
            let mut lines = SseLines::default();
            let mut actual = lines.feed(&bytes[..split]);
            actual.extend(lines.feed(&bytes[split..]));
            assert_eq!(actual, [expected.as_ref()], "split {split}");
        }
    }

    #[test]
    fn newline_emits_exactly_one_line_and_eof_does_not_flush_partial_line() {
        let mut lines = SseLines::default();
        assert_eq!(lines.feed(b"event: ping\n"), ["event: ping"]);
        assert!(lines.feed(&[]).is_empty());
        assert_eq!(lines.feed(b"\n"), [""]);
        assert!(lines.feed(b"data: incomplete").is_empty());
    }
}
