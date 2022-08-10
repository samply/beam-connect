use std::{string::FromUtf8Error, error::Error, fmt::Display};

use hyper::{StatusCode, header::ToStrError, http::uri::Authority};
use log::error;
use shared::errors::SamplyBeamError;

#[derive(Debug)]
pub(crate) struct MyStatusCode {
    pub(crate) code: StatusCode
}

impl From<StatusCode> for MyStatusCode {
    fn from(c: StatusCode) -> Self {
        Self { code: c }
    }
}

impl From<ToStrError> for MyStatusCode {
    fn from(_: ToStrError) -> Self {
        Self { code: StatusCode::BAD_REQUEST }
    }
}

impl From<MyStatusCode> for StatusCode {
    fn from(e: MyStatusCode) -> Self {
        e.code
    }
}

impl From<SamplyBeamError> for MyStatusCode {
    fn from(e: SamplyBeamError) -> Self {
        let code = match e {
            SamplyBeamError::InvalidBeamId(e) => {
                error!("{e}");
                StatusCode::BAD_REQUEST
            },
            _ => StatusCode::NOT_IMPLEMENTED,
        };
        Self { code }
    }
}

impl From<FromUtf8Error> for MyStatusCode {
    fn from(_: FromUtf8Error) -> Self {
        Self { code: StatusCode::UNPROCESSABLE_ENTITY }
    }
}

impl From<serde_json::Error> for MyStatusCode {
    fn from(_: serde_json::Error) -> Self {
        Self { code: StatusCode::UNPROCESSABLE_ENTITY }
    }
}

impl Display for MyStatusCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.code)
    }
}

impl Error for MyStatusCode {}
