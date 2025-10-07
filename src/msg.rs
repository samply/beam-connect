use hyper::{Method, Uri, HeaderMap, StatusCode};
use serde::{Serialize, Deserialize};

#[derive(Serialize,Deserialize, Debug)]
pub(crate) struct HttpRequest {
    #[serde(with = "http_serde::method")]
    pub(crate) method: Method,
    #[serde(with = "http_serde::uri")]
    pub(crate) url: Uri,
    #[serde(with = "http_serde::header_map")]
    pub(crate) headers: HeaderMap,
    #[serde(with = "serde_base64")]
    pub(crate) body: Vec<u8>
}

#[derive(Serialize, Deserialize)]
pub(crate) struct HttpResponse {
    #[serde(with = "http_serde::status_code")]
    pub(crate) status: StatusCode,
    #[serde(with = "http_serde::header_map")]
    pub(crate) headers: HeaderMap,
    #[serde(with = "serde_base64")]
    pub(crate) body: Vec<u8>
}

impl std::fmt::Debug for HttpResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HttpResponse")
            .field("status", &self.status)
            .field("headers", &self.headers)
            .finish()
    }
}


// https://github.com/serde-rs/json/issues/360#issuecomment-330095360
pub mod serde_base64 {
    use serde::{Serializer, de, Deserialize, Deserializer};
    use openssl::base64;

    pub fn serialize<S>(bytes: &[u8], serializer: S) -> Result<S::Ok, S::Error>
        where S: Serializer
    {
        serializer.serialize_str(&base64::encode_block(bytes))
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Vec<u8>, D::Error>
        where D: Deserializer<'de>
    {
        base64::decode_block(<&str>::deserialize(deserializer)?).map_err(de::Error::custom)
    }
}
