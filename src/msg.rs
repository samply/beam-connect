use hyper::{Method, Uri, HeaderMap};
use serde::{Serialize, Deserialize};

#[derive(Serialize)]
pub(crate) struct HttpRequest {
    #[serde(deserialize_with = "hyper_serde::deserialize", serialize_with = "hyper_serde::serialize")]
    pub(crate) method: Method,
    #[serde(deserialize_with = "hyper_serde::deserialize", serialize_with = "hyper_serde::serialize")]
    pub(crate) url: Uri,
    #[serde(deserialize_with = "hyper_serde::deserialize", serialize_with = "hyper_serde::serialize")]
    pub(crate) headers: HeaderMap,
    pub(crate) body: String
}

#[derive(Deserialize)]
pub(crate) struct HttpResponse {
    
}
