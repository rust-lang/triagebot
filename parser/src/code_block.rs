use pulldown_cmark::{Event, Parser, Tag};
use std::ops::Range;

#[derive(Debug)]
pub struct ColorCodeBlocks {
    code: Vec<Range<usize>>,
}

impl ColorCodeBlocks {
    pub fn new(s: &str) -> ColorCodeBlocks {
        let mut code = Vec::new();
        let mut parser = Parser::new(s).into_offset_iter();
        while let Some((event, range)) = parser.next() {
            if let Event::Start(Tag::CodeBlock(_)) = event {
                let start = range.start;
                while let Some((event, range)) = parser.next() {
                    if let Event::End(Tag::CodeBlock(_)) = event {
                        code.push(start..range.end);
                        break;
                    }
                }
            } else if let Event::Code(_) = event {
                code.push(range);
            }
        }

        ColorCodeBlocks { code }
    }

    pub fn overlaps_code(&self, region: Range<usize>) -> Option<Range<usize>> {
        for code in &self.code {
            // See https://stackoverflow.com/questions/3269434.
            if code.start <= region.end && region.start <= code.end {
                return Some(code.clone());
            }
        }
        None
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
            Code::Yes("```language_spec\ntesting\n```"),
            Code::No("\n\nnope\n")
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
            Code::No("\n    "),
            Code::Yes("this is indented\n    this is indented too\n"),
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
