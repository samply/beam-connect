use std::{net::SocketAddr, str::{FromStr, Utf8Error}, convert::Infallible, string::FromUtf8Error, error::Error, fmt::Display};

// use axum::{Router, routing::{post, any}, Server, http::Request, response::{Response, IntoResponse}};
use hyper::{body, Body, StatusCode, service::{Service, service_fn, make_service_fn}, Request, Response, Server, header::{HeaderName, self, ToStrError}};
use log::{info, error};
use msg::HttpRequest;
use shared::{beam_id::{AppId, BeamId}, MsgTaskRequest, errors::SamplyBeamError};
use tower::{ServiceBuilder, Layer};
use tower_http::trace::TraceLayer;
// use tower::{make::Shared, ServiceExt};

mod msg;

#[tokio::main]
async fn main() {
    pretty_env_logger::init();
    let listen = SocketAddr::from_str("0.0.0.0:8082").unwrap();

    // And a MakeService to handle each connection...
    let make_service = make_service_fn(|_conn| async {
        Ok::<_, Infallible>(service_fn(handler_http))
    });

    // Then bind and serve...
    let server =
        Server::bind(&listen)
        .serve(make_service)
        .with_graceful_shutdown(shutdown_signal());

    // And run forever...
    if let Err(e) = server.await {
        eprintln!("server error: {}", e);
    }
}

async fn handler_http(
    // AuthenticatedApp(app_id): AuthenticatedApp,
    mut req: Request<Body>
) -> Result<Response<Body>,MyStatusCode> {
    let config = &shared::config::CONFIG_PROXY;
    let client = hyper::Client::builder().build_http();
    let method = req.method().to_owned();
    let uri = req.uri().to_owned();
    let headers = req.headers_mut();
    
    info!("{method} {uri} {:?}", headers);

    let auth = 
        headers.remove(header::PROXY_AUTHORIZATION)
            .ok_or(StatusCode::PROXY_AUTHENTICATION_REQUIRED)?;
    if let Some(_) = headers.remove(header::AUTHORIZATION) {
        return Err(StatusCode::CONFLICT.into());
    }
    let target = AppId::new(
        headers.remove("X-Samply-TargetApp")
            .ok_or(StatusCode::BAD_REQUEST)?
            .to_str()?)?;

    let my_id = format!("pusher.{}", config.proxy_id);
    let msg = req_to_task(req, &AppId::new("pusher1.proxy23.broker.samply.de").unwrap(), &target).await?;

    let req_to_proxy = Request::builder()
        .uri(&config.broker_uri)
        .header(header::AUTHORIZATION, auth)
        .body(serde_json::to_string(&msg).unwrap())
        .map_err(|e| StatusCode::INTERNAL_SERVER_ERROR)?;

    let resp = client.request(req_to_proxy).await
        .map_err(|_| StatusCode::BAD_GATEWAY)?;
    let resp = Response::builder()
        .status(StatusCode::NOT_IMPLEMENTED)
        .body(body::Body::empty())
        .unwrap();

    Ok(resp)
}

async fn req_to_task(req: Request<Body>, my_id: &AppId, target_id: &AppId) -> Result<MsgTaskRequest,MyStatusCode> {
    let method = req.method().clone();
    let url = req.uri().clone();
    let headers = req.headers().clone();
    let body = body::to_bytes(req).await
        .map_err(|e| {
            println!("{e}");
            MyStatusCode { code: StatusCode::BAD_REQUEST }
    })?;
    let body = String::from_utf8(body.to_vec())?;
    let http_req = HttpRequest {
        method,
        url,
        headers,
        body,
    };
    
    Ok(MsgTaskRequest::new(
        my_id.into(),
        vec![target_id.into()],
        "HttpPusher".into(),
        serde_json::to_string(&http_req)?,
        shared::FailureStrategy::Discard))
}

async fn shutdown_signal() {
    // Wait for the CTRL+C signal
    tokio::signal::ctrl_c()
        .await
        .expect("failed to install CTRL+C signal handler");
}

#[derive(Debug)]
struct MyStatusCode {
    code: StatusCode
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
            _ => StatusCode::NOT_IMPLEMENTED,
            SamplyBeamError::InvalidBeamId(e) => {
                error!("{e}");
                StatusCode::BAD_REQUEST
        }};
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