use rcgen::Error as RcgenError;
use rustls::Error as RustlsError;
use std::fmt;

#[derive(Debug)]
pub enum UtilsError {
    RustlsError(RustlsError),
    RcgenEror(RcgenError),
    StdIoError(std::io::Error),
    UnknownType(String),
}

impl From<RustlsError> for UtilsError {
    fn from(err: RustlsError) -> Self {
        UtilsError::RustlsError(err)
    }
}

impl From<RcgenError> for UtilsError {
    fn from(err: RcgenError) -> Self {
        UtilsError::RcgenEror(err)
    }
}

impl From<std::io::Error> for UtilsError {
    fn from(err: std::io::Error) -> Self {
        UtilsError::StdIoError(err)
    }
}

impl fmt::Display for UtilsError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            UtilsError::RcgenEror(err) => {
                write!(f, "Rcgen error: {}", err)
            }
            UtilsError::RustlsError(err) => {
                write!(f, "Rustls error: {}", err)
            }
            UtilsError::StdIoError(msg) => {
                write!(f, "IO error: {}", msg)
            }
            UtilsError::UnknownType(msg) => {
                write!(f, "Unknown listener type: {}", msg)
            }
        }
    }
}
