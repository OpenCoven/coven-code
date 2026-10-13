//! Slash-command registry golden.
//!
//! Freezes the canonical catalog, including grouped paths, lifecycle, aliases,
//! category, usage, and descriptions. Feature-gated steering paths are excluded.
//!
//! Regenerate after a deliberate registry change (new command, rename,
//! recategorisation) and review the diff in the PR:
//!
//! ```text
//! UPDATE_GOLDENS=1 cargo test -p claurst-commands --test slash_command_registry
//! ```

use std::fs;
use std::path::Path;

use claurst_commands::{
    builtin_catalog,
    catalog::{flatten_catalog, CommandLifecycle},
};

/// Feature-gated names excluded from the golden so it is build-invariant.
const FEATURE_GATED: &[&str] = &["steer", "thinking steer"];

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
    let catalog = builtin_catalog();
    let mut entries = flatten_catalog(&catalog);
    entries.sort_by(|left, right| left.path.cmp(&right.path));
    let mut out = String::from(
        "# Canonical command catalog: path\tcategory\tstate\taliases\tusage\tsummary\n",
    );
    for entry in entries
        .into_iter()
        .filter(|entry| !FEATURE_GATED.contains(&entry.path.as_str()))
    {
        let state = match &entry.lifecycle {
            CommandLifecycle::Active => "active".to_string(),
            CommandLifecycle::Advanced => "advanced".to_string(),
            CommandLifecycle::Removed { replacement } => format!("removed:/{replacement}"),
        };
        let aliases = if entry.aliases.is_empty() {
            "-".to_string()
        } else {
            entry.aliases.join(",")
        };
        out.push_str(&format!(
            "{}\t{}\t{}\t{}\t{}\t{}\n",
            entry.path,
            entry.category.label(),
            state,
            aliases,
            entry.usage.replace("steer|", ""),
            entry.summary
        ));
    }
    out
}

#[test]
fn slash_command_registry_matches_golden() {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden/slash_command_registry.txt");
    assert_golden(&path, &render_registry());
}

#[test]
#[cfg(not(feature = "steer"))]
fn generated_command_reference_is_current() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../docs/slash-commands.md");
    assert_eq!(
        fs::read_to_string(path).expect("command reference"),
        claurst_commands::catalog::render_reference()
    );
}
