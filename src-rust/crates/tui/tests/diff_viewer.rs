//! T5-3: Diff viewer tests.

use claurst_tui::diff_viewer::{
    parse_unified_diff, render_diff_dialog, DiffLineKind, DiffViewerState, FileDiffStats,
};
use ratatui::backend::TestBackend;
use ratatui::Terminal;

// ---------------------------------------------------------------------------
// parse_unified_diff
// ---------------------------------------------------------------------------

const SIMPLE_DIFF: &str = r#"diff --git a/src/main.rs b/src/main.rs
--- a/src/main.rs
+++ b/src/main.rs
@@ -1,5 +1,6 @@
 fn main() {
-    println!("hello");
+    println!("hello, world");
+    println!("added line");
 }
"#;

#[test]
fn parse_diff_returns_hunks() {
    let files = parse_unified_diff(SIMPLE_DIFF);
    assert!(!files.is_empty(), "should have at least one file");
    let has_hunks: bool = files.iter().any(|f| !f.hunks.is_empty());
    assert!(has_hunks, "should have at least one hunk");
}

#[test]
fn parse_diff_has_added_lines() {
    let files = parse_unified_diff(SIMPLE_DIFF);
    let added: Vec<_> = files
        .iter()
        .flat_map(|f| f.hunks.iter())
        .flat_map(|h| h.lines.iter())
        .filter(|l| l.kind == DiffLineKind::Added)
        .collect();
    assert!(!added.is_empty(), "should have added lines");
    let texts: Vec<_> = added.iter().map(|l| l.content.as_str()).collect();
    assert!(texts.iter().any(|t| t.contains("hello, world")));
}

#[test]
fn parse_diff_has_removed_lines() {
    let files = parse_unified_diff(SIMPLE_DIFF);
    let removed: Vec<_> = files
        .iter()
        .flat_map(|f| f.hunks.iter())
        .flat_map(|h| h.lines.iter())
        .filter(|l| l.kind == DiffLineKind::Removed)
        .collect();
    assert!(!removed.is_empty(), "should have removed lines");
    let texts: Vec<_> = removed.iter().map(|l| l.content.as_str()).collect();
    assert!(texts.iter().any(|t| t.contains("hello")));
}

#[test]
fn parse_diff_has_context_lines() {
    let files = parse_unified_diff(SIMPLE_DIFF);
    let context: Vec<_> = files
        .iter()
        .flat_map(|f| f.hunks.iter())
        .flat_map(|h| h.lines.iter())
        .filter(|l| l.kind == DiffLineKind::Context)
        .collect();
    assert!(!context.is_empty(), "should have context lines");
}

#[test]
fn parse_empty_diff() {
    let files = parse_unified_diff("");
    assert!(files.is_empty());
}

#[test]
fn parse_diff_hunk_range() {
    let files = parse_unified_diff(SIMPLE_DIFF);
    assert!(!files.is_empty());
    let hunk = &files[0].hunks[0];
    // old_range.0 = old_start, new_range.0 = new_start
    // The @@ -1,5 +1,6 @@ header gives old_start=1, new_start=1
    // But the first line is the Header kind; real data starts in old_range/new_range
    assert!(hunk.old_range.0 > 0 || hunk.new_range.0 > 0);
}

#[test]
fn parse_multifile_diff() {
    let multi = r#"diff --git a/foo.rs b/foo.rs
--- a/foo.rs
+++ b/foo.rs
@@ -1,2 +1,2 @@
-old
+new
diff --git a/bar.rs b/bar.rs
--- a/bar.rs
+++ b/bar.rs
@@ -1,1 +1,1 @@
-x
+y
"#;
    let files = parse_unified_diff(multi);
    assert!(files.len() >= 2);
}

#[test]
fn parse_diff_stats_added_removed() {
    let files = parse_unified_diff(SIMPLE_DIFF);
    // SIMPLE_DIFF has 2 added lines and 1 removed line (not counting Header)
    let total_added: u32 = files.iter().map(|f| f.added).sum();
    let total_removed: u32 = files.iter().map(|f| f.removed).sum();
    assert!(total_added >= 1, "should have counted added lines");
    assert!(total_removed >= 1, "should have counted removed lines");
}

// ---------------------------------------------------------------------------
// Binary and renamed files (#411)
// ---------------------------------------------------------------------------

/// What `git diff HEAD` prints, rename detection on (its default), after a
/// binary file changed, `git mv` renamed one file, and `git mv` renamed
/// another whose name has a space and then it was edited.
const KINDS_DIFF: &str = "diff --git a/before edit.txt b/after edit.txt
similarity index 94%
rename from before edit.txt
rename to after edit.txt
index c61f4c9..206389a 100644
--- a/before edit.txt\t
+++ b/after edit.txt\t
@@ -9,3 +9,3 @@ moved line 8
 moved line 9
