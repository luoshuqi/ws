use std::borrow::Cow;
use std::error::Error as StdError;
use std::fmt::{Display, Formatter};
use std::io;
use std::str::Utf8Error;

macro_rules! error {
    ($error:literal) => {
        return Err(crate::error::Error::Custom(std::borrow::Cow::Borrowed($error)))
    };
    ($fmt:literal, $($args:tt)+) => {
        return Err(crate::error::Error::Custom(std::borrow::Cow::Owned(format!($fmt, $($args)*))))
    };
}

#[derive(Debug)]
pub enum Error {
    Io(io::Error),
    Utf8(Utf8Error),
    Custom(Cow<'static, str>),
}

impl From<io::Error> for Error {
    #[inline]
    fn from(err: io::Error) -> Self {
        Self::Io(err)
    }
}

impl From<Utf8Error> for Error {
    #[inline]
    fn from(err: Utf8Error) -> Self {
        Self::Utf8(err)
    }
}

impl Display for Error {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(err) => Display::fmt(err, f),
            Self::Utf8(err) => Display::fmt(err, f),
            Self::Custom(err) => Display::fmt(err, f),
        }
    }
}

impl StdError for Error {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match self {
            Self::Io(err) => Some(err),
            Self::Utf8(err) => Some(err),
            _ => None,
        }
    }
}
