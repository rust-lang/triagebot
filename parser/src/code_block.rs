use pulldown_cmark::{Event, Parser, Tag};
use std::ops::Range;

pub struct ColorCodeBlocks {
    code: Vec<Range<usize>>,
}

impl ColorCodeBlocks {
    pub fn new(s: &str) -> ColorCodeBlocks {
        let mut code = Vec::new();
        let mut parser = Parser::new(s);
        let mut before_event = parser.get_offset();
        'outer: while let Some(event) = parser.next() {
            if let Event::Start(Tag::Code) | Event::Start(Tag::CodeBlock(_)) = event {
                let start = before_event;
                loop {
                    match parser.next() {
                        Some(Event::End(Tag::Code)) | Some(Event::End(Tag::CodeBlock(_))) => {
                            let end = parser.get_offset();
                            code.push(start..end);
                            break;
                        }
                        Some(_) => {}
                        None => break 'outer,
                    }
                }
            }
            before_event = parser.get_offset();
        }

        ColorCodeBlocks { code }
    }

    pub fn is_in_code(&self, pos: usize) -> bool {
        for range in &self.code {
            if range.start <= pos && pos <= range.end {
                return true;
            }
        }

        false
    }
}

#[cfg(test)]
#[derive(Debug, PartialEq, Eq)]
enum Code<'a> {
    Yes(&'a str),
    No(&'a str),
}

#[cfg(test)]
fn bodies(s: &str) -> Vec<Code<'_>> {
    let mut bodies = Vec::new();
    let cbs = ColorCodeBlocks::new(s);
    let mut previous = 0..0;
    for range in &cbs.code {
        let range = range.clone();
        if previous.end != range.start {
            bodies.push(Code::No(&s[previous.end..range.start]));
        }
        bodies.push(Code::Yes(&s[range.clone()]));
        previous = range.clone();
    }
    if let Some(range) = cbs.code.last() {
        if range.end != s.len() {
            bodies.push(Code::No(&s[range.end..]));
        }
    }
    bodies
}

#[test]
fn cbs_1() {
    assert_eq!(
        bodies("`hey you`bar me too"),
        [Code::Yes("`hey you`"), Code::No("bar me too")]
    );
}

#[test]
fn cbs_2() {
    assert_eq!(
        bodies("`hey you` <b>me too</b>"),
        [Code::Yes("`hey you`"), Code::No(" <b>me too</b>")]
    );
}

#[test]
fn cbs_3() {
    assert_eq!(
        bodies(r"`hey you\` <b>`me too</b>"),
        [Code::Yes(r"`hey you\`"), Code::No(" <b>`me too</b>")]
    );
}

#[test]
fn cbs_4() {
    assert_eq!(
        bodies(
            "
```language_spec
testing
```

nope
"
        ),
        [
            Code::No("\n"),
            Code::Yes("```language_spec\ntesting\n```\n"),
            Code::No("\nnope\n")
        ],
    );
}

#[test]
fn cbs_5() {
    assert_eq!(
        bodies(concat!(
            "
```     tag_after_space
testing
```",
            "           "
        )),
        [
            Code::No("\n"),
            Code::Yes("```     tag_after_space\ntesting\n```           "),
        ],
    );
}

#[test]
fn cbs_6() {
    assert_eq!(
        bodies(
            "
    this is indented
    this is indented too
"
        ),
        [
            Code::No("\n"),
            Code::Yes("    this is indented\n    this is indented too\n"),
        ],
    );
}

#[test]
fn cbs_7() {
    assert_eq!(
        bodies(
            "
```
testing unclosed
"
        ),
        [Code::No("\n"), Code::Yes("```\ntesting unclosed\n"),],
    );
}

#[test]
fn cbs_8() {
    assert_eq!(
        bodies("`one` not `two`"),
        [Code::Yes("`one`"), Code::No(" not "), Code::Yes("`two`")]
    );
}
