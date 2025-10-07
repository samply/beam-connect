use std::{pin::pin, sync::Arc};

use beam_lib::{AppOrProxyId, TaskRequest, TaskResult, WorkStatus};
use futures_util::future::TryJoinAll;
use hyper::{header, StatusCode, Uri, Method, http::uri::PathAndQuery};
use tracing::{debug, field, info, trace, warn, Instrument, Span};
use serde_json::Value;
use reqwest::Response;

use crate::{config::Config, errors::BeamConnectError, msg::{HttpResponse, HttpRequest}};

pub(crate) async fn process_requests(config: &'static Config) -> Result<(), BeamConnectError> {
    // Fetch a batch of tasks and executed them in parallel
    fetch_task(&config)
        .await?
        .into_iter()
        .map(|task| claim_or_answer(task, config))
        .collect::<TryJoinAll<_>>()
        .await?;
    Ok(())
}

#[tracing::instrument(skip_all, fields(from = %task.from.hide_broker(), method = %task.body.method, orig_url = %task.body.url, dst_url))]
async fn claim_or_answer(task: TaskRequest<HttpRequest>, config: &'static Config) -> Result<(), BeamConnectError> {
    let task = Arc::new(task);
    let task2 = Arc::clone(&task);
    let mut execute_task = Box::pin(async move {
        execute_http_task(&task2, &config).await
    });
    let mut claim_task = pin!(claim_task(&task, &config));
    tokio::select! {
        claimed = &mut claim_task => {
           claimed?; 
           let task = Arc::clone(&task);
           tokio::spawn(async move {
                if let Err(e) = send_reply(&task, &config, execute_task.await).await {
                    warn!("Failed to send execution result: {e}");
                }
           }.instrument(Span::current()));
           Ok(())
        },
        resp = &mut execute_task => {
            send_reply(&task, &config, resp).await
        }
    }
}

async fn claim_task<T>(task: &TaskRequest<T>, config: &Config) -> Result<(), BeamConnectError> {
    let msg = TaskResult {
        from: config.my_app_id.clone().into(),
        to: vec![task.from.clone()],
        task: task.id,
        status: WorkStatus::Claimed,
        metadata: Value::Null,
        body: (),
    };
    debug!("Claiming: {msg:?}");
    let resp = config.client
        .put(format!("{}v1/tasks/{}/results/{}", config.proxy_url, task.id, config.my_app_id.clone()))
        .header(header::AUTHORIZATION, config.proxy_auth.clone())
        .json(&msg)
        .send()
        .await
        .map_err(BeamConnectError::ProxyReqwestError)?;

    if let StatusCode::CREATED | StatusCode::NO_CONTENT = resp.status() {
        Ok(())
    } else {
        Err(BeamConnectError::ProxyOtherError(format!("Got error code {} trying to submit our result.", resp.status())))
    }
}

async fn send_reply(task: &TaskRequest<HttpRequest>, config: &Config, resp: Result<Response, BeamConnectError>) -> Result<(), BeamConnectError> {
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
    let resp = config.client
        .put(format!("{}v1/tasks/{}/results/{}", config.proxy_url, task.id, config.my_app_id.clone()))
        .header(header::AUTHORIZATION, config.proxy_auth.clone())
        .json(&msg)
        .send()
        .await
        .map_err(BeamConnectError::ProxyReqwestError)?;

    if let StatusCode::CREATED | StatusCode::NO_CONTENT = resp.status() {
        Ok(())
    } else {
        Err(BeamConnectError::ProxyOtherError(format!("Got error code {} trying to submit our result.", resp.status())))
    }
}

async fn execute_http_task(task: &TaskRequest<HttpRequest>, config: &Config) -> Result<Response, BeamConnectError> {
    let task_req = &task.body;
    let target = config
        .targets_local
        .get(&task_req.url) 
        .ok_or_else(|| BeamConnectError::NoLocalMapping(task_req.url.clone()))?;
    match &task.from {
        AppOrProxyId::App(app) if target.can_be_accessed_by(app) => {},
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

    Span::current().record("dst_url", field::display(&uri));
    let mut headers = task_req.headers.clone();
    if target.reset_host {
        // This will lead to reqwest generating a new HOST header coresponding to the new hostname of the url.
        // If we don't do this it can lead to problems with reverse proxies because
        // they will look at the old HOST header (the virtual host) in order to route the request.
        tracing::trace!("Resetting host header");
        headers.remove(header::HOST);
    }
    info!("Executing");
    let resp = config.client
        .request(task_req.method.clone(), uri.to_string())
        .headers(headers)
        .body(task_req.body.to_vec())
        .send()
        .await
        .map_err(BeamConnectError::CommunicationWithTargetFailed)?;
    Ok(resp)
}

async fn fetch_task(config: &Config) -> Result<Vec<TaskRequest<HttpRequest>>, BeamConnectError> {
    debug!("fetching requests from proxy");
    let resp = config.client
        .get(format!("{}v1/tasks?to={}&wait_count=1&filter=todo", config.proxy_url, config.my_app_id))
        .header(header::AUTHORIZATION, config.proxy_auth.clone())
        .header(header::ACCEPT, "application/json")
        .send()
        .await
        .map_err(BeamConnectError::ProxyReqwestError)?;
    match resp.status() {
        StatusCode::OK => {
            trace!("Got tasks from beam: {resp:#?}");
        },
        StatusCode::GATEWAY_TIMEOUT => return Err(BeamConnectError::ProxyTimeoutError),
        StatusCode::UNAUTHORIZED => return Err(BeamConnectError::ProxyRejectedAuthorization),
        _ => {
            return Err(BeamConnectError::ProxyOtherError(format!("Got response code {}", resp.status())));
        }
    }
    resp.json::<Vec<TaskRequest<HttpRequest>>>().await.map_err(|e| {
        warn!("Unable to decode TaskRequest<HttpRequest>; error: {e}.");
        BeamConnectError::ProxyOtherError(e.to_string())
    }).map_err(Into::into)
}
