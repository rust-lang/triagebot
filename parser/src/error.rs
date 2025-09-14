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
        let space = 10;
        let end = std::cmp::min(self.input.len(), self.position + space);
        write!(
            f,
            "...'{}' | error: {} at >| '{}'...",
            &self.input[self.position.saturating_sub(space)..self.position],
            self.source,
            &self.input[self.position..end],
        )
    }
}
