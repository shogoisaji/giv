/// Unified diff parser.
///
/// `parse_unified_diff` converts the output of `git diff` (unified format)
/// into a `Diff` struct. Handles binary files, renames, and multi-file diffs.
///
/// `intra_line_spans` computes word-level changed segments between a removed
/// and an added line for highlighted inline diffs.
use crate::git::types::{Diff, DiffLine, DiffLineKind, FileDiff, Hunk};

// ─── Public entry point ──────────────────────────────────────────────────────

/// Parse a unified diff text (e.g. from `git diff`) into a `Diff`.
///
/// Returns an empty `Diff` if `text` is empty or cannot be parsed.
pub fn parse_unified_diff(text: &str) -> Diff {
    if text.trim().is_empty() {
        return Diff::default();
    }

    let mut files: Vec<FileDiff> = Vec::new();
    let mut current_file: Option<FileDiff> = None;
    let mut current_hunk: Option<Hunk> = None;

    for line in text.lines() {
        if line.starts_with("diff --git ") {
            // Flush hunk and file.
            flush_hunk(&mut current_hunk, &mut current_file);
            if let Some(f) = current_file.take() {
                files.push(f);
            }
            // Parse paths from `diff --git a/foo b/bar`
            let (old_path, new_path) = parse_diff_git_header(line);
            current_file = Some(FileDiff {
                old_path,
                new_path,
                is_binary: false,
                hunks: Vec::new(),
            });
        } else if line.starts_with("Binary files ")
            || line.contains("binary") && line.starts_with("GIT binary patch")
        {
            if let Some(f) = current_file.as_mut() {
                f.is_binary = true;
            }
        } else if let Some(stripped) = line.strip_prefix("--- ") {
            // Only update path if not /dev/null (new file mode sets it differently)
            let path = strip_ab_prefix(stripped);
            if let Some(f) = current_file.as_mut() {
                f.old_path = path.to_owned();
            }
        } else if let Some(stripped) = line.strip_prefix("+++ ") {
            let path = strip_ab_prefix(stripped);
            if let Some(f) = current_file.as_mut() {
                f.new_path = path.to_owned();
            }
        } else if line.starts_with("@@ ") {
            // Flush previous hunk.
            flush_hunk(&mut current_hunk, &mut current_file);
            if let Some(h) = parse_hunk_header(line) {
                // Add the hunk header line itself as a Header DiffLine
                let mut hunk = h;
                hunk.lines.push(DiffLine {
                    kind: DiffLineKind::Header,
                    text: line.to_owned(),
                });
                current_hunk = Some(hunk);
            }
        } else if let Some(hunk) = current_hunk.as_mut() {
            // Diff content lines.
            let (kind, text) = if let Some(rest) = line.strip_prefix('+') {
                (DiffLineKind::Added, rest.to_owned())
            } else if let Some(rest) = line.strip_prefix('-') {
                (DiffLineKind::Removed, rest.to_owned())
            } else if let Some(rest) = line.strip_prefix(' ') {
                (DiffLineKind::Context, rest.to_owned())
            } else if line == "\\ No newline at end of file" {
                (DiffLineKind::Meta, line.to_owned())
            } else {
                // Unexpected line inside a hunk — treat as meta.
                (DiffLineKind::Meta, line.to_owned())
            };
            hunk.lines.push(DiffLine { kind, text });
        } else if current_file.is_some() {
            // Header lines between `diff --git` and first `@@` (index, mode, etc.)
            // We skip storing them for now (they aren't needed for rendering).
        }
    }

    // Flush remaining hunk and file.
    flush_hunk(&mut current_hunk, &mut current_file);
    if let Some(f) = current_file.take() {
        files.push(f);
    }

    Diff { files }
}

/// A single intra-line diff span: `(changed, text)`.
/// `changed = true` means that segment differs between the two lines.
pub type IntraSpan = (bool, String);

