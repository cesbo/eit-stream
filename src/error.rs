use config;
use epg;
use std::{result, io, fmt, num};

#[derive(Debug)]
pub enum Error {
    Custom(String),
    Config(config::Error),
    Epg(epg::Error),
    Io(io::Error),
    ParseInt(num::ParseIntError),
}

pub type Result<T> = result::Result<T, Error>;

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            Error::Custom(ref e) => write!(f, "{}", e),
            Error::Config(ref e) => config::Error::fmt(e, f),
            Error::Epg(ref e) => epg::Error::fmt(e, f),
            Error::Io(ref e) => io::Error::fmt(e, f),
            Error::ParseInt(ref e) => num::ParseIntError::fmt(e, f),
        }
    }
}

impl From<&str> for Error {
    fn from(s: &str) -> Self {
        Error::Custom(s.to_owned())
    }
}

impl From<String> for Error {
    fn from(s: String) -> Self {
        Error::Custom(s)
    }
}

impl From<config::Error> for Error {
    fn from(e: config::Error) -> Self {
        Error::Config(e)
    }
}

impl From<epg::Error> for Error {
    fn from(e: epg::Error) -> Self {
        Error::Epg(e)
    }
}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Error::Io(e)
    }
}

impl From<num::ParseIntError> for Error {
    fn from(e: num::ParseIntError) -> Self {
        Error::ParseInt(e)
    }
}
