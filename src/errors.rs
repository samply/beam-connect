use std::{str::Utf8Error, string::FromUtf8Error};

use hyper::Uri;
use shared::beam_id::AppOrProxyId;
use thiserror::Error;

#[derive(Error,Debug)]
pub(crate) enum HttPusherError {
    #[error("Proxy rejected our authorization")]
    ProxyRejectedAuthorization,
    #[error("Unable to communicate with Proxy: {0}")]
    ProxyHyperError(hyper::Error),
    #[error("Unable to communicate with Proxy: {0}")]
    ProxyOtherError(String),
    #[error("Constructing HTTP request failed: {0}")]
    HyperBuildError(hyper::http::Error),
    #[error("Error in (de-)serialization from/to JSON: {0}")]
    SerdeError(#[from] serde_json::Error),
    #[error("AppId {0} is not authorized to access URL {1}")]
    IdNotAuthorizedToAccessUrl(AppOrProxyId, Uri),
    #[error("Unable to communicate with target host: {0}")]
    CommunicationWithTargetFailed(String),
    #[error("Unable to fetch reply from target host: {0}")]
    FailedToReadTargetsReply(hyper::Error),
    #[error("Response was not valid UTF-8: {0}")]
    ResponseNotValidUtf8String(#[from] FromUtf8Error),
    #[error("Reply invalid: {0}")]
    ReplyInvalid(String),
    // #[error("Unable to build reply: {0}")]
    // BuildReplyFailed(hyper::http::Error)
}

