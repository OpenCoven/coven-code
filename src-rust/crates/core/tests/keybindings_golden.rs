//! Default keybinding table golden.
//!
//! Freezes `claurst_core::keybindings::default_bindings()`: every default
//! chord, the action it maps to, and the context it applies in. The text form
//! is one binding per line in declaration order, so a reorder, rename, or
//! dropped binding shows up as a one-line diff.
//!
//! Regenerate after a deliberate keybinding change and review the diff in
//! the PR:
//!
//! ```text
//! UPDATE_GOLDENS=1 cargo test -p claurst-core --test keybindings_golden
//! ```

use std::fs;
use std::path::{Path, PathBuf};

use claurst_core::keybindings::{default_bindings, ParsedKeystroke};

fn golden_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden/default_keybindings.txt")
}

fn assert_golden(path: &Path, actual: &str) {
    if std::env::var_os("UPDATE_GOLDENS").is_some() {
        fs::write(path, actual).expect("write golden");
        return;
    }
    let expected = fs::read_to_string(path).unwrap_or_else(|e| {
        panic!(
            "missing golden {}: {e}\nregenerate with: UPDATE_GOLDENS=1 cargo test -p claurst-core --test keybindings_golden",
            path.display()
        )
    });
    if expected != actual {
        let mut report = String::new();
        for (n, (want, got)) in expected.lines().zip(actual.lines()).enumerate() {
            if want != got {
                report.push_str(&format!("line {}:\n  want: {want}\n  got:  {got}\n", n + 1));
            }
        }
        panic!(
            "default keybindings drifted from {} (expected {} lines, got {}). \
             If this is intentional, regenerate with UPDATE_GOLDENS=1 and review the diff.\n{report}",
            path.display(),
            expected.lines().count(),
            actual.lines().count(),
        );
    }
}

/// Canonical `ctrl+alt+shift+meta+<key>` spelling, modifiers in fixed order.
fn keystroke_text(ks: &ParsedKeystroke) -> String {
    let mut parts: Vec<&str> = Vec::new();
    if ks.ctrl {
        parts.push("ctrl");
    }
    if ks.alt {
        parts.push("alt");
    }
    if ks.shift {
        parts.push("shift");
    }
    if ks.meta {
        parts.push("meta");
    }
    parts.push(ks.key.as_str());
    parts.join("+")
}

fn render_bindings() -> String {
    let mut out =
        String::from("# default keybindings in declaration order: context\tchord\taction\n");
    for binding in default_bindings() {
        let chord = binding
            .chord
            .iter()
            .map(keystroke_text)
            .collect::<Vec<_>>()
            .join(" ");
        out.push_str(&format!(
            "{:?}\t{chord}\t{}\n",
            binding.context,
            binding.action.as_deref().unwrap_or("<unbound>")
        ));
    }
    out
}

#[test]
fn default_keybindings_match_golden() {
    assert_golden(&golden_path(), &render_bindings());
}
