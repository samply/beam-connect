use std::{time::Duration, collections::HashSet, sync::Arc, convert::Infallible};

use futures_util::TryStreamExt;
use http_body_util::{combinators::BoxBody, BodyExt, BodyStream, StreamBody};
use hyper::{body::Incoming, header, http::{uri::PathAndQuery, HeaderValue}, service::service_fn, upgrade::OnUpgrade, Request, StatusCode, Uri};
use hyper_util::rt::{TokioExecutor, TokioIo};
use tokio::{io::AsyncWriteExt, task::JoinHandle};
use tracing::{error, warn, debug, info};
use beam_lib::{SocketTask, MsgId, AppId, AppOrProxyId};
use reqwest::Response;

use crate::{config::Config, errors::BeamConnectError, structs::MyStatusCode};


pub(crate) fn spawn_socket_task_poller(config: Config) -> JoinHandle<()> {
    tokio::spawn(async move {
        use BeamConnectError::*;
        let mut seen: HashSet<MsgId> = HashSet::new();

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
                if seen.contains(&task.id) {
                    continue;
                }
                seen.insert(task.id.clone());
                let AppOrProxyId::App(client) = task.from else {
                    warn!("Invalid app id skipping");
                    continue;
                };
                let config_clone = config.clone();
                tokio::spawn(async move {
                    match connect_proxy(&task.id, &config_clone).await {
                        Ok(resp) => tunnel(resp, client, &config_clone).await,
                        Err(e) => {
                            warn!("{e}");
                        },
                    };
                });
            }
        }
    })
}

async fn poll_socket_task(config: &Config) -> Result<Vec<SocketTask>, BeamConnectError> {
    let resp = config.client
        .get(format!("{}v1/sockets", config.proxy_url))
        .header(header::AUTHORIZATION, config.proxy_auth.clone())
        .header(header::ACCEPT, "application/json")
        .send()
        .await
        .map_err(BeamConnectError::ProxyReqwestError)?;
    match resp.status() {
        StatusCode::OK => {},
        StatusCode::GATEWAY_TIMEOUT => return Err(BeamConnectError::ProxyTimeoutError),
        e => return Err(BeamConnectError::ProxyOtherError(format!("Unexpected status code {e}")))
    };
    resp.json().await.map_err(BeamConnectError::ProxyReqwestError)
}

