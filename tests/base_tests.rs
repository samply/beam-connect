use hyper::{Body, Request, StatusCode};
use serde_json::{Value, json};

mod common;
use common::*;

pub async fn test_normal(scheme: &str) {
    let req = Request::get(format!("{scheme}://postman-get?foo1=bar1&foo2=bar2")).body(Body::empty()).unwrap();
    let mut res = request(req).await;
    assert_eq!(res.status(), StatusCode::OK, "Could not make normal request via beam-connect");
    let bytes = hyper::body::to_bytes(res.body_mut()).await.unwrap();
    let received: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(received.get("args").unwrap(), &json!({
        "foo1": "bar1",
        "foo2": "bar2"
    }), "Json did not match");
}

pub async fn test_json(scheme: &str) {
    let json = serde_json::json!({
        "foo": [1, 2, {}],
        "bar": "foo",
        "foobar": false,
    });
    let req = Request::post(format!("{scheme}://postman-post")).body(Body::from(serde_json::to_vec(&json).unwrap())).unwrap();
    let mut res = request(req).await;
    assert_eq!(res.status(), StatusCode::OK, "Could not make json request via beam-connect");
    let bytes = hyper::body::to_bytes(res.body_mut()).await.unwrap();
    let received: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(received.get("json").unwrap(), &json, "Json did not match");
}


test_http_and_https!{test_normal}
test_http_and_https!{test_json}

#[cfg(feature = "sockets")]
#[cfg(test)]
mod socket_tests {
    use super::request;
    use futures_util::{SinkExt, StreamExt};
    use hyper::{Request, Body, header, StatusCode};
    use tokio_tungstenite::{tungstenite::{protocol::Role, Message}, WebSocketStream};
    
    #[tokio::test]
    pub async fn test_ws() {
        let resp = request(Request::get(format!("http://echo"))
            .header(header::UPGRADE, "websocket")
            .header(header::CONNECTION, "upgrade")
            .header(header::SEC_WEBSOCKET_VERSION, "13")
            .header(header::SEC_WEBSOCKET_KEY, "h/QU7Qscq6DfSTu9aP78HQ==")
            .body(Body::empty()).unwrap()).await;
        assert_eq!(resp.status(), StatusCode::SWITCHING_PROTOCOLS);
        let socket = hyper::upgrade::on(resp).await.unwrap();
        let mut stream = WebSocketStream::from_raw_socket(socket, Role::Client, None).await;
        let _server_hello = stream.next().await.unwrap().unwrap();
        stream.send(Message::Text("Hello World".to_string())).await.unwrap();
        let res = stream.next().await.unwrap().unwrap();
        assert_eq!(res, Message::Text("Hello World".to_string()));
        stream.close(None).await.unwrap();
    }
}
