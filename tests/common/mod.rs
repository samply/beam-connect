
use clap::__derive_refs::once_cell::sync::Lazy;
use hyper::{Response, Body, Request, header, http::HeaderValue, Client, client::HttpConnector};
use hyper_proxy::{Proxy, Intercept, ProxyConnector};
use hyper_tls::HttpsConnector;
use tokio_native_tls::native_tls::TlsConnector;

const CLIENT: Lazy<Client<ProxyConnector<HttpsConnector<HttpConnector>>>> = Lazy::new(|| {
    let tls = TlsConnector::builder()
        .danger_accept_invalid_certs(true)
        .danger_accept_invalid_hostnames(true)
        .build()
        .unwrap();
    let connector = HttpsConnector::from((HttpConnector::new(), tls.clone().into()));
    let proxy = Proxy::new(Intercept::All, "http://localhost:8062".parse().unwrap());
    let mut proxy_con = ProxyConnector::from_proxy(connector, proxy).unwrap();
    proxy_con.set_tls(Some(tls));
    Client::builder().build(proxy_con)
});

pub async fn request(mut req: Request<Body>) -> Response<Body> {
    req.headers_mut().append(header::PROXY_AUTHORIZATION, HeaderValue::from_static("ApiKey app1.proxy1.broker App1Secret"));
    CLIENT.request(req).await.unwrap()
}

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
