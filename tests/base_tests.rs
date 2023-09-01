use hyper::{StatusCode, header};
use reqwest::Client;
use serde_json::{Value, json};

mod common;
use common::TEST_CLIENT;

pub async fn test_normal(scheme: &str) {
    let res = TEST_CLIENT.get(format!("{scheme}://echo-get?foo1=bar1&foo2=bar2")).send().await.unwrap();
    assert_eq!(res.status(), StatusCode::OK, "Could not make normal request via beam-connect");
    let received: Value = res.json().await.unwrap();
    assert_eq!(received.get("query").unwrap(), &json!({
        "foo1": "bar1",
        "foo2": "bar2"
    }), "Json did not match");
    assert_eq!(received.get("path"), Some(&json!("/get/")))
}

pub async fn test_json(scheme: &str) {
    let json = serde_json::json!({
        "foo": [1, 2, {}],
        "bar": "foo",
        "foobar": false,
    });
    let res = TEST_CLIENT.post(format!("{scheme}://echo-post")).json(&json).send().await.unwrap();
    assert_eq!(res.status(), StatusCode::OK, "Could not make json request via beam-connect");
    let received: Value = res.json().await.unwrap();
    assert_eq!(received.get("body").and_then(Value::as_str).and_then(|s| serde_json::from_str::<Value>(s).ok()).unwrap(), json, "Json did not match");
    assert_eq!(received.get("path"), Some(&json!("/post/")))
}

#[tokio::test] 
pub async fn test_empty_authority() { // Test only works with http, so no macro usage
    let client = Client::builder()
        .danger_accept_invalid_certs(true)
        .build()
        .unwrap();
    let res = client.get("http://localhost:8062")
        .query(&[("foo1","bar1"),("foo2","bar2")])
        .header(header::HOST, "echo-get")
        .header(header::PROXY_AUTHORIZATION, "ApiKey app1.proxy1.broker App1Secret")
        .send().await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK, "Could not make normal request via beam-connect");
    let received: Value = res.json().await.unwrap();
    assert_eq!(received.get("query").unwrap(), &json!({
        "foo1": "bar1",
        "foo2": "bar2"
    }), "Json did not match");
    assert_eq!(received.get("path"), Some(&json!("/get/")))
}



test_http_and_https!{test_normal}
test_http_and_https!{test_json}

#[cfg(feature = "sockets")]
#[cfg(test)]
mod socket_tests {
    use futures_util::{SinkExt, StreamExt};
    use hyper::{header, StatusCode};
    use tokio_tungstenite::{tungstenite::{protocol::Role, Message}, WebSocketStream};

    use crate::common::TEST_CLIENT;
    
    #[tokio::test]
    pub async fn test_ws() {
        let resp = TEST_CLIENT.get(format!("http://ws-echo"))
            .header(header::UPGRADE, "websocket")
            .header(header::CONNECTION, "upgrade")
            .header(header::SEC_WEBSOCKET_VERSION, "13")
            .header(header::SEC_WEBSOCKET_KEY, "h/QU7Qscq6DfSTu9aP78HQ==")
            .send().await.unwrap();
        assert_eq!(resp.status(), StatusCode::SWITCHING_PROTOCOLS);
        let socket = resp.upgrade().await.unwrap();
        let mut stream = WebSocketStream::from_raw_socket(socket, Role::Client, None).await;
        let _server_hello = stream.next().await.unwrap().unwrap();
        stream.send(Message::Text("Hello World".to_string())).await.unwrap();
        let res = stream.next().await.unwrap().unwrap();
        assert_eq!(res, Message::Text("Hello World".to_string()));
        stream.close(None).await.unwrap();
    }
}
