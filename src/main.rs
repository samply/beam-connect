use std::{net::SocketAddr, str::FromStr, convert::Infallible, string::FromUtf8Error, error::Error, fmt::Display, collections::{hash_map, HashMap}, sync::Arc};

use config::Config;
use hyper::{body, Body, service::{service_fn, make_service_fn}, Request, Response, Server, header::{HeaderName, self, ToStrError}, Uri, http::uri::Authority, server::conn::AddrStream, Client, client::HttpConnector};
use hyper_proxy::ProxyConnector;
use hyper_tls::HttpsConnector;
use log::{info, error, debug, warn};
use shared::beam_id::AppId;
use structs::InternalHost;

mod msg;
mod example_targets;
mod config;
mod errors;
mod structs;
mod logic_ask;
mod logic_reply;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>>{
    pretty_env_logger::init();
    let listen = SocketAddr::from_str("0.0.0.0:8082").unwrap();

    let config = Config::load()?;
    let config2 = config.clone();
    let client = shared::http_proxy::build_hyper_client(config.http_proxy.clone(), config.tls_ca_certificates.clone())?;
    //let client = hyper::Client::builder().build_http();
    let client2 = client.clone();

    let http_executor = tokio::task::spawn(async move {
        loop {
            debug!("Waiting for next request ...");
            if let Err(e) = logic_reply::process_requests(config2.clone(), &client2.clone()).await {
                warn!("Error in processing request: {e}. Will continue with the next one.");
            }
        }
    });

    let targets = Arc::new(example_targets::get_examples());
    let config = Arc::new(config.clone());

    let make_service = 
        make_service_fn(|_conn: &AddrStream| {
            // let remote_addr = conn.remote_addr();
            let client = client.clone();
            let targets = targets.clone();
            let config = config.clone();
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

async fn handler_http_wrapper(
    req: Request<Body>,
    config: Arc<Config>,
    client: Client<ProxyConnector<HttpsConnector<HttpConnector>>>,
    targets: Arc<HashMap<InternalHost, AppId>>
) -> Result<Response<Body>, Infallible> {
    match logic_ask::handler_http(req, config, client, targets).await {
        Ok(e) => Ok(e),
        Err(e) => Ok(Response::builder().status(e.code).body(body::Body::empty()).unwrap()),
    }
}

async fn shutdown_signal() {
    // Wait for the CTRL+C signal
    info!("Starting ...");
    tokio::signal::ctrl_c()
        .await
        .expect("failed to install CTRL+C signal handler");
    info!("(1/2) Shutting down gracefully ...");
}