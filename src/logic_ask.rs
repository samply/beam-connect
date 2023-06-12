use std::{sync::Arc, str::FromStr};
use std::time::{Duration, SystemTime};
use hyper::Method;
use hyper::http::HeaderValue;
use hyper::http::uri::{Authority, Scheme};
use hyper::server::conn::Http;
use hyper::service::service_fn;
use hyper::{Request, Body, Client, client::HttpConnector, Response, header, StatusCode, body, Uri};
use hyper_proxy::ProxyConnector;
use hyper_tls::HttpsConnector;
use tracing::{info, debug, warn, error};
use serde_json::Value;
use shared::http_client::SamplyHttpClient;
use shared::{beam_id::AppId, MsgTaskResult, MsgTaskRequest};

use crate::config::CentralMapping;
use crate::{config::Config, structs::MyStatusCode, msg::{HttpRequest, HttpResponse}, errors::BeamConnectError};

/// GET   http://some.internal.system?a=b&c=d
/// Host: <identical>
/// This function knows from its map which app to direct the message to 
pub(crate) async fn handler_http(
    mut req: Request<Body>,
    config: Arc<Config>,
    https_authority: Option<Authority>,
) -> Result<Response<Body>, MyStatusCode> {

    let targets = &config.targets_public;
    let method = req.method().to_owned();
    let uri = req.uri().to_owned();
    let Some(authority) = https_authority.as_ref().or(uri.authority()) else {
        return if uri.path() == "/sites" {
            // Case 1 for sites request: no authority set and /sites
            respond_with_sites(targets)
        } else {
            Err(StatusCode::BAD_REQUEST.into())
        }
    };
    let headers = req.headers_mut();

    headers.insert(header::VIA, format!("Via: Samply.Beam.Connect/0.1 {}", config.my_app_id).parse().unwrap());

    let auth = headers.remove(header::PROXY_AUTHORIZATION)
            .ok_or(StatusCode::PROXY_AUTHENTICATION_REQUIRED)?;

    // Re-pack Authorization: Not necessary since we're not looking at the Authorization header.
    // if headers.remove(header::AUTHORIZATION).is_some() {
    //     return Err(StatusCode::CONFLICT.into());
    // }

    let Some(target) = &targets
        .get(authority)
        .map(|target| &target.beamconnect) else {
            return if uri.path() == "/sites" {
                // Case 2: target not in sites and /sites
                respond_with_sites(targets)
            } else {
                warn!("Failed to lookup virtualhost in central mapping: {}", authority);
                Err(StatusCode::UNAUTHORIZED.into())
            }
        };

    info!("{method} {uri} via {target}");

    // Set the right authority as it might have been passed by the caller because it was a CONNECT request
    *req.uri_mut() = {
        let mut parts = req.uri().to_owned().into_parts();
        parts.authority = Some(authority.clone());
        if https_authority.is_some() {
            parts.scheme = Some(Scheme::HTTPS);
        } else {
            parts.scheme = Some(Scheme::HTTP);
        }
        Uri::from_parts(parts).map_err(|e| {
            warn!("Could not transform uri authority: {e}");
            StatusCode::INTERNAL_SERVER_ERROR
        })?
    };
    #[cfg(feature = "sockets")]
    return crate::sockets::handle_via_sockets(req, &config, target, auth).await;
    #[cfg(not(feature = "sockets"))]
    return handle_via_tasks(req, &config, target, auth).await;
}

