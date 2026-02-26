use core::fmt;
use serde::{de, ser};

pub type Result<T> = core::result::Result<T, Error>;

#[derive(Debug)]
pub enum Error {
    Message(String),
    Eof,
    ExpectedColon,
    ExpectedOpenParen,
    ExpectedCloseParen,
    ExpectedOpenBrace,
    ExpectedCloseBrace,
    ExpectedOpenBracket,
    ExpectedCloseBracket,
    ExpectedComma,
    ExpectedValue,
    TrailingCharacters,
    InvalidEscape(char),
    InvalidNumber,
    InvalidBool,
    UnclosedString,
    UnclosedComment,
    UnclosedParen,
    UnclosedBracket,
    FieldCountMismatch { expected: usize, got: usize },
    InvalidUnicodeEscape,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Message(msg) => write!(f, "{}", msg),
            Error::Eof => write!(f, "unexpected end of input"),
            Error::ExpectedColon => write!(f, "expected ':'"),
            Error::ExpectedOpenParen => write!(f, "expected '('"),
            Error::ExpectedCloseParen => write!(f, "expected ')'"),
            Error::ExpectedOpenBrace => write!(f, "expected '{{'"),
            Error::ExpectedCloseBrace => write!(f, "expected '}}'"),
            Error::ExpectedOpenBracket => write!(f, "expected '['"),
            Error::ExpectedCloseBracket => write!(f, "expected ']'"),
            Error::ExpectedComma => write!(f, "expected ','"),
            Error::ExpectedValue => write!(f, "expected value"),
            Error::TrailingCharacters => write!(f, "trailing characters"),
            Error::InvalidEscape(c) => write!(f, "invalid escape: \\{}", c),
            Error::InvalidNumber => write!(f, "invalid number"),
            Error::InvalidBool => write!(f, "invalid bool"),
            Error::UnclosedString => write!(f, "unclosed string"),
            Error::UnclosedComment => write!(f, "unclosed comment"),
            Error::UnclosedParen => write!(f, "unclosed parenthesis"),
            Error::UnclosedBracket => write!(f, "unclosed bracket"),
            Error::FieldCountMismatch { expected, got } => {
                write!(f, "field count mismatch: expected {}, got {}", expected, got)
            }
            Error::InvalidUnicodeEscape => write!(f, "invalid unicode escape"),
        }
    }
}

impl std::error::Error for Error {}

impl ser::Error for Error {
    fn custom<T: fmt::Display>(msg: T) -> Self {
        Error::Message(msg.to_string())
    }
}

impl de::Error for Error {
    fn custom<T: fmt::Display>(msg: T) -> Self {
        Error::Message(msg.to_string())
    }
}
