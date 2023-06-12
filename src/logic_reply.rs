use hyper::{Client, client::HttpConnector, Request, header, StatusCode, body, Response, Body, Uri, Method, http::uri::Scheme};
use hyper_proxy::ProxyConnector;
use hyper_tls::HttpsConnector;
use tracing::{info, warn, debug};
use serde_json::Value;
use shared::{MsgTaskRequest, MsgTaskResult, MsgId,beam_id::{BeamId,AppId}, WorkStatus, Plain, http_client::SamplyHttpClient};

use crate::{config::Config, errors::BeamConnectError, msg::{IsValidHttpTask, HttpResponse}};

pub(crate) async fn process_requests(config: Config, client: SamplyHttpClient) -> Result<(), BeamConnectError> {
    // Fetch tasks from Proxy
    let msgs = fetch_requests(&config, &client).await?;

    for task in msgs {
        // If we fail to execute the http task we should report this as a failure to beam
        let resp = execute_http_task(&task, &config, &client).await;

        send_reply(&task, &config, &client, resp).await?;
    }

    Ok(())
}

async fn send_reply(task: &MsgTaskRequest, config: &Config, client: &SamplyHttpClient, resp: Result<Response<Body>, BeamConnectError>) -> Result<(), BeamConnectError> {
    let (reply_body, status) = match resp {
        Ok(mut resp) => {
            let body = body::to_bytes(resp.body_mut()).await
                .map_err(BeamConnectError::FailedToReadTargetsReply)?;
            if !resp.status().is_success() {
                warn!("Httptask returned with status {}. Reporting failure to broker.", resp.status());
                // warn!("Response body was: {}", &body);
            };
            (serde_json::to_string(&HttpResponse {
                status: resp.status(),
                headers: resp.headers().clone(),
                body: body.to_vec()
            })?, WorkStatus::Succeeded)
        },
        Err(e) => {
            warn!("Failed to execute http task. Err: {e}");
            ("Error executing http task. See beam connect logs".to_string(), WorkStatus::PermFailed)
        },
    };
    let msg = MsgTaskResult {
        from: config.my_app_id.clone().into(),
        to: vec![task.from.clone()],
        task: task.id,
        status,
        metadata: Value::Null,
        body: Plain::from(reply_body),
    };
    let req_to_proxy = Request::builder()
        .method("PUT")
        .uri(format!("{}v1/tasks/{}/results/{}", config.proxy_url, task.id,config.my_app_id.clone()))
        .header(header::AUTHORIZATION, config.proxy_auth.clone())
        .body(body::Body::from(serde_json::to_vec(&msg)?))
        .map_err( BeamConnectError::HyperBuildError)?;
    debug!("Delivering response to Proxy: {:?}, {:?}", msg, req_to_proxy);
    let resp = client.request(req_to_proxy).await
        .map_err(BeamConnectError::ProxyHyperError)?;
    if resp.status() != StatusCode::CREATED {
        return Err(BeamConnectError::ProxyOtherError(format!("Got error code {} trying to submit our result.", resp.status())));
    }
    Ok(())
}

async fn execute_http_task(task: &MsgTaskRequest, config: &Config, client: &SamplyHttpClient) -> Result<Response<Body>, BeamConnectError> {
    let task_req = task.http_request()?;
    info!("{} | {} {}", task.from, task_req.method, task_req.url);
    let target = config
        .targets_local
        .get(task_req.url.authority().unwrap()) //TODO unwrap
        .ok_or_else(|| {
            warn!("Lookup of local target {} failed", task_req.url.authority().unwrap());
            BeamConnectError::CommunicationWithTargetFailed(String::from("Target not defined"))
        })?;
    if !target.allowed.contains(&AppId::try_from(&task.from).or(Err(BeamConnectError::IdNotAuthorizedToAccessUrl(task.from.clone(), task_req.url.clone())))?) {
        return Err(BeamConnectError::IdNotAuthorizedToAccessUrl(task.from.clone(), task_req.url.clone()));
    }
    if task_req.method == Method::CONNECT {
        debug!("Connect Request URL: {:?}", task_req.url);
    }
    
    let mut uri = Uri::builder();
    if let Some(scheme) = task_req.url.scheme_str() {
        uri = uri.scheme(scheme).path_and_query(task_req.url.path())
    } 
    let uri = uri
        .authority(target.replace.to_owned())
        .build()?;

    info!("Rewritten to: {} {}", task_req.method, uri);
    
    let mut req = Request::builder()
        .method(task_req.method)
        .uri(uri);
    *req.headers_mut().unwrap() = task_req.headers;
    let body = body::Body::from(task_req.body);
    let req = req.body(body)?;
    debug!("Issuing request: {:?}", req);
    let resp = client.request(req).await
        .map_err(|e| BeamConnectError::CommunicationWithTargetFailed(e.to_string()))?;
    Ok(resp)
}

async fn fetch_requests(config: &Config, client: &SamplyHttpClient) -> Result<Vec<MsgTaskRequest>, BeamConnectError> {
    let req_to_proxy = Request::builder()
        .uri(format!("{}v1/tasks?to={}&wait_count=1&filter=todo", config.proxy_url, config.my_app_id))
        .header(header::AUTHORIZATION, config.proxy_auth.clone())
        .header(header::ACCEPT, "application/json")
        .body(body::Body::empty())?;
    info!("Requesting {req_to_proxy:?}");
    let mut resp = client.request(req_to_proxy).await
        .map_err(BeamConnectError::ProxyHyperError)?;
    match resp.status() {
        StatusCode::OK => {
            info!("Got request: {:?}", resp);
        },
        StatusCode::GATEWAY_TIMEOUT => return Err(BeamConnectError::ProxyTimeoutError),
        _ => {
            return Err(BeamConnectError::ProxyOtherError(format!("Got response code {}", resp.status())));
        }
    }
    let bytes = body::to_bytes(resp.body_mut()).await
        .map_err(BeamConnectError::ProxyHyperError)?;
    let msgs = serde_json::from_slice::<Vec<MsgTaskRequest>>(&bytes);
    if let Err(e) = msgs {
        warn!("Unable to decode MsgTaskRequest; error: {e}. Content: {}", String::from_utf8_lossy(&bytes));
        return Err(e.into());
    }
    let msgs = msgs.unwrap();
    debug!("Broker gave us {} tasks: {:?}", msgs.len(), msgs.first());
    Ok(msgs)
}