async fn handle_via_tasks(req: Request<Body>, config: &Arc<Config>, target: &AppId, auth: HeaderValue) -> Result<Response<Body>, MyStatusCode> {
    let msg = http_req_to_struct(req, &config.my_app_id, &target, &config.expire).await?;

    // Send to Proxy
    let req_to_proxy = Request::builder()
        .method("POST")
        .uri(format!("{}v1/tasks", config.proxy_url))
        .header(header::AUTHORIZATION, auth.clone())
        .body(body::Body::from(serde_json::to_vec(&msg)?))
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    debug!("SENDING request to Proxy: {:?}, {:?}", msg, req_to_proxy);
    let resp = config.client.request(req_to_proxy).await
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
        .path_and_query(format!("{}/results?wait_count=1&wait_timeout=10000", location.path()))
        .build().unwrap(); // TODO
    debug!("Fetching reply from Proxy: {results_uri}");
    let req = Request::builder()
        .header(header::AUTHORIZATION, auth)
        .header(header::ACCEPT, "application/json")
        .uri(results_uri)
        .body(body::Body::empty()).unwrap();
    let mut resp = config.client.request(req).await
        .map_err(|e| {
            warn!("Got error from server: {e}");
            StatusCode::BAD_GATEWAY
        })?;
    info!("Got reply: {:?}", resp);

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
    let mut task_results = serde_json::from_slice::<Vec<MsgTaskResult>>(&bytes)
        .map_err(|e| {
            warn!("Unable to parse HTTP result: {}", e);
            StatusCode::BAD_GATEWAY
        })?;
    debug!("Got reply: {:?}", task_results);

    if task_results.len() != 1 {
        error!("Reply had more than one answer (namely: {}). This should not happen; discarding request.", task_results.len());
        return Err(StatusCode::BAD_GATEWAY)?;
    }
    let result = task_results.pop().unwrap();
    let response_inner = match result.status {
        shared::WorkStatus::Succeeded => {
            serde_json::from_str::<HttpResponse>(&result.body.body.ok_or_else(|| {
                warn!("Received one successful result but it has no body");
                StatusCode::BAD_GATEWAY
            })?)?
        },
        e => {
            warn!("Reply had unexpected workresult code: {}", e);
            return Err(StatusCode::BAD_GATEWAY)?;
        }
    };

    if let Some(expected_len) = response_inner.headers.get(header::CONTENT_LENGTH) {
        let expected_len = usize::from_str(expected_len.to_str()?)
            .map_err(|e| {
                error!("We got an invalid content length: {e}. This is probably a bug.");
                StatusCode::INTERNAL_SERVER_ERROR
            })?;
        if expected_len != response_inner.body.len() {
            error!("Critical Error: The other Proxy received a reply with length {expected_len}; now it's {}.", response_inner.body.len());
            return Err(StatusCode::INTERNAL_SERVER_ERROR)?;
        }
    }

    let mut resp = Response::builder()
        .status(response_inner.status);
    *resp.headers_mut().unwrap() = response_inner.headers;
    let resp = resp
        .body(body::Body::from(response_inner.body))
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(resp)
}

async fn http_req_to_struct(req: Request<Body>, my_id: &AppId, target_id: &AppId, expire: &u64) -> Result<MsgTaskRequest, MyStatusCode> {
    let method = req.method().clone();
    let url = req.uri().clone();
    let headers = req.headers().clone();
    let body = body::to_bytes(req).await
        .map_err(|e| {
            warn!("Failed to read body: {e}");
            StatusCode::BAD_REQUEST
    })?;

    let http_req = HttpRequest {
        method,
        url,
        headers,
        body: body.to_vec(),
    };
    let mut msg = MsgTaskRequest::new(
        my_id.into(),
        vec![target_id.into()],
        serde_json::to_string(&http_req)?,
        shared::FailureStrategy::Discard,
        Value::Null
    );

    msg.expire = SystemTime::now() + Duration::from_secs(*expire);
    
    Ok(msg)
}

/// If the authority is empty (e.g. if localhost is used) or the authoroty is not in the routing
/// table AND the path is /sites, return global routing table
fn respond_with_sites(targets: &CentralMapping) -> Result<Response<Body>, MyStatusCode> {
    debug!("Central Site Discovery requested");
    let body = body::Body::from(serde_json::to_string(targets)?);
    let response = Response::builder()
        .status(200)
        .body(body)
        .unwrap();
    Ok(response)
}
