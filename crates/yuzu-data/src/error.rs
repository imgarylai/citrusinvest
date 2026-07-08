use std::fmt;

#[derive(Debug)]
pub enum DataError {
    Io(String),
    Parse(String),
}

impl fmt::Display for DataError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DataError::Io(m) => write!(f, "io error: {m}"),
            DataError::Parse(m) => write!(f, "parse error: {m}"),
        }
    }
}

impl std::error::Error for DataError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_messages() {
        assert!(DataError::Io("x".into()).to_string().contains("io"));
        assert!(DataError::Parse("y".into()).to_string().contains("parse"));
    }
}
