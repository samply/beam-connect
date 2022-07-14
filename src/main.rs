use std::{net::SocketAddr, str::FromStr, convert::Infallible, string::FromUtf8Error, error::Error, fmt::Display, collections::{hash_map, HashMap}, sync::Arc};

use config::Config;
use example_targets::InternalHost;
use hyper::{body::{self, Bytes}, Body, StatusCode, service::{Service, service_fn, make_service_fn}, Request, Response, Server, header::{HeaderName, self, ToStrError}, Uri, http::uri::Authority, server::conn::AddrStream, Client, client::HttpConnector};
use log::{info, error, debug, warn};
use msg::HttpRequest;
use shared::{beam_id::{AppId, BeamId}, MsgTaskRequest, errors::SamplyBeamError, MsgTaskResult};
use thiserror::Error;

mod msg;
mod example_targets;
mod config;

#[derive(Error,Debug)]
enum HttPusherError {
    #[error("Proxy rejected our authorization")]
    ProxyRejectedAuthorization,
    #[error("Unable to fetch request from Proxy: {0}")]
    ProxyFetchHyperError(#[from] hyper::Error),
    #[error("Constructing HTTP request failed: {0}")]
    HyperBuildError(#[from] hyper::http::Error),
    #[error("Unable to fetch request from Proxy: {0}")]
    ProxyFetchOtherError(String),
    #[error("Error parsing response: {0}")]
    ProxyParseResponseError(#[from] serde_json::Error)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>>{
    pretty_env_logger::init();
    let listen = SocketAddr::from_str("0.0.0.0:8082").unwrap();

    let config = Config::load()?;
    let config2 = config.clone();
    let client = hyper::Client::builder().build_http();
    let client2 = client.clone();

    let http_executor = tokio::task::spawn(async move {
        loop {
            debug!("Waiting for next request ...");
            if let Err(e) = process_requests(config2.clone(), client2.clone()).await {
                warn!("Error in processing request: {e}. Will continue with the next one.");
            }
        }
    });

    // A MakeService handles each connection
    let make_service = 
        make_service_fn(|_conn: &AddrStream| {
            debug!("make_service");
            // let remote_addr = conn.remote_addr();
            let targets = Arc::new(example_targets::get_examples());
            let config = Arc::new(config.clone());
            let client = client.clone();
            async {
                Ok::<_, Infallible>(service_fn(move |req|
                    handler_http_wrapper(req, config.clone(), client.clone(), targets.clone())))
            }
    });

    let server =
        Server::bind(&listen)
        .serve(make_service)
        .with_graceful_shutdown(shutdown_signal());

    if let Err(e) = server.await {
        eprintln!("server error: {}", e);
    }
    info!("(2/2) Shutting down gracefully ...");
    http_executor.abort();
    http_executor.await.unwrap();
    Ok(())
}

async fn process_requests(config: Config, client: Client<HttpConnector>) -> Result<(), HttPusherError> {
    // Fetch task from Proxy
    let req_to_proxy = Request::builder()
        .uri(format!("{}v1/tasks?to={}&poll_count=1&unanswered=true", config.proxy_url, config.my_app_id))
        .header(header::AUTHORIZATION, config.proxy_auth)
        .body(body::Body::empty())?;
    let mut resp = client.request(req_to_proxy).await?;
    match resp.status() {
        StatusCode::OK => {
            info!("Got request: {:?}", resp);
        },
        _ => {
            return Err(HttPusherError::ProxyFetchOtherError(format!("Got response code {}", resp.status())));
        }
    }

    // Try to parse
    let bytes = body::to_bytes(resp.body_mut()).await?;
    // let msg
    let msg = serde_json::from_slice::<Vec<MsgTaskRequest>>(&bytes);
    if let Err(e) = msg {
        warn!("Unable to decode MsgTaskRequest; error: {e}. Content: {}", String::from_utf8_lossy(&bytes));
        return Err(e.into());
    }
    let msg = msg.unwrap();
    debug!("SUCCESS! Request contained {} tasks: {:?}", msg.len(), msg.first());

    Ok(())
}

async fn handler_http_wrapper(
    req: Request<Body>,
    config: Arc<Config>,
    client: Client<HttpConnector>,
    targets: Arc<HashMap<InternalHost, AppId>>
) -> Result<Response<Body>, Infallible> {
    match handler_http(req, config, client, targets).await {
        Ok(e) => Ok(e),
        Err(e) => Ok(Response::builder().status(e.code).body(body::Body::empty()).unwrap()),
    }
}

/// GET   http://some.internal.system?a=b&c=d
/// Host: <identical>
/// This function knows from its map which app to direct the message to 
async fn handler_http(
    mut req: Request<Body>,
    config: Arc<Config>,
    client: Client<HttpConnector>,
    targets: Arc<HashMap<InternalHost, AppId>>
) -> Result<Response<Body>,MyStatusCode> {
    let method = req.method().to_owned();
    let uri = req.uri().to_owned();
    let headers = req.headers_mut();
    
    info!("{method} {uri} {:?}", headers);

    headers.insert(header::VIA, format!("Via: Samply.Beam.Httpusher/0.1 {}", config.my_app_id).parse().unwrap());

    let auth = 
        headers.remove(header::PROXY_AUTHORIZATION)
            .ok_or(StatusCode::PROXY_AUTHENTICATION_REQUIRED)?;

    // Re-pack Authorization: Not necessary since we're not looking at the Authorization header.
    // if headers.remove(header::AUTHORIZATION).is_some() {
    //     return Err(StatusCode::CONFLICT.into());
    // }

    let target = targets.get(uri.authority().unwrap()) //TODO unwrap
        .ok_or(StatusCode::UNAUTHORIZED)?;

    let msg = req_to_task(req, &config.my_app_id, &target).await?;

    // Send to Proxy
    let req_to_proxy = Request::builder()
        .method("POST")
        .uri(format!("{}v1/tasks", config.proxy_url))
        .header(header::AUTHORIZATION, auth.clone())
        .body(body::Body::from(serde_json::to_vec(&msg).unwrap()))
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    debug!("SENDING request to Proxy: {:?}, {:?}", msg, req_to_proxy);
    let resp = client.request(req_to_proxy).await
        .map_err(|_| StatusCode::BAD_GATEWAY)?;
    if resp.status() != StatusCode::CREATED {
        return Err(StatusCode::BAD_GATEWAY.into());
    }

    // Fetch Task ID
    let location = resp.headers().get(header::LOCATION)
        .and_then(|loc| loc.to_str().ok())
        .and_then(|loc| Uri::from_str(loc).ok())
        .ok_or(StatusCode::BAD_GATEWAY)?;

    // Ask Proxy for MsgResult
    let results_uri = Uri::builder()
        .scheme(config.proxy_url.scheme().unwrap().as_str())
        .authority(config.proxy_url.authority().unwrap().to_owned())
        .path_and_query(format!("{}/results?poll_count=1&poll_timeout=10000", location.path()))
        .build().unwrap(); // TODO
    debug!("Fetching reply from Proxy: {results_uri}");
    let req = Request::builder()
        .header(header::AUTHORIZATION, auth)
        .uri(results_uri)
        .body(body::Body::empty()).unwrap();
    let mut resp = client.request(req).await
        .map_err(|e| {
            warn!("Got error from server: {e}");
            StatusCode::BAD_GATEWAY
        })?;
    info!("Got reply: {:?}", resp);
    // let bytes = body::to_bytes(resp.body_mut()).await
    //     .map_err(|_| StatusCode::BAD_GATEWAY)?;

    match resp.status() {
        StatusCode::PARTIAL_CONTENT => {
            warn!("Timeout fetching reply.");
            return Err(StatusCode::GATEWAY_TIMEOUT)?;
        },
        StatusCode::OK => {
            debug!("Got non-empty reply: {:?}", resp);
        },
        e => {
            warn!("Error fetching reply, got code: {e}");
            return Err(StatusCode::BAD_GATEWAY)?;
        }
    }

    let bytes = body::to_bytes(resp.body_mut()).await
        .map_err(|_| StatusCode::BAD_GATEWAY)?;
    let body = serde_json::from_slice::<MsgTaskResult>(&bytes)
        .map_err(|e| {
            warn!("Unable to parse HTTP result: {}", e);
            StatusCode::BAD_GATEWAY
        })?;
    debug!("Got reply: {:?}", body);

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
        serde_json::to_string(&http_req)?,
        shared::FailureStrategy::Discard,
        "HttPusher".into()
    ))
}

async fn shutdown_signal() {
    // Wait for the CTRL+C signal
    info!("Starting ...");
    tokio::signal::ctrl_c()
        .await
        .expect("failed to install CTRL+C signal handler");
    info!("(1/2) Shutting down gracefully ...");
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
            SamplyBeamError::InvalidBeamId(e) => {
                error!("{e}");
                StatusCode::BAD_REQUEST
            },
            _ => StatusCode::NOT_IMPLEMENTED,
        };
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