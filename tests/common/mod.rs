
use clap::__derive_refs::once_cell::sync::Lazy;
use hyper::{header, http::HeaderValue, HeaderMap};
use reqwest::{Client, Proxy};

pub static TEST_CLIENT: Lazy<Client> = Lazy::new(|| {
    Client::builder()
        .danger_accept_invalid_certs(true)
        .proxy(Proxy::all("http://localhost:8062").unwrap())
        .default_headers({
            let mut headers = HeaderMap::new();
            headers.append(header::PROXY_AUTHORIZATION, HeaderValue::from_static("ApiKey app1.proxy1.broker App1Secret"));
            headers
        })
        .build()
        .unwrap()
});

#[macro_export]
macro_rules! test_http_and_https {
    ($name:ident) => {
        paste::paste! {
            #[tokio::test]
            async fn [<$name _http>]() {
                $name("http").await
            }

            #[tokio::test]
            async fn [<$name _https>]() {
                $name("https").await
            }
        }
    };
}
