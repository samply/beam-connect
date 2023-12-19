use beam_lib::{TaskRequest, TaskResult, WorkStatus, AppOrProxyId};
use hyper::{header, StatusCode, body, Uri, Method, http::uri::PathAndQuery};
use tracing::{info, warn, debug};
use serde_json::Value;
use reqwest::{Client, Response};

use crate::{config::Config, errors::BeamConnectError, msg::{HttpResponse, HttpRequest}};

pub(crate) async fn process_requests(config: Config, client: Client) -> Result<(), BeamConnectError> {
    // Fetch tasks from Proxy
    let msgs = fetch_requests(&config, &client).await?;

    for task in msgs {
        // If we fail to execute the http task we should report this as a failure to beam
        let resp = execute_http_task(&task, &config, &client).await;

        send_reply(&task, &config, &client, resp).await?;
    }

    Ok(())
}

async fn send_reply(task: &TaskRequest<HttpRequest>, config: &Config, client: &Client, resp: Result<Response, BeamConnectError>) -> Result<(), BeamConnectError> {
    let (reply_body, status) = match resp {
        Ok(resp) => {
            let status = resp.status();
            let headers = resp.headers().clone();
            if !status.is_success() {
                warn!("Httptask returned with status {}. Reporting failure to broker.", resp.status());
                // warn!("Response body was: {}", &body);
            };
            let body = resp.bytes().await
                .map_err(BeamConnectError::FailedToReadTargetsReply)?;
            (HttpResponse {
                status,
                headers,
                body: body.to_vec()
            }, WorkStatus::Succeeded)
        },
        Err(e) => {
            warn!("Failed to execute http task. Err: {e}");
            (HttpResponse { 
                body: b"Error executing http task. See beam connect logs".to_vec(),
                status: StatusCode::INTERNAL_SERVER_ERROR,
                headers: Default::default(), 
            }, WorkStatus::PermFailed)
        },
    };
    let msg = TaskResult {
        from: config.my_app_id.clone().into(),
        to: vec![task.from.clone()],
        task: task.id,
        status,
        metadata: Value::Null,
        body: reply_body,
    };
    debug!("Delivering response to Proxy: {msg:?}");
    let resp = client
        .put(format!("{}v1/tasks/{}/results/{}", config.proxy_url, task.id, config.my_app_id.clone()))
        .header(header::AUTHORIZATION, config.proxy_auth.clone())
        .json(&msg)
        .send()
        .await
        .map_err(BeamConnectError::ProxyReqwestError)?;

    if resp.status() != StatusCode::CREATED {
        return Err(BeamConnectError::ProxyOtherError(format!("Got error code {} trying to submit our result.", resp.status())));
    }
    Ok(())
}

// TODO: Take ownership of `task` to save clones
async fn execute_http_task(task: &TaskRequest<HttpRequest>, config: &Config, client: &Client) -> Result<Response, BeamConnectError> {
    let task_req = &task.body;
    info!("{} | {} {}", task.from, task_req.method, task_req.url);
    let target = config
        .targets_local
        .get(task_req.url.authority().unwrap()) //TODO unwrap
        .ok_or_else(|| {
            warn!("Lookup of local target {} failed", task_req.url.authority().unwrap());
            BeamConnectError::CommunicationWithTargetFailed(String::from("Target not defined"))
        })?;
    match &task.from {
        AppOrProxyId::App(app) if target.allowed.contains(app) => {},
        id => return Err(BeamConnectError::IdNotAuthorizedToAccessUrl(id.clone(), task_req.url.clone())),
    };
    if task_req.method == Method::CONNECT {
        debug!("Connect Request URL: {:?}", task_req.url);
    }
    
    let mut uri = Uri::builder();
    // Normal non CONNECT http request replacement
    if let Some(scheme) = task_req.url.scheme_str() {
        if target.force_https {
            uri = uri.scheme(hyper::http::uri::Scheme::HTTPS);
        } else {
            uri = uri.scheme(scheme);
        }
        uri = if let Some(path) = target.replace.path {
            uri.path_and_query(&format!("/{path}{}", task_req.url.path_and_query().unwrap_or(&PathAndQuery::from_static(""))))
        } else {
            uri.path_and_query(task_req.url.path_and_query().unwrap_or(&PathAndQuery::from_static("")).as_str())
        };
    } 
    let uri = uri
        .authority(target.replace.authority.to_owned())
        .build()?;

    info!("Rewritten to: {} {}", task_req.method, uri);
    let resp = client
        .request(task_req.method.clone(), uri.to_string())
        .headers(task_req.headers.clone())
        .body(body::Body::from(task_req.body.clone()))
        .send()
        .await
        .map_err(|e| BeamConnectError::CommunicationWithTargetFailed(e.to_string()))?;
    Ok(resp)
}

async fn fetch_requests(config: &Config, client: &Client) -> Result<Vec<TaskRequest<HttpRequest>>, BeamConnectError> {
    info!("fetching requests from proxy");
    let resp = client
        .get(format!("{}v1/tasks?to={}&wait_count=1&filter=todo", config.proxy_url, config.my_app_id))
        .header(header::AUTHORIZATION, config.proxy_auth.clone())
        .header(header::ACCEPT, "application/json")
        .send()
        .await
        .map_err(BeamConnectError::ProxyReqwestError)?;
    match resp.status() {
        StatusCode::OK => {
            info!("Got request: {:?}", resp);
        },
        StatusCode::GATEWAY_TIMEOUT => return Err(BeamConnectError::ProxyTimeoutError),
        _ => {
            return Err(BeamConnectError::ProxyOtherError(format!("Got response code {}", resp.status())));
        }
    }
    resp.json().await.map_err(|e| {
        warn!("Unable to decode TaskRequest<HttpRequest>; error: {e}.");
        BeamConnectError::ProxyOtherError(e.to_string())
    })
}