/// Compute word-level diff spans between a removed line and an added line.
///
/// Returns `(removed_spans, added_spans)` where each span is `(changed: bool, text: String)`.
/// `changed = true` means that segment differs between the two lines.
pub fn intra_line_spans(removed: &str, added: &str) -> (Vec<IntraSpan>, Vec<IntraSpan>) {
    use similar::{ChangeTag, TextDiff};

    // Use character-level diff for intra-line highlighting.
    let diff = TextDiff::from_chars(removed, added);

    let mut rem_spans: Vec<(bool, String)> = Vec::new();
    let mut add_spans: Vec<(bool, String)> = Vec::new();

    for change in diff.iter_all_changes() {
        let ch = change.value();
        match change.tag() {
            ChangeTag::Equal => {
                // Character exists in both sides — not changed
                push_or_append(&mut rem_spans, false, ch);
                push_or_append(&mut add_spans, false, ch);
            }
            ChangeTag::Delete => {
                // Character only in removed side
                push_or_append(&mut rem_spans, true, ch);
            }
            ChangeTag::Insert => {
                // Character only in added side
                push_or_append(&mut add_spans, true, ch);
            }
        }
    }

    (rem_spans, add_spans)
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

/// Append `ch` to the last span if it has the same `changed` flag; otherwise push a new span.
fn push_or_append(spans: &mut Vec<(bool, String)>, changed: bool, ch: &str) {
    if let Some(last) = spans.last_mut() {
        if last.0 == changed {
            last.1.push_str(ch);
            return;
        }
    }
    spans.push((changed, ch.to_owned()));
}

/// Flush `current_hunk` into `current_file` if both are present.
fn flush_hunk(current_hunk: &mut Option<Hunk>, current_file: &mut Option<FileDiff>) {
    if let Some(h) = current_hunk.take() {
        if let Some(f) = current_file.as_mut() {
            f.hunks.push(h);
        }
    }
}

/// Parse paths from `diff --git a/old b/new` header line.
/// Falls back to empty strings on parse failure.
fn parse_diff_git_header(line: &str) -> (String, String) {
    // Format: diff --git a/<old> b/<new>
    // The filenames may contain spaces. We can't split on space naively.
    // However git guarantees the format is `a/<old> b/<new>` where old and new
    // are mirrored paths for non-renames. For renames the --- and +++ lines are
    // more reliable. We do a best-effort parse here.
    let rest = match line.strip_prefix("diff --git ") {
        Some(r) => r,
        None => return (String::new(), String::new()),
    };

    // Try to find ` b/` after `a/` as a split point.
    // Look for ` b/` from the right to handle spaces in filenames.
    if let Some(b_pos) = find_b_separator(rest) {
        let old = rest[2..b_pos].to_owned(); // strip `a/`
        let new = rest[b_pos + 3..].to_owned(); // strip ` b/`
        return (old, new);
    }

    (String::new(), String::new())
}

/// Find the position of ` b/` that separates `a/<old>` from `b/<new>` in the diff header.
/// Handles filenames with spaces by scanning from the right.
fn find_b_separator(s: &str) -> Option<usize> {
    // s starts with `a/`; look for last occurrence of ` b/`
    let bytes = s.as_bytes();
    if bytes.len() < 3 {
        return None;
    }
    // Start scanning from `len - 3` downward; we need [i], [i+1], [i+2] all valid.
    let mut i = bytes.len() - 3;
    loop {
        if bytes[i] == b' ' && bytes[i + 1] == b'b' && bytes[i + 2] == b'/' {
            return Some(i);
        }
        if i == 0 {
            break;
        }
        i -= 1;
    }
    None
}

/// Strip the `a/` / `b/` prefix that git prepends to filenames.
fn strip_ab_prefix(s: &str) -> &str {
    if s.starts_with("a/") || s.starts_with("b/") {
        &s[2..]
    } else if s == "/dev/null" {
        "/dev/null"
    } else {
        s
    }
}

/// Parse a hunk header line like `@@ -1,4 +1,6 @@ optional context`.
///
/// Returns `None` on parse failure (hunk will simply not be added).
fn parse_hunk_header(line: &str) -> Option<Hunk> {
    // Format: @@ -old_start[,old_lines] +new_start[,new_lines] @@[ context]
    let inner = line.strip_prefix("@@ ")?;
    let (ranges, rest) = inner.split_once(" @@")?;
    let header_context = rest.trim().to_owned();

    let mut parts = ranges.split_whitespace();
    let old_part = parts.next()?.strip_prefix('-')?;
    let new_part = parts.next()?.strip_prefix('+')?;

    let (old_start, old_lines) = parse_range(old_part);
    let (new_start, new_lines) = parse_range(new_part);

    Some(Hunk {
        header: format!("@@ -{old_part} +{new_part} @@ {header_context}"),
        old_start,
        old_lines,
        new_start,
        new_lines,
        lines: Vec::new(),
    })
}

fn parse_range(s: &str) -> (u32, u32) {
    if let Some((start, count)) = s.split_once(',') {
        let s = start.parse().unwrap_or(1);
        let c = count.parse().unwrap_or(0);
        (s, c)
    } else {
        let s = s.parse().unwrap_or(1);
        (s, 1)
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::types::DiffLineKind;

    const SIMPLE_DIFF: &str = r#"diff --git a/src/main.rs b/src/main.rs
index 1234567..abcdefg 100644
--- a/src/main.rs
+++ b/src/main.rs
@@ -1,4 +1,6 @@ fn main() {
 fn main() {
-    println!("hello");
+    println!("hello, world");
+    println!("goodbye");
 }
"#;

    const BINARY_DIFF: &str = r#"diff --git a/image.png b/image.png
index 1234567..abcdefg 100644
Binary files a/image.png and b/image.png differ
"#;

    const TWO_FILE_DIFF: &str = r#"diff --git a/foo.rs b/foo.rs
index 0000000..1111111 100644
--- a/foo.rs
+++ b/foo.rs
@@ -1,3 +1,3 @@
 fn foo() {}
-fn bar() {}
+fn baz() {}
 fn qux() {}
diff --git a/bar.rs b/bar.rs
index 2222222..3333333 100644
--- a/bar.rs
+++ b/bar.rs
@@ -10,2 +10,3 @@
 let x = 1;
+let y = 2;
 let z = 3;
"#;

    const NEW_FILE_DIFF: &str = r#"diff --git a/new_file.txt b/new_file.txt
new file mode 100644
index 0000000..e69de29
--- /dev/null
+++ b/new_file.txt
@@ -0,0 +1,2 @@
+line one
+line two
"#;

    #[test]
    fn test_empty_diff() {
        let diff = parse_unified_diff("");
        assert!(diff.files.is_empty());
    }

    #[test]
    fn test_simple_diff_file_count() {
        let diff = parse_unified_diff(SIMPLE_DIFF);
        assert_eq!(diff.files.len(), 1);
    }

    #[test]
    fn test_simple_diff_paths() {
        let diff = parse_unified_diff(SIMPLE_DIFF);
        let file = &diff.files[0];
        assert_eq!(file.old_path, "src/main.rs");
        assert_eq!(file.new_path, "src/main.rs");
        assert!(!file.is_binary);
    }

    #[test]
    fn test_simple_diff_hunk_header() {
        let diff = parse_unified_diff(SIMPLE_DIFF);
        let file = &diff.files[0];
        assert_eq!(file.hunks.len(), 1);
        let hunk = &file.hunks[0];
        assert_eq!(hunk.old_start, 1);
        assert_eq!(hunk.old_lines, 4);
        assert_eq!(hunk.new_start, 1);
        assert_eq!(hunk.new_lines, 6);
    }

    #[test]
    fn test_simple_diff_added_removed_counts() {
        let diff = parse_unified_diff(SIMPLE_DIFF);
        let hunk = &diff.files[0].hunks[0];
        // Filter out the header line itself
        let content_lines: Vec<_> = hunk
            .lines
            .iter()
            .filter(|l| l.kind != DiffLineKind::Header)
            .collect();
        let added = content_lines
            .iter()
            .filter(|l| l.kind == DiffLineKind::Added)
            .count();
        let removed = content_lines
            .iter()
            .filter(|l| l.kind == DiffLineKind::Removed)
            .count();
        let context = content_lines
            .iter()
            .filter(|l| l.kind == DiffLineKind::Context)
            .count();
        assert_eq!(added, 2);
        assert_eq!(removed, 1);
        assert_eq!(context, 2);
    }

    #[test]
    fn test_binary_diff() {
        let diff = parse_unified_diff(BINARY_DIFF);
        assert_eq!(diff.files.len(), 1);
        assert!(diff.files[0].is_binary);
        assert!(diff.files[0].hunks.is_empty());
    }

    #[test]
    fn test_two_file_diff() {
        let diff = parse_unified_diff(TWO_FILE_DIFF);
        assert_eq!(diff.files.len(), 2);
        assert_eq!(diff.files[0].old_path, "foo.rs");
        assert_eq!(diff.files[1].old_path, "bar.rs");
    }

    #[test]
    fn test_two_file_diff_hunk_counts() {
        let diff = parse_unified_diff(TWO_FILE_DIFF);
        assert_eq!(diff.files[0].hunks.len(), 1);
        assert_eq!(diff.files[1].hunks.len(), 1);

        // foo.rs: 1 removed, 1 added
        let hunk0 = &diff.files[0].hunks[0];
        let content: Vec<_> = hunk0
            .lines
            .iter()
            .filter(|l| l.kind != DiffLineKind::Header)
            .collect();
        assert_eq!(
            content
                .iter()
                .filter(|l| l.kind == DiffLineKind::Added)
                .count(),
            1
        );
        assert_eq!(
            content
                .iter()
                .filter(|l| l.kind == DiffLineKind::Removed)
                .count(),
            1
        );
    }

    #[test]
    fn test_new_file_diff() {
        let diff = parse_unified_diff(NEW_FILE_DIFF);
        assert_eq!(diff.files.len(), 1);
        let file = &diff.files[0];
        assert_eq!(file.old_path, "/dev/null");
        assert_eq!(file.new_path, "new_file.txt");
        let hunk = &file.hunks[0];
        assert_eq!(hunk.old_start, 0);
        assert_eq!(hunk.new_start, 1);
        assert_eq!(hunk.new_lines, 2);
        let added: Vec<_> = hunk
            .lines
            .iter()
            .filter(|l| l.kind == DiffLineKind::Added)
            .collect();
        assert_eq!(added.len(), 2);
    }

    #[test]
    fn test_intra_line_spans_identical() {
        let (rem, add) = intra_line_spans("hello", "hello");
        // Everything is equal — no changed spans
        assert!(rem.iter().all(|(c, _)| !c));
        assert!(add.iter().all(|(c, _)| !c));
        let rem_text: String = rem.iter().map(|(_, t)| t.as_str()).collect();
        let add_text: String = add.iter().map(|(_, t)| t.as_str()).collect();
        assert_eq!(rem_text, "hello");
        assert_eq!(add_text, "hello");
    }

    #[test]
    fn test_intra_line_spans_completely_different() {
        let (rem, add) = intra_line_spans("foo", "bar");
        // All chars differ
        let rem_text: String = rem.iter().map(|(_, t)| t.as_str()).collect();
        let add_text: String = add.iter().map(|(_, t)| t.as_str()).collect();
        assert_eq!(rem_text, "foo");
        assert_eq!(add_text, "bar");
        // At least some changed spans
        assert!(rem.iter().any(|(c, _)| *c));
        assert!(add.iter().any(|(c, _)| *c));
    }

    #[test]
    fn test_intra_line_spans_partial_change() {
        let (rem, add) = intra_line_spans("hello world", "hello earth");
        // "hello " is common prefix; "world" vs "earth" differ
        let rem_text: String = rem.iter().map(|(_, t)| t.as_str()).collect();
        let add_text: String = add.iter().map(|(_, t)| t.as_str()).collect();
        assert_eq!(rem_text, "hello world");
        assert_eq!(add_text, "hello earth");
        // There must be at least one unchanged span (the common prefix)
        assert!(rem.iter().any(|(c, _)| !c));
        assert!(add.iter().any(|(c, _)| !c));
    }

    // ── Edge cases for parse_unified_diff ───────────────────────────────────

    #[test]
    fn parse_whitespace_only_input_returns_empty() {
        let diff = parse_unified_diff("   \n  \n");
        assert!(diff.files.is_empty());
    }

    #[test]
    fn parse_no_newline_at_end_of_file_meta_line() {
        let text = r#"diff --git a/f.txt b/f.txt
index 123..456 100644
--- a/f.txt
+++ b/f.txt
@@ -1,2 +1,2 @@
 line1
-line2
\ No newline at end of file
+line2
\ No newline at end of file
"#;
        let diff = parse_unified_diff(text);
        assert_eq!(diff.files.len(), 1);
        let hunk = &diff.files[0].hunks[0];
        // Find the Meta lines (the "\ No newline at end of file" markers).
        let meta_count = hunk
            .lines
            .iter()
            .filter(|l| l.kind == DiffLineKind::Meta)
            .count();
        assert_eq!(meta_count, 2);
    }

    #[test]
    fn parse_rename_diff_uses_triple_plus_path() {
        // A rename: a/old.txt → b/new.txt. The `--- a/old.txt` and
        // `+++ b/new.txt` lines are authoritative.
        let text = r#"diff --git a/old.txt b/new.txt
similarity index 90%
rename from old.txt
rename to new.txt
--- a/old.txt
+++ b/new.txt
@@ -1,1 +1,1 @@
-old content
+new content
"#;
        let diff = parse_unified_diff(text);
        assert_eq!(diff.files.len(), 1);
        let f = &diff.files[0];
        // The --- /+++ lines override the diff --git header.
        assert_eq!(f.old_path, "old.txt");
        assert_eq!(f.new_path, "new.txt");
    }

    #[test]
    fn parse_deleted_file_diff() {
        let text = r#"diff --git a/gone.txt b/gone.txt
deleted file mode 100644
index 123..000
--- a/gone.txt
+++ /dev/null
@@ -1,2 +0,0 @@
-line one
-line two
"#;
        let diff = parse_unified_diff(text);
        assert_eq!(diff.files.len(), 1);
        let f = &diff.files[0];
        assert_eq!(f.old_path, "gone.txt");
        assert_eq!(f.new_path, "/dev/null");
        let hunk = &f.hunks[0];
        assert_eq!(hunk.old_start, 1);
        assert_eq!(hunk.old_lines, 2);
        assert_eq!(hunk.new_start, 0);
        assert_eq!(hunk.new_lines, 0);
        let removed = hunk
            .lines
            .iter()
            .filter(|l| l.kind == DiffLineKind::Removed)
            .count();
        assert_eq!(removed, 2);
    }

    #[test]
    fn parse_hunk_header_without_line_counts() {
        // Some hunks use the single-number form `@@ -1 +1 @@` (count omitted
        // means 1).
        let text = r#"diff --git a/f b/f
index 1..2 100644
--- a/f
+++ b/f
@@ -1 +1 @@
-old
+new
"#;
        let diff = parse_unified_diff(text);
        let hunk = &diff.files[0].hunks[0];
        assert_eq!(hunk.old_start, 1);
        assert_eq!(hunk.old_lines, 1);
        assert_eq!(hunk.new_start, 1);
        assert_eq!(hunk.new_lines, 1);
    }

    #[test]
    fn parse_hunk_header_with_context_label() {
        let text = r#"diff --git a/f b/f
index 1..2 100644
--- a/f
+++ b/f
@@ -1,3 +1,3 @@ fn some_function() {
 ctx
-old
+new
 ctx
"#;
        let diff = parse_unified_diff(text);
        let hunk = &diff.files[0].hunks[0];
        // The header line itself is stored as a Header DiffLine whose text is
        // the raw `@@ ... @@` line.
        let header_line = hunk.lines.iter().find(|l| l.kind == DiffLineKind::Header);
        assert!(header_line.is_some());
        assert!(header_line.unwrap().text.contains("fn some_function()"));
    }

    #[test]
    fn parse_multiple_hunks_in_one_file() {
        let text = r#"diff --git a/f b/f
index 1..2 100644
--- a/f
+++ b/f
@@ -1,2 +1,2 @@
-a
+a
 ctx
@@ -10,2 +10,2 @@
-b
+b
 ctx
"#;
        let diff = parse_unified_diff(text);
        assert_eq!(diff.files.len(), 1);
        assert_eq!(diff.files[0].hunks.len(), 2);
        assert_eq!(diff.files[0].hunks[0].old_start, 1);
        assert_eq!(diff.files[0].hunks[1].old_start, 10);
    }

    #[test]
    fn parse_empty_hunk_body() {
        // A hunk header with no content lines (rare but valid when counts are 0).
        let text = r#"diff --git a/f b/f
index 1..2 100644
--- a/f
+++ b/f
@@ -0,0 +0,0 @@
"#;
        let diff = parse_unified_diff(text);
        assert_eq!(diff.files.len(), 1);
        assert_eq!(diff.files[0].hunks.len(), 1);
        // Only the header line is stored.
        assert_eq!(diff.files[0].hunks[0].lines.len(), 1);
        assert_eq!(diff.files[0].hunks[0].lines[0].kind, DiffLineKind::Header);
    }

    #[test]
    fn parse_git_binary_patch_marker() {
        let text = r#"diff --git a/bin.dat b/bin.dat
index 1..2 100644
GIT binary patch
literal 0
HcmV?d00001

literal 0
HcmV?d00001

"#;
        let diff = parse_unified_diff(text);
        assert_eq!(diff.files.len(), 1);
        assert!(diff.files[0].is_binary);
    }

    #[test]
    fn parse_context_line_with_leading_space() {
        let text = r#"diff --git a/f b/f
index 1..2 100644
--- a/f
+++ b/f
@@ -1,1 +1,1 @@
 unchanged
"#;
        let diff = parse_unified_diff(text);
        let hunk = &diff.files[0].hunks[0];
        let ctx = hunk
            .lines
            .iter()
            .filter(|l| l.kind == DiffLineKind::Context)
            .count();
        assert_eq!(ctx, 1);
    }

    // ── Edge cases for intra_line_spans ─────────────────────────────────────

    #[test]
    fn intra_line_spans_both_empty() {
        let (rem, add) = intra_line_spans("", "");
        assert!(rem.is_empty());
        assert!(add.is_empty());
    }

    #[test]
    fn intra_line_spans_removed_empty_added_nonempty() {
        let (rem, add) = intra_line_spans("", "abc");
        // No removed spans; added is all "changed".
        assert!(rem.is_empty());
        let add_text: String = add.iter().map(|(_, t)| t.as_str()).collect();
        assert_eq!(add_text, "abc");
        assert!(add.iter().all(|(c, _)| *c));
    }

    #[test]
    fn intra_line_spans_added_empty_removed_nonempty() {
        let (rem, add) = intra_line_spans("xyz", "");
        let rem_text: String = rem.iter().map(|(_, t)| t.as_str()).collect();
        assert_eq!(rem_text, "xyz");
        assert!(rem.iter().all(|(c, _)| *c));
        assert!(add.is_empty());
    }

    #[test]
    fn intra_line_spans_single_char_change() {
        let (rem, add) = intra_line_spans("a", "b");
        assert_eq!(rem.len(), 1);
        assert!(rem[0].0);
        assert_eq!(rem[0].1, "a");
        assert_eq!(add.len(), 1);
        assert!(add[0].0);
        assert_eq!(add[0].1, "b");
    }

    #[test]
    fn intra_line_spans_common_suffix() {
        // "foo!" vs "bar!" — common suffix "!" should be a single unchanged span.
        let (rem, add) = intra_line_spans("foo!", "bar!");
        let rem_text: String = rem.iter().map(|(_, t)| t.as_str()).collect();
        let add_text: String = add.iter().map(|(_, t)| t.as_str()).collect();
        assert_eq!(rem_text, "foo!");
        assert_eq!(add_text, "bar!");
        // The "!" should appear as an unchanged span on both sides.
        assert!(rem.iter().any(|(c, t)| !c && t == "!"));
        assert!(add.iter().any(|(c, t)| !c && t == "!"));
    }

    #[test]
    fn intra_line_spans_unicode() {
        // Multi-byte UTF-8 characters should be handled correctly (char-level).
        let (rem, add) = intra_line_spans("héllo", "hållo");
        let rem_text: String = rem.iter().map(|(_, t)| t.as_str()).collect();
        let add_text: String = add.iter().map(|(_, t)| t.as_str()).collect();
        assert_eq!(rem_text, "héllo");
        assert_eq!(add_text, "hållo");
        // The 'h' prefix and 'llo' suffix are unchanged.
        assert!(rem.iter().any(|(c, t)| !c && t == "h"));
        assert!(rem.iter().any(|(c, t)| !c && t == "llo"));
    }

    // ── Helper function tests ───────────────────────────────────────────────

    #[test]
    fn strip_ab_prefix_a() {
        assert_eq!(strip_ab_prefix("a/foo.txt"), "foo.txt");
    }

    #[test]
    fn strip_ab_prefix_b() {
        assert_eq!(strip_ab_prefix("b/foo.txt"), "foo.txt");
    }

    #[test]
    fn strip_ab_prefix_dev_null() {
        assert_eq!(strip_ab_prefix("/dev/null"), "/dev/null");
    }

    #[test]
    fn strip_ab_prefix_no_prefix() {
        assert_eq!(strip_ab_prefix("foo.txt"), "foo.txt");
    }

    #[test]
    fn parse_diff_git_header_simple() {
        let (old, new) = parse_diff_git_header("diff --git a/src/main.rs b/src/main.rs");
        assert_eq!(old, "src/main.rs");
        assert_eq!(new, "src/main.rs");
    }

    #[test]
    fn parse_diff_git_header_with_spaces_in_path() {
        let (old, new) = parse_diff_git_header("diff --git a/my file.txt b/my file.txt");
        assert_eq!(old, "my file.txt");
        assert_eq!(new, "my file.txt");
    }

    #[test]
    fn parse_diff_git_header_rename() {
        let (old, new) = parse_diff_git_header("diff --git a/old.txt b/new.txt");
        assert_eq!(old, "old.txt");
        assert_eq!(new, "new.txt");
    }

    #[test]
    fn parse_diff_git_header_invalid_returns_empty() {
        let (old, new) = parse_diff_git_header("not a diff header");
        assert_eq!(old, "");
        assert_eq!(new, "");
    }

    #[test]
    fn find_b_separator_finds_split() {
        // "a/src/main.rs b/src/main.rs"
        //  0123456789...
        // The ` b/` separator is at byte index 13 (after "a/src/main.rs").
        assert_eq!(find_b_separator("a/src/main.rs b/src/main.rs"), Some(13));
    }

    #[test]
    fn find_b_separator_handles_spaces_in_filename() {
        // "a/my file.txt b/my file.txt" — the separator is the ` b/` before the
        // second "my file.txt". Scans from the right, so the last ` b/` wins.
        let s = "a/my file.txt b/my file.txt";
        assert_eq!(find_b_separator(s), Some(13));
    }

    #[test]
    fn find_b_separator_none_when_no_separator() {
        assert_eq!(find_b_separator("a/foo.txt"), None);
    }

    #[test]
    fn parse_range_with_count() {
        assert_eq!(parse_range("1,4"), (1, 4));
        assert_eq!(parse_range("10,20"), (10, 20));
    }

    #[test]
    fn parse_range_without_count() {
        assert_eq!(parse_range("5"), (5, 1));
    }

    #[test]
    fn parse_range_zero() {
        assert_eq!(parse_range("0,0"), (0, 0));
    }

    #[test]
    fn parse_range_invalid_falls_back_to_defaults() {
        // Non-numeric → unwrap_or defaults to start=1, count=0.
        assert_eq!(parse_range("abc"), (1, 1));
        assert_eq!(parse_range("abc,def"), (1, 0));
    }
}
