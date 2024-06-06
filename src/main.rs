use std::{convert::Infallible, sync::Arc, time::Duration};

use config::Config;
use http_body_util::combinators::BoxBody;
use hyper::{body::{Bytes, Incoming}, service::service_fn, Method, Request};
use hyper_util::{rt::{TokioExecutor, TokioIo}, server};
use logic_ask::handler_http;
use tokio::{net::TcpListener, task::JoinHandle};
use tracing::{debug, error, info, warn};
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
async fn main() -> anyhow::Result<()> {
    tracing::subscriber::set_global_default(tracing_subscriber::fmt().with_env_filter(EnvFilter::builder().with_default_directive(LevelFilter::INFO.into()).from_env_lossy()).finish())?;
    banner::print_banner();
    let config = Config::load().await?;
    let config2 = config.clone();
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
    #[allow(unused_mut)]
    let mut executers = vec![http_executor];
    #[cfg(feature = "sockets")]
    executers.push(sockets::spawn_socket_task_poller(config.clone()));

    let config = Arc::new(config.clone());

    if let Err(e) = server(&config).await {
        error!("Server error: {}", e);
    }
    info!("Shutting down...");
    executers.iter().for_each(JoinHandle::abort);
    Ok(())
}

// See https://github.com/hyperium/hyper-util/blob/master/examples/server_graceful.rs
async fn server(config: &Arc<Config>) -> anyhow::Result<()> {
    let listener = TcpListener::bind(config.bind_addr.clone()).await?;

    let server = hyper_util::server::conn::auto::Builder::new(TokioExecutor::new());
    let graceful = hyper_util::server::graceful::GracefulShutdown::new();
    let mut ctrl_c = std::pin::pin!(crate::shutdown::wait_for_signal());

    loop {
        tokio::select! {
            conn = listener.accept() => {
                let (stream, peer_addr) = match conn {
                    Ok(conn) => conn,
                    Err(e) => {
                        warn!("accept error: {}", e);
                        tokio::time::sleep(Duration::from_secs(1)).await;
                        continue;
                    }
                };
                debug!("incomming connection accepted: {}", peer_addr);

                let stream = hyper_util::rt::TokioIo::new(stream);

                let config = config.clone();
                let conn = server.serve_connection_with_upgrades(stream, service_fn(move |req| {
                    let config = config.clone();
                    handler_http_wrapper(req, config)
                }));

                let conn = graceful.watch(conn.into_owned());

                tokio::spawn(async move {
                    if let Err(err) = conn.await {
                        warn!("connection error: {}", err);
                    }
                    debug!("Connection dropped: {}", peer_addr);
                });
            },

            _ = ctrl_c.as_mut() => {
                drop(listener);
                info!("Ctrl-C received, starting shutdown");
                break;
            }
        }
    }

    tokio::select! {
        _ = graceful.shutdown() => {
            info!("Gracefully shutdown!");
        },
        _ = tokio::time::sleep(Duration::from_secs(5)) => {
            warn!("Waited 5 seconds for graceful shutdown, aborting...");
        }
    }

    Ok(())
}

pub type Response<T = BoxBody<Bytes, anyhow::Error>> = hyper::Response<T>;

pub(crate) async fn handler_http_wrapper(
    req: Request<Incoming>,
    config: Arc<Config>,
) -> Result<Response, Infallible> {
    // On https connections we want to emulate that we successfully connected to get the actual http request
    if req.method() == Method::CONNECT {
        tokio::spawn(async move {
            let authority = req.uri().authority().cloned();
            match hyper::upgrade::on(req).await {
                Ok(connection) => {
                    let tls_connection = match config.tls_acceptor.accept(TokioIo::new(connection)).await {
                        Err(e) => {
                            warn!("Error accepting tls connection: {e}");
                            return;
                        },
                        Ok(s) => s,
                    };
                    server::conn::auto::Builder::new(TokioExecutor::new()).serve_connection_with_upgrades(TokioIo::new(tls_connection), service_fn(|req| {
                        let config = config.clone();
                        let authority = authority.clone();
                        async move {
                            match handler_http(req, config, authority).await {
                                Ok(e) => Ok::<_, Infallible>(e),
                                Err(e) => Ok(Response::builder().status(e.code).body(BoxBody::default()).unwrap()),
                            }
                        }
                    })).await.unwrap_or_else(|e| warn!("Failed to handle upgraded connection: {e}"));
                },
                Err(e) => warn!("Failed to upgrade connection: {e}"),
            };
        });
        Ok(Response::new(BoxBody::default()))
    } else {
        match handler_http(req, config, None).await {
            Ok(e) => Ok(e),
            Err(e) => Ok(Response::builder().status(e.code).body(BoxBody::default()).unwrap()),
        }
    }

}
