use crate::github::FileDiff;

/// Parse each `diff --git` section as a [`FileDiff`].
pub fn parse_git_diff(input: &str) -> impl Iterator<Item = FileDiff> + '_ {
    const HEADER: &str = "diff --git ";
    const SEP: &str = "\ndiff --git ";

    // Hunk lines always start with ' ', '+', '-' or '\', so a line starting
    // with "diff --git " can only be a file header/section.
    let mut chunks = input.split(SEP);

    // The first chunk is either the first file section (its header has no
    // leading newline, so it still carries the prefix) or a preamble to skip.
    let first = chunks.next().and_then(|c| c.strip_prefix(HEADER));

    first.into_iter().chain(chunks).filter_map(parse_file)
}

/// Parse a chunk, between two `diff --git `.
fn parse_file(chunk: &str) -> Option<FileDiff> {
    // `a/<old> b/<new>`; old == new unless the file was renamed.
    let header = chunk.lines().next()?.strip_prefix("a/")?;
    let (old, new) = header.split_once(" b/")?;

    let (mut old, mut new) = (old.to_string(), new.to_string());
    let mut patch = "";

    let mut offset = 0;
    for raw in chunk.split_inclusive('\n') {
        // Found the start of the actual patch
        if raw.starts_with("@@") {
            patch = chunk[offset..].trim_end_matches(['\r', '\n']);
            break;
        }

        let line = raw.trim_end_matches(['\r', '\n']);
        if let Some(p) = line.strip_prefix("rename from ") {
            old = p.to_string();
        } else if let Some(p) = line.strip_prefix("rename to ") {
            new = p.to_string();
        }

        offset += raw.len();
    }

    Some(FileDiff {
        previous_filename: (old != new).then_some(old),
        filename: new,
        patch: patch.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(input: &str) -> Vec<FileDiff> {
        parse_git_diff(input).collect()
    }

    #[test]
    fn empty_and_garbage_input() {
        assert!(parse("").is_empty());
        assert!(parse("\n").is_empty());
        assert!(parse("just some text\nwith no header\n").is_empty());
    }

    #[test]
    fn modify_rename_delete() {
        let input = "\
diff --git a/src/a.rs b/src/a.rs
index c5f9a4288c632..4d72f562a0bc6 100644
--- a/src/a.rs
+++ b/src/a.rs
@@ -1,2 +1,2 @@ fn main
 ctx
-x
+y
diff --git a/old name.txt b/new name.txt
similarity index 100%
rename from old name.txt
rename to new name.txt
diff --git a/gone.txt b/gone.txt
deleted file mode 100644
--- a/gone.txt
+++ /dev/null
@@ -1 +0,0 @@
-bye
";
        let f: Vec<_> = parse(input);
        assert_eq!(f.len(), 3);

        assert_eq!(f[0].filename, "src/a.rs");
        assert_eq!(f[0].previous_filename, None);
        assert_eq!(f[0].patch, "@@ -1,2 +1,2 @@ fn main\n ctx\n-x\n+y");

        assert_eq!(f[1].filename, "new name.txt");
        assert_eq!(f[1].previous_filename.as_deref(), Some("old name.txt"));
        assert_eq!(f[1].patch, "");

        assert_eq!(f[2].filename, "gone.txt");
        assert_eq!(f[2].previous_filename, None);
        assert_eq!(f[2].patch, "@@ -1 +0,0 @@\n-bye");
    }

    #[test]
    fn multiple_hunks_and_no_newline_marker() {
        let input = "\
diff --git a/a.rs b/a.rs
--- a/a.rs
+++ b/a.rs
@@ -1,2 +1,2 @@
 a
-b
+c
@@ -10,2 +10,2 @@ fn foo
-old
\\ No newline at end of file
+new
\\ No newline at end of file
";
        let f = parse(input);
        assert_eq!(f.len(), 1);
        assert_eq!(
            f[0].patch,
            "@@ -1,2 +1,2 @@
 a
-b
+c
@@ -10,2 +10,2 @@ fn foo
-old
\\ No newline at end of file
+new
\\ No newline at end of file"
        );
    }

    #[test]
    fn new_file_and_empty_new_file() {
        let input = "\
diff --git a/new.txt b/new.txt
new file mode 100644
index 0000000..e69de29
--- /dev/null
+++ b/new.txt
@@ -0,0 +1 @@
+hello
diff --git a/empty.txt b/empty.txt
new file mode 100644
index 0000000..e69de29
";
        let f = parse(input);
        assert_eq!(f.len(), 2);
        assert_eq!(f[0].filename, "new.txt");
        assert_eq!(f[0].previous_filename, None);
        assert_eq!(f[0].patch, "@@ -0,0 +1 @@\n+hello");
        // No ---/+++ lines at all: filename must come from the header.
        assert_eq!(f[1].filename, "empty.txt");
        assert_eq!(f[1].patch, "");
    }

    #[test]
    fn mode_change_only() {
        let input = "\
diff --git a/run.sh b/run.sh
old mode 100644
new mode 100755
";
        let f = parse(input);
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].filename, "run.sh");
        assert_eq!(f[0].previous_filename, None);
        assert_eq!(f[0].patch, "");
    }

    #[test]
    fn rename_with_modification() {
        let input = "\
diff --git a/old.rs b/new.rs
similarity index 90%
rename from old.rs
rename to new.rs
index 1111111..2222222 100644
--- a/old.rs
+++ b/new.rs
@@ -1 +1 @@
-a
+b
";
        let f = parse(input);
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].filename, "new.rs");
        assert_eq!(f[0].previous_filename.as_deref(), Some("old.rs"));
        assert_eq!(f[0].patch, "@@ -1 +1 @@\n-a\n+b");
    }
}
