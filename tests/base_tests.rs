use clap::__derive_refs::once_cell::sync::Lazy;
use hyper::{Response, Body, Request, header, http::HeaderValue, StatusCode, Client, client::HttpConnector};
use hyper_proxy::{Proxy, Intercept, ProxyConnector};
use hyper_tls::HttpsConnector;
use serde_json::Value;
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

pub async fn test_normal(scheme: &str) {
    let req = Request::get(format!("{scheme}://httpbin.org/anything")).body(Body::empty()).unwrap();
    let res = request(req).await;
    assert_eq!(res.status(), StatusCode::OK, "Could not make normal request via beam-connect");
}

pub async fn test_json(scheme: &str) {
    let json = serde_json::json!({
        "foo": [1, 2, {}],
        "bar": "foo",
        "foobar": false,
    });
    let req = Request::get(format!("{scheme}://httpbin.org/anything")).body(Body::from(serde_json::to_vec(&json).unwrap())).unwrap();
    let mut res = request(req).await;
    assert_eq!(res.status(), StatusCode::OK, "Could not make json request via beam-connect");
    let bytes = hyper::body::to_bytes(res.body_mut()).await.unwrap();
    let recieved: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(recieved.get("json").unwrap(), &json, "Json did not match");
}

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

test_http_and_https!{test_normal}
test_http_and_https!{test_json}
