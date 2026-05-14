use std::error;
use std::fmt;

#[derive(Debug)]
pub struct Error<'a> {
    pub input: &'a str,
    pub position: usize,
    pub source: Box<dyn error::Error + Send>,
}

impl PartialEq for Error<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.input == other.input && self.position == other.position
    }
}

impl error::Error for Error<'_> {
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        Some(&*self.source)
    }
}

impl Error<'_> {
    pub fn position(&self) -> usize {
        self.position
    }
}

impl fmt::Display for Error<'_> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        if self.input.is_empty() {
            writeln!(f, "{}", self.source)
        } else {
            const MARGIN: usize = 10;

            let start = self.position.saturating_sub(MARGIN);
            let end = std::cmp::min(self.input.len(), self.position + MARGIN);
            write!(
                f,
                "{} when parsing: {}[!]{}",
                self.source,
                &self.input[start..self.position],
                &self.input[self.position..end],
            )
        }
    }
}
