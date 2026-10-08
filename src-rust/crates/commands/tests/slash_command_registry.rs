//! Slash-command registry golden.
//!
//! Freezes the three things the TUI, the `/help` text, and the command
//! palette agree on: the autocomplete list (`PROMPT_SLASH_COMMANDS`), the
//! category each name maps to (`slash_command_category`), and the registered
//! `SlashCommand` set (names, aliases, hidden flag). Lives in this crate
//! because it is the one place both `claurst_commands::all_commands()` and
//! `claurst_tui::app` are reachable.
//!
//! The golden is for the default feature set. The opt-in `steer` entry is
//! filtered out so a `--features steer` build compares the same text.
//!
//! Regenerate after a deliberate registry change (new command, rename,
//! recategorisation) and review the diff in the PR:
//!
//! ```text
//! UPDATE_GOLDENS=1 cargo test -p claurst-commands --test slash_command_registry
//! ```

use std::fs;
use std::path::{Path, PathBuf};

use claurst_commands::all_commands;
use claurst_tui::app::{slash_command_category, PROMPT_SLASH_COMMANDS};

/// Feature-gated names excluded from the golden so it is build-invariant.
const FEATURE_GATED: &[&str] = &["steer"];

fn golden_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden/slash_command_registry.txt")
}

fn assert_golden(path: &Path, actual: &str) {
    if std::env::var_os("UPDATE_GOLDENS").is_some() {
        fs::write(path, actual).expect("write golden");
        return;
    }
    let expected = fs::read_to_string(path).unwrap_or_else(|e| {
        panic!(
            "missing golden {}: {e}\nregenerate with: UPDATE_GOLDENS=1 cargo test -p claurst-commands --test slash_command_registry",
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
            "slash-command registry drifted from {} (expected {} lines, got {}). \
             If this is intentional, regenerate with UPDATE_GOLDENS=1 and review the diff.\n{report}",
            path.display(),
            expected.lines().count(),
            actual.lines().count(),
        );
    }
}

fn render_registry() -> String {
    let mut out = String::new();

    out.push_str("# PROMPT_SLASH_COMMANDS in palette order: name\tcategory\tdescription\n");
    for (name, description) in PROMPT_SLASH_COMMANDS
        .iter()
        .filter(|(name, _)| !FEATURE_GATED.contains(name))
    {
        out.push_str(&format!(
            "{name}\t{}\t{description}\n",
            slash_command_category(name)
        ));
    }

    out.push_str("\n# registered SlashCommands sorted by name: name\tcategory\taliases\thidden\n");
    let mut registered: Vec<(&str, Vec<&str>, bool)> = all_commands()
        .iter()
        .filter(|c| !FEATURE_GATED.contains(&c.name()))
        .map(|c| (c.name(), c.aliases(), c.hidden()))
        .collect();
    registered.sort_by(|a, b| a.0.cmp(b.0));
    for (name, aliases, hidden) in registered {
        out.push_str(&format!(
            "{name}\t{}\t{}\t{hidden}\n",
            slash_command_category(name),
            if aliases.is_empty() {
                "-".to_string()
            } else {
                aliases.join(",")
            },
        ));
    }
    out
}

#[test]
fn slash_command_registry_matches_golden() {
    assert_golden(&golden_path(), &render_registry());
}
