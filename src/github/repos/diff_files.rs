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
    let (mut old, mut new) = parse_header(chunk.lines().next()?)?;
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
            old = parse_path(p)?;
        } else if let Some(p) = line.strip_prefix("rename to ") {
            new = parse_path(p)?;
        }

        offset += raw.len();
    }

    Some(FileDiff {
        previous_filename: (old != new).then_some(old),
        filename: new,
        patch: patch.to_string(),
    })
}

/// Parses the two paths in a `diff --git` header, where Git may quote either path.
fn parse_header(header: &str) -> Option<(String, String)> {
    if header.starts_with('"') {
        let (old, rest) = parse_quoted_path(header)?;
        let new = parse_path(rest.strip_prefix(' ')?)?;
        return Some((
            old.strip_prefix("a/")?.to_string(),
            new.strip_prefix("b/")?.to_string(),
        ));
    }

    if let Some(split) = header.find(" \"b/") {
        let old = header[..split].strip_prefix("a/")?.to_string();
        let new = parse_path(&header[split + 1..])?
            .strip_prefix("b/")?
            .to_string();
        return Some((old, new));
    }

    let header = header.strip_prefix("a/")?;
    let (old, new) = header.split_once(" b/")?;
    Some((old.to_string(), new.to_string()))
}

/// Decodes one complete Git path token and rejects trailing text after a quoted path.
fn parse_path(path: &str) -> Option<String> {
    if path.starts_with('"') {
        let (path, rest) = parse_quoted_path(path)?;
        return rest.is_empty().then_some(path);
    }
    Some(path.to_string())
}

/// Consumes one Git C-quoted path and returns its decoded value with the remaining input.
///
/// Octal escapes are decoded as bytes before constructing the string because Git uses them
/// for path bytes that are not emitted literally.
fn parse_quoted_path(path: &str) -> Option<(String, &str)> {
    let path = path.strip_prefix('"')?;
    let mut bytes = Vec::with_capacity(path.len());
    let mut chars = path.char_indices().peekable();

    while let Some((index, ch)) = chars.next() {
        match ch {
            '"' => {
                return Some((
                    String::from_utf8_lossy(&bytes).into_owned(),
                    &path[index + 1..],
                ));
            }
            '\\' => {
                let (_, escaped) = chars.next()?;
                let byte = match escaped {
                    'a' => 0x07,
                    'b' => 0x08,
                    't' => b'\t',
                    'n' => b'\n',
                    'v' => 0x0b,
                    'f' => 0x0c,
                    'r' => b'\r',
                    '\\' => b'\\',
                    '"' => b'"',
                    '0'..='7' => {
                        let mut value = escaped.to_digit(8)?;
                        for _ in 1..3 {
                            let Some((_, next)) = chars.peek().copied() else {
                                break;
                            };
                            let Some(digit) = next.to_digit(8) else {
                                break;
                            };
                            chars.next();
                            value = value * 8 + digit;
                        }
                        u8::try_from(value).ok()?
                    }
                    _ => return None,
                };
                bytes.push(byte);
            }
            _ => {
                let mut encoded = [0; 4];
                bytes.extend_from_slice(ch.encode_utf8(&mut encoded).as_bytes());
            }
        }
    }

    None
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

    #[test]
    fn quoted_non_ascii_path() {
        let input = r#"diff --git "a/\350\213\261\346\226\207\346\226\207\346\241\243.md" "b/\350\213\261\346\226\207\346\226\207\346\241\243.md"
index 1111111..2222222 100644
--- "a/\350\213\261\346\226\207\346\226\207\346\241\243.md"
+++ "b/\350\213\261\346\226\207\346\226\207\346\241\243.md"
@@ -1 +1 @@
-old
+new
"#;
        let f = parse(input);
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].filename, "英文文档.md");
        assert_eq!(f[0].previous_filename, None);
    }

    #[test]
    fn quoted_rename_paths() {
        let input = r#"diff --git "a/old \"draft\".txt" "b/new\\name.txt"
similarity index 100%
rename from "old \"draft\".txt"
rename to "new\\name.txt"
"#;
        let f = parse(input);
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].filename, "new\\name.txt");
        assert_eq!(f[0].previous_filename.as_deref(), Some("old \"draft\".txt"));
    }

    #[test]
    fn mixed_quoted_header() {
        let input = r#"diff --git a/plain.txt "b/\350\213\261\346\226\207.md"
similarity index 100%
rename from plain.txt
rename to "\350\213\261\346\226\207.md"
"#;
        let f = parse(input);
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].filename, "英文.md");
        assert_eq!(f[0].previous_filename.as_deref(), Some("plain.txt"));
    }
}
