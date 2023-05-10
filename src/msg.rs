use hyper::{Method, Uri, HeaderMap, StatusCode};
use serde::{Serialize, Deserialize};
use shared::MsgTaskRequest;
use crate::errors::BeamConnectError;

#[derive(Serialize,Deserialize, Debug)]
pub(crate) struct HttpRequest {
    #[serde(with = "hyper_serde")]
    pub(crate) method: Method,
    #[serde(with = "hyper_serde")]
    pub(crate) url: Uri,
    #[serde(with = "hyper_serde")]
    pub(crate) headers: HeaderMap,
    #[serde(with = "shared::serde_helpers::serde_base64")]
    pub(crate) body: Vec<u8>
}

#[derive(Serialize,Deserialize)]
pub(crate) struct HttpResponse {
    #[serde(with = "hyper_serde")]
    pub(crate) status: StatusCode,
    #[serde(with = "hyper_serde")]
    pub(crate) headers: HeaderMap,
    #[serde(with = "shared::serde_helpers::serde_base64")]
    pub(crate) body: Vec<u8>
}

pub(crate) trait IsValidHttpTask {
    fn http_request(&self) -> Result<HttpRequest,BeamConnectError>;
}


impl IsValidHttpTask for MsgTaskRequest {
    fn http_request(&self) -> Result<HttpRequest,BeamConnectError> {
        let req_struct: HttpRequest = serde_json::from_str(self.body.body.as_ref().ok_or(BeamConnectError::ReplyInvalid("MsgTaskRequest had no content.".to_string()))?)?;
        if false { // TODO
            return Err(BeamConnectError::IdNotAuthorizedToAccessUrl(self.from.clone(), req_struct.url));
        }
        Ok(req_struct)
    }
}
