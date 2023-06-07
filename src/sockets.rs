use std::{time::Duration, collections::HashSet, io, sync::Arc, convert::Infallible};

use hyper::{header, Request, Body, body::{self, HttpBody}, StatusCode, upgrade::{Upgraded, self}, Response, http::{uri::Authority, HeaderValue}, client::conn::Builder, server::conn::Http, service::service_fn, Uri};
use serde::{Serialize, Deserialize};
use shared::{MsgId, beam_id::{AppOrProxyId, AppId}};
use tokio::net::TcpStream;
use tracing::{error, warn, debug};

use crate::{config::Config, errors::BeamConnectError, structs::MyStatusCode};

#[derive(Debug, Serialize, Deserialize)]
struct SocketTask {
    from: AppOrProxyId,
    to: Vec<AppOrProxyId>,
    id: MsgId,
    ttl: String,
}

pub(crate) fn spwan_socket_task_poller(config: Config) {
    tokio::spawn(async move {
        use BeamConnectError::*;

        loop {
            let tasks = match poll_socket_task(&config).await {
                Ok(tasks) => tasks,
                Err(HyperBuildError(e)) => {
                    error!("{e}");
                    error!("This is most likely caused by wrong configuration");
                    break;
                },
                Err(ProxyTimeoutError) => continue,
                Err(e) => {
                    warn!("Error during socket task polling: {e}");
                    tokio::time::sleep(Duration::from_secs(10)).await;
                    continue;
                }
            };
            for task in tasks {
                let Ok(client) = AppId::try_from(&task.from) else {
                    warn!("Invalid app id skipping");
                    continue;
                };
                match connect_proxy(&task.id, &config).await {
                    Ok(resp) => tunnel(resp, client, config.clone()),
                    Err(e) => {
                        warn!("{e}");
                        continue;
                    },
                };
                
            }


        }
    });
}

async fn poll_socket_task(config: &Config) -> Result<Vec<SocketTask>, BeamConnectError> {
    let poll_socket_tasks = Request::builder()
        .uri(format!("{}v1/sockets", config.proxy_url))
        .header(header::AUTHORIZATION, config.proxy_auth.clone())
        .header(header::ACCEPT, "application/json")
        .body(Body::empty())?;
    let mut resp = config.client.request(poll_socket_tasks).await.map_err(BeamConnectError::ProxyHyperError)?;
    match resp.status() {
        StatusCode::OK => {},
        StatusCode::GATEWAY_TIMEOUT => return Err(BeamConnectError::ProxyTimeoutError),
        e => return Err(BeamConnectError::ProxyOtherError(format!("Unexpected status code {e}")))
    };
    let body = body::to_bytes(resp.body_mut()).await.map_err(BeamConnectError::ProxyHyperError)?;
    Ok(serde_json::from_slice(&body)?)
}

async fn connect_proxy(task_id: &MsgId, config: &Config) -> Result<Response<Body>, BeamConnectError> {
    let connect_proxy_req = Request::builder()
        .uri(format!("{}v1/sockets/{task_id}", config.proxy_url))
        .header(header::AUTHORIZATION, config.proxy_auth.clone())
        .header(header::ACCEPT, "application/json")
        .body(Body::empty())
        .expect("This is a valid request");
    let resp = config.client.request(connect_proxy_req).await.map_err(BeamConnectError::ProxyHyperError)?;
    let invalid_status_reason = match resp.status() {
        StatusCode::SWITCHING_PROTOCOLS => return Ok(resp),
        StatusCode::NOT_FOUND | StatusCode::GONE => {
            "Task already expired".to_string()
        },
        StatusCode::UNAUTHORIZED => {
            "This socket is not for this authorized for this app".to_string()
        }
        other => other.to_string()
    };
    Err(BeamConnectError::ProxyOtherError(invalid_status_reason))
}

fn status_to_response(status: StatusCode) -> Response<Body> {
    let mut res = Response::default();
    *res.status_mut() = status;
    res
}

fn tunnel(proxy: Response<Body>, client: AppId, config: Config) {
    tokio::spawn(async move {
        let proxy = match upgrade::on(proxy).await {
            Ok(socket) => socket,
            Err(e) => {
                warn!("Failed to upgrade connection to proxy: {e}");
                return;
            },
        };
        let http_err = Http::new()
            .http1_only(true)
            .http1_keep_alive(true)
            .serve_connection(proxy, service_fn(move |req| {
                let client2 = client.clone();
                let config2 = config.clone();
                async move {
                    Ok::<_, Infallible>(handle_tunnel(req, &client2, &config2).await.unwrap_or_else(status_to_response))
                }
            }))
            .await;

        if let Err(e) = http_err {
            warn!("Error while serving HTTP connection: {e}");
        }
    });
}

async fn handle_tunnel(mut req: Request<Body>, app: &AppId, config: &Config) -> Result<Response<Body>, StatusCode> {
    // TODO: What exactly happens here if we dont have an authority
    let authority = req.uri().authority().unwrap();
    let Some(target) = config.targets_local.get(authority) else {
        warn!("Failed to lookup authority {authority}");
        return Err(StatusCode::BAD_REQUEST);
    };
    if !target.allowed.contains(&app) {
        warn!("App {app} not autherized to access url {}", req.uri());
        return Err(StatusCode::UNAUTHORIZED);
    };
    *req.uri_mut() = {
        let mut parts = req.uri().to_owned().into_parts();
        parts.authority = Some(authority.clone());
        Uri::from_parts(parts).map_err(|e| {
            warn!("Could not transform uri authority: {e}");
            StatusCode::INTERNAL_SERVER_ERROR
        })?
    };
    
    let resp = config.client.request(req).await.map_err(|e| {
        warn!("Communication with target failed: {e}");
        StatusCode::BAD_GATEWAY
    })?;
    Ok(resp)
}

async fn handle_via_sockets(req: Request<Body>, config: &Arc<Config>, target: &AppId, auth: HeaderValue) -> Result<Response<Body>, MyStatusCode> {
    let connect_proxy_req = Request::builder()
        .uri(format!("{}v1/sockets/{target}", config.proxy_url))
        .header(header::AUTHORIZATION, config.proxy_auth.clone())
        .header(header::ACCEPT, "application/json")
        .body(Body::empty())
        .expect("This is a valid request");
    let resp = config.client.request(connect_proxy_req).await.map_err(|e| {
        warn!("Failed to reach proxy");
        StatusCode::BAD_GATEWAY
    })?;
    if resp.status() != StatusCode::SWITCHING_PROTOCOLS {
        return Err(resp.status().into());
    }
    let proxy_socket = upgrade::on(resp).await.map_err(|e| {
        warn!("Failed to upgrade response from proxy to socket: {e}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let (mut sender, conn) = Builder::new()
        .http1_preserve_header_case(true)
        .http1_title_case_headers(true)
        .handshake(proxy_socket)
        .await
        .map_err(|e| {
            warn!("Error doing handshake with proxy");
            StatusCode::BAD_GATEWAY
        })?;
    tokio::task::spawn(async move {
        if let Err(err) = conn.await {
            warn!("Connection failed: {:?}", err);
        }
    });

    let resp = sender.send_request(req).await.map_err(|e| {
        warn!("Failed to send request to proxy");
        StatusCode::BAD_GATEWAY
    })?;
    Ok(resp)
}