async fn connect_proxy(task_id: &MsgId, config: &Config) -> Result<Response, BeamConnectError> {
    let resp = config.client
        .get(format!("{}v1/sockets/{task_id}", config.proxy_url))
        .header(header::AUTHORIZATION, config.proxy_auth.clone())
        .header(header::UPGRADE, "tcp")
        .send()
        .await
        .map_err(BeamConnectError::ProxyReqwestError)?;
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

fn status_to_response(status: StatusCode) -> crate::Response {
    let mut res = hyper::Response::default();
    *res.status_mut() = status;
    res
}

async fn tunnel(proxy: Response, client: AppId, config: &Config) {
    let proxy = match proxy.upgrade().await {
        Ok(socket) => socket,
        Err(e) => {
            warn!("Failed to upgrade connection to proxy: {e}");
            return;
        },
    };
    let http_err = hyper_util::server::conn::auto::Builder::new(TokioExecutor::new())
        .serve_connection_with_upgrades(TokioIo::new(proxy), service_fn(move |req| {
            let client2 = client.clone();
            let config2 = config.clone();
            async move {
                Ok::<_, Infallible>(execute_http_task(req, &client2, &config2).await.unwrap_or_else(status_to_response))
            }
        }))
        .await;

    if let Err(e) = http_err {
        warn!("Error while serving HTTP connection: {e}");
    }
}

async fn execute_http_task(mut req: Request<Incoming>, app: &AppId, config: &Config) -> Result<crate::Response, StatusCode> {
    let authority = req.uri().authority().expect("Authority is always set by the requesting beam-connect");
    let Some(target) = config.targets_local.get(authority) else {
        warn!("Failed to lookup authority {authority}");
        return Err(StatusCode::BAD_REQUEST);
    };
    if !target.can_be_accessed_by(&app) {
        warn!("App {app} not authorized to access url {}", req.uri());
        return Err(StatusCode::UNAUTHORIZED);
    };
    *req.uri_mut() = {
        let mut parts = req.uri().to_owned().into_parts();
        if target.force_https {
            parts.scheme = Some(hyper::http::uri::Scheme::HTTPS)
        }
        parts.authority = Some(target.replace.authority.clone());
        if let Some(path) = target.replace.path {
            parts.path_and_query = Some(PathAndQuery::try_from(&format!("/{path}{}", parts.path_and_query.as_ref().map(PathAndQuery::as_str).unwrap_or(""))).map_err(|e| {
                warn!("Failed to set redirect path: {e}");
                StatusCode::INTERNAL_SERVER_ERROR
            })?);
        }
        Uri::from_parts(parts).map_err(|e| {
            warn!("Could not transform uri authority: {e}");
            StatusCode::INTERNAL_SERVER_ERROR
        })?
    };
    info!("Requesting {} {}", req.method(), req.uri());
    let req_upgrade = if req.headers().contains_key(header::UPGRADE) {
        req.extensions_mut().remove::<OnUpgrade>()
    } else {
        None
    };

    let mut resp = config.client
        .execute(req.map(|b| reqwest::Body::wrap_stream(BodyStream::new(b).map_ok(|v| v.into_data().expect("TODO: How to handle trailers?")))).try_into().expect("This should always convert"))
        .await
        .map_err(|e| {
            warn!("Error executuing http task. Failed handshake with server: {e}");
            StatusCode::BAD_GATEWAY
        })?;

    if req_upgrade.is_some() {
        tunnel_upgrade(resp.extensions_mut().remove::<OnUpgrade>(), req_upgrade);
    }
    Ok(convert_to_hyper_response(resp))
}

// TODO: Make a PR to add into_parts for reqwest::Response or even a conversion trait impl to avoid clones
fn convert_to_hyper_response(resp: Response) -> crate::Response {
    let mut builder = hyper::http::response::Builder::new()
        .status(resp.status())
        .version(resp.version());
    builder.headers_mut().map(|headers| *headers = resp.headers().clone());
    let stream = resp
        .bytes_stream()
        .map_ok(hyper::body::Frame::data)
        .map_err(Into::into);
    builder.body(BoxBody::new(StreamBody::new(stream))).expect("This should always convert")
}

fn tunnel_upgrade(client: Option<OnUpgrade>, server: Option<OnUpgrade>) {
    if let (Some(client), Some(proxy)) = (client, server) {
        tokio::spawn(async move {
            let (client, proxy) = match tokio::try_join!(client, proxy) {
                Err(e) => {
                    warn!("Upgrading connection between client and beam-connect failed: {e}");
                    return;
                },
                Ok(sockets) => sockets
            };
            let result = tokio::io::copy_bidirectional(&mut TokioIo::new(client), &mut TokioIo::new(proxy)).await;
            if let Err(e) = result {
                debug!("Relaying socket connection ended: {e}");
            }
        });
    }
}

pub(crate) async fn handle_via_sockets(mut req: Request<Incoming>, config: &Arc<Config>, target: &AppId, auth: HeaderValue) -> Result<crate::Response, MyStatusCode> {
    let resp = config.client
        .post(format!("{}v1/sockets/{target}", config.proxy_url))
        .header(header::AUTHORIZATION, auth)
        .header(header::UPGRADE, "tcp")
        .send()
        .await
        .map_err(|e| {
            warn!("Failed to reach proxy: {e}");
            StatusCode::BAD_GATEWAY
        }
    )?;
    if resp.status() != StatusCode::SWITCHING_PROTOCOLS {
        return Err(resp.status().into());
    }
    let proxy_socket = resp.upgrade().await.map_err(|e| {
        warn!("Failed to upgrade response from proxy to socket: {e}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let (mut sender, proxy_conn) = hyper::client::conn::http1::Builder::new()
        .preserve_header_case(true)
        .title_case_headers(true)
        .handshake(TokioIo::new(proxy_socket))
        .await
        .map_err(|e| {
            warn!("Error doing handshake with proxy: {e}");
            StatusCode::BAD_GATEWAY
        })?;
    let req_upgrade = if req.headers().contains_key(header::UPGRADE) {
        req.extensions_mut().remove::<OnUpgrade>()
    } else {
        None
    };
    let resp_future = sender.send_request(req);
    let resp = if let Some(upgrade) = req_upgrade {
        let (resp, proxy_connection) = tokio::join!(resp_future, proxy_conn.without_shutdown());
        match proxy_connection {
            Ok(mut proxy_io) => {
                tokio::spawn(async move {
                    let Ok(client) = upgrade.await else {
                        warn!("Failed to upgrade client connection");
                        return;
                    };
                    let mut client = TokioIo::new(client);
                    if !proxy_io.read_buf.is_empty() {
                        if let Err(e) = client.write_all_buf(&mut proxy_io.read_buf).await {
                            warn!("Failed to send initial bytes from remote to client: {e}");
                        }
                    }
                    if let Err(e) = tokio::io::copy_bidirectional(&mut client, &mut TokioIo::new(proxy_io.io)).await {
                        debug!("Error relaying connection from client to proxy: {e}");
                    }
                });
            },
            Err(e) => {
                warn!("Connection failed: {e}");
            },
        };
        resp
    } else {
        tokio::spawn(proxy_conn);
        resp_future.await
    };
    let resp = resp.map_err(|e| {
        warn!("Failed to send request to proxy: {e}");
        StatusCode::BAD_GATEWAY
    })?;
    Ok(resp.map(|b| BoxBody::new(b.map_err(Into::into))))
}