-moved line 10
+moved line TEN
 moved line 11
diff --git a/blob.bin b/blob.bin
index 32f65bc..b0041d4 100644
Binary files a/blob.bin and b/blob.bin differ
diff --git a/old-name.txt b/new-name.txt
similarity index 100%
rename from old-name.txt
rename to new-name.txt
";

/// The diff dialog drawn at `width` x `height` with `selected` chosen, one
/// string per row.
fn draw_dialog(files: Vec<FileDiffStats>, selected: usize, width: u16, height: u16) -> Vec<String> {
    let mut state = DiffViewerState::new();
    state.visible = true;
    state.collapsed = vec![false; files.len()];
    state.files = files;
    state.selected_file = selected;
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| render_diff_dialog(&state, frame.area(), frame.buffer_mut()))
        .unwrap();
    terminal
        .backend()
        .buffer()
        .content()
        .chunks(usize::from(width))
        .map(|row| row.iter().map(|cell| cell.symbol()).collect())
        .collect()
}

fn shows(rows: &[String], text: &str) -> bool {
    rows.iter().any(|row| row.contains(text))
}

#[test]
fn the_viewer_names_both_paths_of_a_rename_and_marks_a_binary_change() {
    for (width, height) in [(120, 40), (80, 24)] {
        let at = format!("{width}x{height}");

        let rows = draw_dialog(parse_unified_diff(KINDS_DIFF), 0, width, height);
        let all = rows.join("\n");
        assert!(
            shows(&rows, "before edit.txt \u{2192} after edit.txt  +1 -1"),
            "{at}: an edited rename names both paths and counts its lines:\n{all}"
        );
        assert!(
            shows(&rows, "moved line TEN"),
            "{at}: and shows its edit:\n{all}"
        );
        assert!(
            rows.iter().any(|row| row.contains("\u{25be} ")
                && row.contains("edit.txt ")
                && row.contains(" renamed")),
            "{at}: the list marks it renamed and keeps the end of its name:\n{all}"
        );

        let rows = draw_dialog(parse_unified_diff(KINDS_DIFF), 1, width, height);
        let all = rows.join("\n");
        assert!(
            shows(&rows, "blob.bin  binary"),
            "{at}: the header says binary:\n{all}"
        );
        assert!(
            shows(&rows, "Binary file \u{2014} no diff available"),
            "{at}: a binary change says so:\n{all}"
        );

        let rows = draw_dialog(parse_unified_diff(KINDS_DIFF), 2, width, height);
        let all = rows.join("\n");
        assert!(
            shows(&rows, "old-name.txt \u{2192} new-name.txt"),
            "{at}: a pure rename names both paths:\n{all}"
        );
        assert!(
            shows(&rows, "Renamed from old-name.txt"),
            "{at}: and says what it was:\n{all}"
        );
        assert!(
            rows.iter()
                .any(|row| row.contains("\u{25be} new-name.txt") && row.contains(" renamed")),
            "{at}: the list marks it renamed, not +0 -0:\n{all}"
        );
    }
}

#[test]
fn a_rename_keeps_its_old_path_and_its_new_one_in_full() {
    let files = parse_unified_diff(KINDS_DIFF);
    let found: Vec<_> = files
        .iter()
        .map(|file| {
            (
                file.path.as_str(),
                file.renamed_from.as_deref(),
                file.binary,
                file.added,
                file.removed,
                file.hunks.len(),
            )
        })
        .collect();
    assert_eq!(
        found,
        [
            ("after edit.txt", Some("before edit.txt"), false, 1, 1, 1),
            ("blob.bin", None, true, 0, 0, 0),
            ("new-name.txt", Some("old-name.txt"), false, 0, 0, 0),
        ]
    );
}

#[test]
fn a_long_path_is_cut_at_a_character_and_keeps_its_end() {
    let path = format!(
        "src/{}/end.rs",
        "\u{65e5}\u{672c}\u{8a9e}\u{00e9}".repeat(12)
    );
    let diff = format!(
        "diff --git a/{path} b/{path}\n--- a/{path}\n+++ b/{path}\n@@ -1 +1 @@\n-old\n+new\n"
    );
    for width in 60..=120 {
        let rows = draw_dialog(parse_unified_diff(&diff), 0, width, 24);
        let all = rows.join("\n");
        assert!(
            rows.iter().any(|row| row.contains("\u{2026}")
                && row.contains("end.rs")
                && row.contains("+1 -1")),
            "{width} columns: the list row keeps the end of the path and its stats:\n{all}"
        );
    }
}
