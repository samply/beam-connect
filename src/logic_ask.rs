use std::{sync::Arc, str::FromStr};
use std::time::{Duration, SystemTime};
use hyper::http::HeaderValue;
use hyper::{Request, Body, Client, client::HttpConnector, Response, header, StatusCode, body, Uri};
use hyper_proxy::ProxyConnector;
use hyper_tls::HttpsConnector;
use tracing::{info, debug, warn, error};
use serde_json::Value;
use shared::http_client::SamplyHttpClient;
use shared::{beam_id::AppId, MsgTaskResult, MsgTaskRequest};

use crate::{config::Config, structs::MyStatusCode, msg::{HttpRequest, HttpResponse}, errors::BeamConnectError};

/// GET   http://some.internal.system?a=b&c=d
/// Host: <identical>
/// This function knows from its map which app to direct the message to 
pub(crate) async fn handler_http(
    mut req: Request<Body>,
    config: Arc<Config>,
    client: SamplyHttpClient
) -> Result<Response<Body>, MyStatusCode> {
    let targets = &config.targets_public;
    let method = req.method().to_owned();
    let uri = req.uri().to_owned();
    let headers = req.headers_mut();

    headers.insert(header::VIA, format!("Via: Samply.Beam.Connect/0.1 {}", config.my_app_id).parse().unwrap());

    let auth = headers
        .remove(header::PROXY_AUTHORIZATION)
        .unwrap_or(config.proxy_auth.parse().expect("Proxy auth header could not be generated."));

    // Re-pack Authorization: Not necessary since we're not looking at the Authorization header.
    // if headers.remove(header::AUTHORIZATION).is_some() {
    //     return Err(StatusCode::CONFLICT.into());
    // }

    // If the autority is empty (e.g. if localhost is used) or the authoroty is not in the routing
    // table AND the path is /sites, return global routing table
    if let Some(path) = uri.path_and_query() { 
        if (uri.authority().is_none() || targets.get(uri.authority().unwrap()).is_none()) && path == "/sites" {
            debug!("Central Site Discovery requested");
            let body = body::Body::from(serde_json::to_string(targets)?);
            let response = Response::builder()
                .status(200)
                .body(body)
                .unwrap();
            return Ok(response);

        }
    }

    let target = &targets.get(uri.authority().unwrap()) //TODO unwrap
        .ok_or_else(|| {
            warn!("Failed to lookup virtualhost in central mapping: {}", uri.authority().unwrap());
            StatusCode::UNAUTHORIZED
        })?
        .beamconnect;

    info!("{method} {uri} via {target}");

    let msg = http_req_to_struct(req, &config.my_app_id, &target, &config.expire).await?;

    // Send to Proxy
    let req_to_proxy = Request::builder()
        .method("POST")
        .uri(format!("{}v1/tasks", config.proxy_url))
        .header(header::AUTHORIZATION, auth.clone())
        .body(body::Body::from(serde_json::to_vec(&msg)?))
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
        .path_and_query(format!("{}/results?wait_count=1&wait_timeout=10000", location.path()))
        .build().unwrap(); // TODO
    debug!("Fetching reply from Proxy: {results_uri}");
    let req = Request::builder()
        .header(header::AUTHORIZATION, auth)
        .header(header::ACCEPT, "application/json")
        .uri(results_uri)
        .body(body::Body::empty()).unwrap();
    let mut resp = client.request(req).await
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
            serde_json::from_str::<HttpResponse>(&result.body.body.ok_or({
                warn!("Recieved one sucessfull result but it has no body");
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
            println!("{e}");
            StatusCode::BAD_REQUEST
    })?;
    let body = String::from_utf8(body.to_vec())?;

    let http_req = HttpRequest {
        method,
        url,
        headers,
        body,
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
