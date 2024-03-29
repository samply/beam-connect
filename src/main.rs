use std::{net::SocketAddr, str::FromStr, convert::Infallible, error::Error, sync::Arc, time::Duration};

use config::Config;
use hyper::{body, Body, service::{service_fn, make_service_fn}, Request, Response, Server, server::conn::{AddrStream, Http}, Method};
use logic_ask::handler_http;
use tracing::{info, debug, warn};
use tracing_subscriber::{EnvFilter, filter::LevelFilter};

use crate::errors::BeamConnectError;

mod shutdown;
mod msg;
mod example_targets;
mod config;
mod errors;
mod structs;
mod logic_ask;
mod logic_reply;
mod banner;
#[cfg(feature = "sockets")]
mod sockets;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>>{
    tracing::subscriber::set_global_default(tracing_subscriber::fmt().with_env_filter(EnvFilter::builder().with_default_directive(LevelFilter::INFO.into()).from_env_lossy()).finish())?;
    banner::print_banner();
    let config = Config::load().await?;
    let config2 = config.clone();
    let listen = SocketAddr::from_str(&config2.bind_addr).unwrap();
    let client = config.client.clone();
    let client2 = client.clone();
    banner::print_startup_app_config(&config);

    info!("Global site discovery: {:?}", config.targets_public);
    info!("Local site Access: {:?}", config.targets_local);


    let http_executor = tokio::task::spawn(async move {
        loop {
            debug!("Waiting for next request ...");
            if let Err(e) = logic_reply::process_requests(config2.clone(), client2.clone()).await {
                match e {
                    BeamConnectError::ProxyTimeoutError => {
                        debug!("{e}");
                    },
                    BeamConnectError::ProxyReqwestError(e) => {
                        warn!("Error reaching beam proxy: {e}");
                        tokio::time::sleep(Duration::from_secs(10)).await;
                    }
                    _ => {
                        warn!("Error in processing request: {e}. Will continue with the next one.");
                    }
                }
            }
        }
    });
    #[cfg(feature = "sockets")]
    sockets::spawn_socket_task_poller(config.clone());

    let config = Arc::new(config.clone());

    let make_service = make_service_fn(|_conn: &AddrStream| {
        // let remote_addr = conn.remote_addr();
        let config = config.clone();
        async {
            Ok::<_, Infallible>(service_fn(move |req|
                handler_http_wrapper(req, config.clone())))
        }
    });

    let server = Server::bind(&listen)
        .serve(make_service)
        .with_graceful_shutdown(crate::shutdown::wait_for_signal());

    if let Err(e) = server.await {
        eprintln!("server error: {}", e);
    }
    info!("(2/2) Shutting down gracefully ...");
    http_executor.abort();
    Ok(())
}

pub(crate) async fn handler_http_wrapper(
    req: Request<Body>,
    config: Arc<Config>,
) -> Result<Response<Body>, Infallible> {
    // On https connections we want to emulate that we successfully connected to get the actual http request
    if req.method() == Method::CONNECT {
        tokio::spawn(async move {
            let authority = req.uri().authority().cloned();
            match hyper::upgrade::on(req).await {
                Ok(connection) => {
                    let tls_connection = match config.tls_acceptor.accept(connection).await {
                        Err(e) => {
                            warn!("Error accepting tls connection: {e}");
                            return;
                        },
                        Ok(s) => s,
                    };
                    Http::new().serve_connection(tls_connection, service_fn(|req| {
                        let config = config.clone();
                        let authority = authority.clone();
                        async move {
                            match handler_http(req, config, authority).await {
                                Ok(e) => Ok::<_, Infallible>(e),
                                Err(e) => Ok(Response::builder().status(e.code).body(body::Body::empty()).unwrap()),
                            }
                        }
                    })).await.unwrap_or_else(|e| warn!("Failed to handle upgraded connection: {e}"));
                },
                Err(e) => warn!("Failed to upgrade connection: {e}"),
            };
        });
        Ok(Response::new(Body::empty()))
    } else {
        match handler_http(req, config, None).await {
            Ok(e) => Ok(e),
            Err(e) => Ok(Response::builder().status(e.code).body(body::Body::empty()).unwrap()),
        }
    }

}
