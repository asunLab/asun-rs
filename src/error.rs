//! Error and result types for ASUN encoding and decoding.

use core::fmt;

/// A specialized [`Result`](core::result::Result) for ASUN operations.
pub type Result<T> = core::result::Result<T, Error>;

/// An error produced while encoding or (more commonly) decoding ASUN data.
///
/// Most variants are parse errors carrying the specific token that was expected
/// at the point of failure; [`Error::Message`] wraps free-form messages produced
/// deeper in the decode path. The [`Display`](fmt::Display) impl renders a short,
/// human-readable description of each variant.
///
/// The payload types are deliberately narrow (`Box<str>` rather than `String`,
/// `u32` rather than `usize`) to keep `Result<T>` small: every scalar decode
/// primitive returns one, so the enum's size is copied on every field read.
#[derive(Debug)]
pub enum Error {
    Message(Box<str>),
    Eof,
    ExpectedColon,
    ExpectedOpenParen,
    ExpectedCloseParen,
    ExpectedOpenBrace,
    ExpectedCloseBrace,
    ExpectedOpenBracket,
    ExpectedCloseBracket,
    ExpectedOpenAngle,
    ExpectedCloseAngle,
    ExpectedComma,
    ExpectedValue,
    TrailingCharacters,
    /// Binary input decoded a value successfully but still contains bytes.
    TrailingBytes,
    InvalidEscape(char),
    InvalidNumber,
    /// An integer literal was valid but out of range for the target type.
    IntegerOutOfRange,
    InvalidBool,
    /// A binary tag was not one of the values defined by the wire format.
    InvalidTag,
    InvalidUtf8,
    /// A LEB128 value uses more than 64 bits, or is padded with redundant
    /// bytes (each value has exactly one valid encoding).
    VarintOverflow,
    /// An attacker-controlled sequence length exceeded the configured limit.
    SequenceTooLong,
    /// A fallible reserve failed while decoding untrusted input.
    AllocationFailed,
    UnclosedString,
    UnclosedComment,
    UnclosedParen,
    UnclosedBracket,
    FieldCountMismatch {
        expected: u32,
        got: u32,
    },
    InvalidUnicodeEscape,
    /// Input nesting exceeded [`crate::decode::MAX_DEPTH`].
    DepthLimitExceeded,
    /// The document holds no value (only whitespace / comments).
    EmptyDocument,
    /// A bare `(...)` tuple at the top level; tuples need a schema.
    BareTuple,
    /// A byte that cannot start or continue the expected token.
    UnexpectedChar(char),
    /// A null slot (empty or `null`) decoded into a non-`Option` target.
    NullNotAllowed,
    /// A scalar hint (`@int` …) the value or the target type contradicts.
    HintMismatch,
    /// The same field name appears twice in one schema.
    DuplicateField,
    /// A bare field name outside `[A-Za-z0-9_]+`, or an empty one.
    InvalidFieldName,
    /// A raw control character (U+0000–U+001F) inside a quoted string.
    ControlCharInString,
    /// A float literal whose magnitude overflows the target type.
    FloatOverflow,
}

impl Error {
    #[cold]
    #[inline(never)]
    pub(crate) fn msg(s: impl Into<Box<str>>) -> Self {
        Error::Message(s.into())
    }
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
            Error::ExpectedOpenAngle => write!(f, "expected '<'"),
            Error::ExpectedCloseAngle => write!(f, "expected '>'"),
            Error::ExpectedComma => write!(f, "expected ','"),
            Error::ExpectedValue => write!(f, "expected value"),
            Error::TrailingCharacters => write!(f, "trailing characters"),
            Error::TrailingBytes => write!(f, "trailing bytes"),
            Error::InvalidEscape(c) => write!(f, "invalid escape: \\{}", c),
            Error::InvalidNumber => write!(f, "invalid number"),
            Error::IntegerOutOfRange => write!(f, "integer out of range for target type"),
            Error::InvalidBool => write!(f, "invalid bool"),
            Error::InvalidTag => write!(f, "invalid binary tag"),
            Error::InvalidUtf8 => write!(f, "invalid utf-8"),
            Error::VarintOverflow => write!(f, "malformed or overflowing varint"),
            Error::SequenceTooLong => write!(f, "sequence exceeds decode limit"),
            Error::AllocationFailed => write!(f, "allocation failed"),
            Error::UnclosedString => write!(f, "unclosed string"),
            Error::UnclosedComment => write!(f, "unclosed comment"),
            Error::UnclosedParen => write!(f, "unclosed parenthesis"),
            Error::UnclosedBracket => write!(f, "unclosed bracket"),
            Error::FieldCountMismatch { expected, got } => {
                write!(
                    f,
                    "field count mismatch: expected {}, got {}",
                    expected, got
                )
            }
            Error::InvalidUnicodeEscape => write!(f, "invalid unicode escape"),
            Error::DepthLimitExceeded => write!(f, "input nesting exceeds the depth limit"),
            Error::EmptyDocument => write!(f, "empty document"),
            Error::BareTuple => write!(f, "a top-level tuple needs a schema"),
            Error::UnexpectedChar(c) => write!(f, "unexpected character {:?}", c),
            Error::NullNotAllowed => write!(f, "null value for a non-optional target"),
            Error::HintMismatch => write!(f, "value does not match its schema type hint"),
            Error::DuplicateField => write!(f, "duplicate field name in schema"),
            Error::InvalidFieldName => write!(f, "invalid bare field name"),
            Error::ControlCharInString => write!(f, "raw control character in quoted string"),
            Error::FloatOverflow => write!(f, "float literal out of range"),
        }
    }
}

impl std::error::Error for Error {}

#[cfg(test)]
mod tests {
    use super::Error;

    // `Result<T>` is returned by every scalar decode primitive; keep it small.
    #[test]
    fn error_stays_small() {
        assert!(
            core::mem::size_of::<Error>() <= 24,
            "Error grew to {} bytes",
            core::mem::size_of::<Error>()
        );
        assert!(core::mem::size_of::<super::Result<i64>>() <= 32);
    }
}
