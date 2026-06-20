// Copied from https://github.com/rust-lang/rust/blob/693b3e4c6e4e686cb9878c1722ad26858b5f1d2a/compiler/rustc_ast/src/util/unicode.rs

#[inline]
pub fn contains_text_flow_control_chars(s: &str) -> bool {
    // Char   - UTF-8
    // U+202A - E2 80 AA
    // U+202B - E2 80 AB
    // U+202C - E2 80 AC
    // U+202D - E2 80 AD
    // U+202E - E2 80 AE
    // U+2066 - E2 81 A6
    // U+2067 - E2 81 A7
    // U+2068 - E2 81 A8
    // U+2069 - E2 81 A9
    let mut bytes = s.as_bytes();
    loop {
        match memchr::memchr(0xE2, bytes) {
            Some(idx) => {
                // bytes are valid UTF-8 -> E2 must be followed by two bytes
                let ch = &bytes[idx..idx + 3];
                match ch {
                    [_, 0x80, 0xAA..=0xAE] | [_, 0x81, 0xA6..=0xA9] => break true,
                    _ => {}
                }
                bytes = &bytes[idx + 3..];
            }
            None => {
                break false;
            }
        }
    }
}
