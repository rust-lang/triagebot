use pulldown_cmark::{Event, Parser, Tag, TagEnd};
use regex::{Captures, Regex, Replacer};
use std::{borrow::Cow, cell::LazyCell, ops::Range};

#[derive(Debug)]
pub struct IgnoreBlocks {
    ignore: Vec<Range<usize>>,
}

impl IgnoreBlocks {
    pub fn new(s: &str) -> IgnoreBlocks {
        let mut ignore = Vec::new();
        let mut parser = Parser::new(s).into_offset_iter();
        while let Some((event, range)) = parser.next() {
            if let Event::Start(Tag::CodeBlock(_)) = event {
                let start = range.start;
                while let Some((event, range)) = parser.next() {
                    if let Event::End(TagEnd::CodeBlock) = event {
                        ignore.push(start..range.end);
                        break;
                    }
                }
            } else if let Event::Start(Tag::BlockQuote(_)) = event {
                let start = range.start;
                let mut count = 1;
                while let Some((event, range)) = parser.next() {
                    if let Event::Start(Tag::BlockQuote(_)) = event {
                        count += 1;
                    } else if let Event::End(TagEnd::BlockQuote(_)) = event {
                        count -= 1;
                        if count == 0 {
                            ignore.push(start..range.end);
                            break;
                        }
                    }
                }
            } else if let Event::Start(Tag::HtmlBlock) = event {
                let start = range.start;
                while let Some((event, range)) = parser.next() {
                    if let Event::End(TagEnd::HtmlBlock) = event {
                        ignore.push(start..range.end);
                        break;
                    }
                }
            } else if let Event::InlineHtml(_) = event {
                ignore.push(range);
            } else if let Event::Code(_) = event {
                ignore.push(range);
            }
        }

        IgnoreBlocks { ignore }
    }

    pub fn overlaps_ignore(&self, region: Range<usize>) -> Option<Range<usize>> {
        for ignore in &self.ignore {
            // See https://stackoverflow.com/questions/3269434.
            // We have strictly < because `end` is not included in our ranges.
            if ignore.start < region.end && region.start < ignore.end {
                return Some(ignore.clone());
            }
        }
        None
    }
}

pub fn replace_all_outside_ignore_blocks<'h, R: Replacer>(
    re: &Regex,
    haystack: &'h str,
    mut replacement: R,
) -> Cow<'h, str> {
    let ignore_blocks = LazyCell::new(|| IgnoreBlocks::new(haystack));
    re.replace_all(haystack, |c: &Captures| {
        let m = c.get(0).unwrap();
        // This is the "custom" part, we check if the capture range does not
        // overlap with a ignore range, if it does we use the match as-is,
        // otherwise we apply the replacement.
        if ignore_blocks.overlaps_ignore(m.range()).is_some() {
            m.as_str().to_string()
        } else {
            let mut out = String::new();
            replacement.replace_append(c, &mut out);
            out
        }
    })
}

#[cfg(test)]
#[derive(Debug, PartialEq, Eq)]
enum Ignore<'a> {
    Yes(&'a str),
    No(&'a str),
}

#[cfg(test)]
fn bodies(s: &str) -> Vec<Ignore<'_>> {
    let mut bodies = Vec::new();
    let cbs = IgnoreBlocks::new(s);
    let mut previous = 0..0;
    for range in &cbs.ignore {
        let range = range.clone();
        if previous.end != range.start {
            assert!(cbs.overlaps_ignore(previous.end..range.start).is_none());
            bodies.push(Ignore::No(&s[previous.end..range.start]));
        }
        assert!(cbs.overlaps_ignore(range.clone()).is_some());
        bodies.push(Ignore::Yes(&s[range.clone()]));
        previous = range.clone();
    }
    if let Some(range) = cbs.ignore.last() {
        if range.end != s.len() {
            assert!(cbs.overlaps_ignore(range.end..s.len()).is_none());
            bodies.push(Ignore::No(&s[range.end..]));
        }
    }
    bodies
}

#[test]
fn cbs_1() {
    assert_eq!(
        bodies("`hey you`bar me too"),
        [Ignore::Yes("`hey you`"), Ignore::No("bar me too")]
    );
}

#[test]
fn cbs_2() {
    assert_eq!(
        bodies("`hey you` <b>me too</b>"),
        [
            Ignore::Yes("`hey you`"),
            Ignore::No(" "),
            Ignore::Yes("<b>"),
            Ignore::No("me too"),
            Ignore::Yes("</b>")
        ]
    );
}

#[test]
fn cbs_3() {
    assert_eq!(
        bodies(r"`hey you\` <b>`me too</b>"),
        [
            Ignore::Yes("`hey you\\`"),
            Ignore::No(" "),
            Ignore::Yes("<b>"),
            Ignore::No("`me too"),
            Ignore::Yes("</b>")
        ]
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
            Ignore::No("\n"),
            Ignore::Yes("```language_spec\ntesting\n```"),
            Ignore::No("\n\nnope\n")
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
            Ignore::No("\n"),
            Ignore::Yes("```     tag_after_space\ntesting\n```           "),
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
            Ignore::No("\n    "),
            Ignore::Yes("this is indented\n    this is indented too\n"),
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
        [Ignore::No("\n"), Ignore::Yes("```\ntesting unclosed\n"),],
    );
}

#[test]
fn cbs_8() {
    assert_eq!(
        bodies("`one` not `two`"),
        [
            Ignore::Yes("`one`"),
            Ignore::No(" not "),
            Ignore::Yes("`two`")
        ]
    );
}

#[test]
fn cbs_9() {
    assert_eq!(
        bodies(
            "
some text
> testing citations
still in citation

more text
"
        ),
        [
            Ignore::No("\nsome text\n"),
            Ignore::Yes("> testing citations\nstill in citation\n"),
            Ignore::No("\nmore text\n")
        ],
    );
}

#[test]
fn cbs_10() {
    assert_eq!(
        bodies(
            "
# abc

> multiline
> citation

lorem ipsum
"
        ),
        [
            Ignore::No("\n# abc\n\n"),
            Ignore::Yes("> multiline\n> citation\n"),
            Ignore::No("\nlorem ipsum\n")
        ],
    );
}

#[test]
fn cbs_11() {
    assert_eq!(
        bodies(
            "
> some
> > nested
> citations
"
        ),
        [
            Ignore::No("\n"),
            Ignore::Yes("> some\n> > nested\n> citations\n"),
        ],
    );
}

#[test]
fn cbs_12() {
    assert_eq!(
        bodies(
            "
Test

<!-- Test -->
<!--
This is an HTML comment.
-->
"
        ),
        [
            Ignore::No("\nTest\n\n"),
            Ignore::Yes("<!-- Test -->\n"),
            Ignore::Yes("<!--\nThis is an HTML comment.\n-->\n")
        ],
    );
}

#[test]
fn cbs_13() {
    assert_eq!(
        bodies("<!-- q -->\n@rustbot label +F-trait_upcasting"),
        [
            Ignore::Yes("<!-- q -->\n"),
            Ignore::No("@rustbot label +F-trait_upcasting")
        ],
    );
}
