use pulldown_cmark::{Event, Options, Parser, TagEnd};

pub fn strip_markdown(text: &str) -> String {
    let mut buffer = String::new();

    let mut parser = Parser::new_ext(
        text,
        Options::ENABLE_TABLES
            | Options::ENABLE_GFM
            | Options::ENABLE_MATH
            | Options::ENABLE_STRIKETHROUGH
            | Options::ENABLE_FOOTNOTES
            | Options::ENABLE_HEADING_ATTRIBUTES,
    )
    .into_iter();

    while let Some(event) = parser.next() {
        match event {
            // Text and inline code are the content we want to keep
            Event::Text(t) | Event::Code(t) => {
                let stripped = t.replace("@", ""); // Strip mentions as well
                buffer.push_str(&stripped);
            }
            // Add a newline when a block-level element ends to maintain spacing
            Event::End(tag) => {
                if is_block_tag(&tag) {
                    buffer.push('\n');
                }
            }
            _ => {}
        }
    }

    buffer.trim().to_string()
}

fn is_block_tag(tag: &TagEnd) -> bool {
    matches!(
        tag,
        TagEnd::Paragraph | TagEnd::Heading { .. } | TagEnd::Item | TagEnd::CodeBlock
    )
}

#[test]
fn basic_formatting() {
    let input = "This is **bold**, *italic* and ~~strikethrough~~ text.";
    let expected = "This is bold, italic and strikethrough text.";
    assert_eq!(strip_markdown(input), expected);
}

#[test]
fn links_and_images() {
    let input = "Check out [Google](https://google.com) and this ![alt text](image.png).";
    let expected = "Check out Google and this alt text.";
    assert_eq!(strip_markdown(input), expected);
}

#[test]
fn headers_and_lists() {
    let input = "# Title\n- Item 1\n- Item 2";
    let expected = "Title\nItem 1\nItem 2";
    assert_eq!(strip_markdown(input), expected);
}

#[test]
fn inline_code() {
    let input = "Use the `fn main()` function.";
    let expected = "Use the fn main() function.";
    assert_eq!(strip_markdown(input), expected);
}

#[test]
fn test_pings() {
    let input = "let's ping @foo";
    let expected = "let's ping foo";
    assert_eq!(strip_markdown(input), expected);
}

#[test]
fn empty_and_whitespace() {
    assert_eq!(strip_markdown(""), "");
    assert_eq!(strip_markdown("   "), "");
}
