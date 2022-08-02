use hyper::{Client, client::HttpConnector, Request, header, StatusCode, body, Response, Body};
use log::{info, warn, debug};
use serde_json::Value;
use shared::{MsgTaskRequest, MsgTaskResult, MsgId};

use crate::{config::Config, errors::BeamConnectError, msg::{IsValidHttpTask, HttpResponse}};

pub(crate) async fn process_requests(config: Config, client: Client<HttpConnector>) -> Result<(), BeamConnectError> {
    // Fetch tasks from Proxy
    let msgs = fetch_requests(&config, &client).await?;

    for task in msgs {
        let resp = execute_http_task(&task, &client).await?;

        send_reply(&task, &config, &client, resp).await?;
    }

    Ok(())
}

async fn send_reply(task: &MsgTaskRequest, config: &Config, client: &Client<HttpConnector>, mut resp: Response<Body>) -> Result<(), BeamConnectError> {
    let body = body::to_bytes(resp.body_mut()).await
        .map_err(BeamConnectError::FailedToReadTargetsReply)?;
    let http_reply = HttpResponse {
        status: resp.status(),
        headers: resp.headers().clone(),
        body: String::from_utf8(body.to_vec())?
    };
    let http_reply = serde_json::to_string(&http_reply)?;
    let msg = MsgTaskResult {
        id: MsgId::new(),
        from: config.my_app_id.clone().into(),
        to: vec![task.from.clone()],
        task: task.id,
        status: shared::WorkStatus::Succeeded(http_reply),
        metadata: Value::Null
    };
    let req_to_proxy = Request::builder()
        .method("POST")
        .uri(format!("{}v1/tasks/{}/results", config.proxy_url, task.id))
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

async fn execute_http_task(task: &MsgTaskRequest, client: &Client<HttpConnector>) -> Result<Response<Body>, BeamConnectError> {
    let task_req = task.http_request()?;
    info!("{} | {} {}", task.from, task_req.method, task_req.url);
    let mut req = Request::builder()
        .method(task_req.method)
        .uri(task_req.url);
    *req.headers_mut().unwrap() = task_req.headers;
    let body = body::Body::from(task_req.body);
    let req = 
        req.body(body)
        .map_err(BeamConnectError::HyperBuildError)?;
    debug!("Issuing request: {:?}", req);
    let resp = client.request(req).await
        .map_err(|e| BeamConnectError::CommunicationWithTargetFailed(e.to_string()))?;
    Ok(resp)
}

async fn fetch_requests(config: &Config, client: &Client<HttpConnector>) -> Result<Vec<MsgTaskRequest>, BeamConnectError> {
    let req_to_proxy = Request::builder()
        .uri(format!("{}v1/tasks?to={}&poll_count=1&unanswered=true", config.proxy_url, config.my_app_id))
        .header(header::AUTHORIZATION, config.proxy_auth.clone())
        .body(body::Body::empty())
        .map_err(BeamConnectError::HyperBuildError)?;
    let mut resp = client.request(req_to_proxy).await
        .map_err(BeamConnectError::ProxyHyperError)?;
    match resp.status() {
        StatusCode::OK => {
            info!("Got request: {:?}", resp);
        },
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
