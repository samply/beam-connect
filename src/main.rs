use std::{convert::Infallible, time::Duration};

use config::Config;
use http_body_util::combinators::BoxBody;
use hyper::{
    Method, Request,
    body::{Bytes, Incoming},
    service::service_fn,
};
use hyper_util::{
    rt::{TokioExecutor, TokioIo},
    server,
};
use logic_ask::handler_http;
use logic_reply::poller;
use tokio::{net::TcpListener, task::JoinHandle};
use tracing::{debug, error, info, warn};
use tracing_subscriber::{EnvFilter, filter::LevelFilter};

mod banner;
mod config;
mod errors;
mod example_targets;
mod logic_ask;
mod logic_reply;
mod msg;
mod shutdown;
#[cfg(feature = "sockets")]
mod sockets;
mod structs;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing::subscriber::set_global_default(
        tracing_subscriber::fmt()
            .with_env_filter(
                EnvFilter::builder()
                    .with_default_directive(LevelFilter::INFO.into())
                    .from_env_lossy(),
            )
            .finish(),
    )?;
    banner::print_banner();
    let config = Config::load().await?;
    let config: &'static _ = Box::leak(Box::new(config));
    banner::print_startup_app_config(&config).await;

    info!("Global site discovery: {:?}", config.targets_public);
    info!("Local site Access: {:?}", config.targets_local);

    let mut executers = vec![];
    if !config.targets_local.entries.is_empty() {
        executers.push(tokio::spawn(poller(move || {
            logic_reply::poll_and_execute_task(config)
        })));
        #[cfg(feature = "sockets")]
        executers.push(tokio::spawn(poller(move || {
            sockets::poll_and_execute_socket_task(config)
        })));
    } else {
        info!("No local targets configured, will not poll for tasks.");
    };

    if let Err(e) = server(config).await {
        error!("Server error: {}", e);
    }
    info!("Shutting down...");
    executers.iter().for_each(JoinHandle::abort);
    Ok(())
}

// See https://github.com/hyperium/hyper-util/blob/master/examples/server_graceful.rs
async fn server(config: &'static Config) -> anyhow::Result<()> {
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

                let conn = server.serve_connection_with_upgrades(stream, service_fn(move |req| {
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
    config: &'static Config,
) -> Result<Response, Infallible> {
    // On https connections we want to emulate that we successfully connected to get the actual http request
    if req.method() == Method::CONNECT
        && let Some(tls_acceptor) = &config.tls_acceptor
    {
        tokio::spawn(async move {
            let authority = req.uri().authority().cloned();
            match hyper::upgrade::on(req).await {
                Ok(connection) => {
                    let tls_connection = match tls_acceptor.accept(TokioIo::new(connection)).await {
                        Ok(s) => s,
                        Err(e) => {
                            warn!("Error accepting tls connection: {e}");
                            return;
                        }
                    };
                    server::conn::auto::Builder::new(TokioExecutor::new())
                        .serve_connection_with_upgrades(
                            TokioIo::new(tls_connection),
                            service_fn(|req| {
                                let authority = authority.clone();
                                async move {
                                    match handler_http(req, config, authority).await {
                                        Ok(e) => Ok::<_, Infallible>(e),
                                        Err(e) => Ok(Response::builder()
                                            .status(e.code)
                                            .body(BoxBody::default())
                                            .unwrap()),
                                    }
                                }
                            }),
                        )
                        .await
                        .unwrap_or_else(|e| warn!("Failed to handle upgraded connection: {e}"));
                }
                Err(e) => warn!("Failed to upgrade connection: {e}"),
            };
        });
        Ok(Response::new(BoxBody::default()))
    } else {
        match handler_http(req, config, None).await {
            Ok(e) => Ok(e),
            Err(e) => Ok(Response::builder()
                .status(e.code)
                .body(BoxBody::default())
                .unwrap()),
        }
    }
}

// Can't use AsyncFnMut closure here without -Zhigher-ranked-assumptions :sad:
pub async fn retry_beam_req<F: Fn() -> Fut, Fut>(
    req_fn: F,
    retries: usize,
) -> reqwest::Result<reqwest::Response>
where
    Fut: Send,
    Fut: Future<Output = reqwest::Result<reqwest::Response>>,
{
    let mut tries = 0;
    loop {
        tries += 1;
        let resp = match req_fn().await {
            Ok(resp) => resp,
            Err(e) if e.is_timeout() && tries < retries => {
                warn!("Timeout requesting beam: {e:#?}. Retrying");
                continue;
            }
            Err(e) => break Err(e),
        };
        tracing::trace!("Got beam reply: {resp:#?}");

        match resp.error_for_status_ref() {
            Ok(_) => break Ok(resp),
            Err(_) if tries > retries => {
                warn!(
                    "Error requesting beam, got code: {}. Retrying",
                    resp.status()
                );
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
            Err(e) => {
                warn!(
                    "Error requesting beam, got code: {}. Giving up",
                    resp.status()
                );
                break Err(e);
            }
        }
    }
}
