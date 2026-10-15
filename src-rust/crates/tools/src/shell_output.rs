/// Retain up to 50,000 bytes from each end of long foreground shell output.
/// The omission marker is additional to that byte budget.
pub(crate) fn truncate(output: String) -> String {
    const MAX_OUTPUT_BYTES: usize = 100_000;
    if output.len() <= MAX_OUTPUT_BYTES {
        return output;
    }

    let mut head_end = MAX_OUTPUT_BYTES / 2;
    while !output.is_char_boundary(head_end) {
        head_end -= 1;
    }
    let mut tail_start = output.len() - MAX_OUTPUT_BYTES / 2;
    while !output.is_char_boundary(tail_start) {
        tail_start += 1;
    }
    format!(
        "{}\n\n... ({} bytes truncated) ...\n\n{}",
        &output[..head_end],
        tail_start - head_end,
        &output[tail_start..]
    )
}

#[cfg(test)]
mod tests {
    use super::truncate;

    #[test]
    fn retains_short_output_and_exact_limit_without_reallocation() {
        for input in [String::new(), "short\u{e9}".into(), "a".repeat(100_000)] {
            let original_pointer = input.as_ptr();
            let expected = input.clone();
            let actual = truncate(input);
            assert_eq!(actual, expected);
            assert_eq!(actual.as_ptr(), original_pointer);
        }
    }

    #[test]
    fn ascii_retains_both_ends_and_reports_omitted_bytes() {
        for middle in ["x", "middle"] {
            let head = "a".repeat(50_000);
            let tail = "b".repeat(50_000);
            assert_eq!(
                truncate(format!("{head}{middle}{tail}")),
                format!(
                    "{head}\n\n... ({} bytes truncated) ...\n\n{tail}",
                    middle.len()
                )
            );
        }
    }

    #[test]
    fn cuts_inside_multibyte_characters_discard_whole_characters() {
        for character in ['\u{e9}', '\u{6771}', '\u{10000}'] {
            for head_offset in 1..character.len_utf8() {
                for tail_offset in 1..character.len_utf8() {
                    let head = "a".repeat(50_000 - head_offset);
                    let tail = "b".repeat(50_000 - tail_offset);
                    let input = format!("{head}{character}middle{character}{tail}");
                    let removed = 2 * character.len_utf8() + "middle".len();
                    assert_eq!(
                        truncate(input),
                        format!("{head}\n\n... ({removed} bytes truncated) ...\n\n{tail}")
                    );
                }
            }
        }
    }

    #[test]
    fn keeps_multibyte_characters_at_exact_boundaries() {
        for character in ['\u{e9}', '\u{6771}', '\u{10000}'] {
            let head = format!("{}{character}", "a".repeat(50_000 - character.len_utf8()));
            let tail = format!("{character}{}", "b".repeat(50_000 - character.len_utf8()));
            assert_eq!(
                truncate(format!("{head}middle{tail}")),
                format!("{head}\n\n... (6 bytes truncated) ...\n\n{tail}")
            );
        }
    }
}
