use std::{
    fs::{self, read_to_string},
    path::PathBuf,
    str::FromStr,
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::Result;
use beam_lib::{AppId, AppOrProxyId, set_broker_id};
use clap::Parser;
use hyper::{Uri, header, http::uri::Authority};
use regex::Regex;
use reqwest::{Certificate, Client, Url};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use tokio_native_tls::{
    TlsAcceptor,
    native_tls::{self, Identity},
};
use tracing::warn;

use crate::{errors::BeamConnectError, example_targets};

#[derive(Debug, Clone)]
enum PathOrUri {
    Path(PathBuf),
    Uri(Uri),
}

impl FromStr for PathOrUri {
    type Err = BeamConnectError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match Uri::try_from(s) {
            Ok(uri) => Ok(Self::Uri(uri)),
            Err(..) => {
                let p = PathBuf::from(s);
                if p.is_file() {
                    Ok(Self::Path(p))
                } else {
                    Err(BeamConnectError::ConfigurationError(format!(
                        "Failed to convert {s} to Filepath or Uri"
                    )))
                }
            }
        }
    }
}

/// Settings for Samply.Beam (Shared)
#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
struct CliArgs {
    #[clap(long, env, value_parser)]
    proxy_url: Uri,

    /// Your App ID (e.g. connect1.proxy1.broker)
    #[clap(long, env, value_parser)]
    app_id: String,

    /// Your API Key to the Proxy
    #[clap(long, env, value_parser)]
    proxy_apikey: String,

    /// Bind address
    #[clap(long, env, value_parser, default_value = "0.0.0.0:8062")]
    bind_addr: String,

    /// URL to Service Discovery JSON
    #[clap(long, env, value_parser)]
    discovery_url: PathOrUri,

    /// Path of the local target configuration.
    #[clap(long, env, value_parser)]
    local_targets_file: Option<PathBuf>,

    /// Outgoing HTTP proxy: Directory with CA certificates to trust for TLS connections (e.g. /etc/samply/cacerts/)
    #[clap(long, env, value_parser)]
    tls_ca_certificates_dir: Option<PathBuf>,

    /// Pem file used for tls termination (e.g. the server certificate).
    #[clap(long, env, value_parser)]
    tls_termination_cert_path: Option<String>,

    /// Key file used for tls termination (e.g. the server key).
    #[clap(long, env, value_parser)]
    tls_termination_key_path: Option<String>,

    /// Expiry time of the request in seconds
    #[clap(long, env, value_parser, default_value = "3600")]
    expire: u64,

    /// If set will enable any local apps to authenticate without the `Proxy-Authorization` header.
    /// Security note: This allows any app with network access to beam-connect to send requests to any other beam-connect service in the beam network.
    #[clap(long, env, action)]
    no_auth: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub(crate) struct CentralMapping {
    pub(crate) sites: Vec<Site>,
}

impl CentralMapping {
    pub(crate) fn get(&self, auth: &Authority) -> Option<&Site> {
        for site in &self.sites {
            if site.virtualhost == *auth {
                return Some(site);
            }
        }
        return None;
    }
}

/// Maps an Authority to a given Beam App
#[derive(Serialize, Deserialize, Clone, Debug)]
pub(crate) struct Site {
    pub(crate) id: String,
    pub(crate) name: String,
    #[serde(with = "http_serde::authority")]
    pub(crate) virtualhost: Authority,
    #[serde(with = "http_serde::option::authority", rename = "remapVhost", default, skip_serializing)]
    pub(crate) remap_vhost: Option<Authority>,
    pub(crate) beamconnect: AppId,
}

#[derive(Clone, Deserialize, Debug)]
pub(crate) struct LocalMapping {
    pub(crate) entries: Vec<LocalMappingEntry>,
}
impl LocalMapping {
    pub(crate) fn get(&self, uri: &Uri) -> Option<LocalMappingEntry> {
        for entry in &self.entries {
            if Some(&entry.needle) == uri.authority()
                && entry
                    .external_path
                    .as_ref()
                    .map_or(true, |r| r.is_match(uri.path()))
            {
                return Some(entry.clone());
            }
        }
        return None;
    }
}

/// Maps an external authority to some internal authority if the requesting App is allowed to
#[derive(Clone, Deserialize, Debug)]
pub(crate) struct LocalMappingEntry {
    #[serde(with = "http_serde::authority", rename = "external")]
    pub(crate) needle: Authority, // Host part of URL
    #[serde(rename = "internal")]
    pub(crate) replace: AuthorityReplacement,
    pub(crate) allowed: Vec<AllowListEntry>,
    #[serde(default, rename = "forceHttps")]
    pub(crate) force_https: bool,
    #[serde(default, rename = "resetHost")]
    pub(crate) reset_host: bool,
    #[serde(
        default,
        rename = "externalPathRegex",
        deserialize_with = "deserialize_regex"
    )]
    pub(crate) external_path: Option<Regex>,
}

fn deserialize_regex<'de, D>(deserializer: D) -> Result<Option<Regex>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    match Option::<String>::deserialize(deserializer)? {
        Some(regex_str) => Regex::new(&regex_str)
            .map(Some)
            .map_err(serde::de::Error::custom),
        None => Ok(None),
    }
}

impl LocalMappingEntry {
    pub fn can_be_accessed_by(&self, who: &AppId) -> bool {
        self.allowed.iter().any(|id| match id {
            AllowListEntry::AppOrProxyId(AppOrProxyId::App(app)) => app == who,
            AllowListEntry::AppOrProxyId(AppOrProxyId::Proxy(proxy)) => {
                who.as_ref().ends_with(proxy.as_ref())
            }
            AllowListEntry::Glob(glob) => glob.matches(who.as_ref()),
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct AuthorityReplacement {
    pub authority: Authority,
    pub path: Option<String>,
}

impl From<Authority> for AuthorityReplacement {
    fn from(authority: Authority) -> Self {
        Self {
            authority,
            path: None,
        }
    }
}

impl<'de> Deserialize<'de> for AuthorityReplacement {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let string = String::deserialize(deserializer)?;
        match string.split_once('/') {
            Some((auth, path)) => Ok(Self {
                authority: auth.parse().map_err(serde::de::Error::custom)?,
                path: Some(path.to_owned()),
            }),
            None => Ok(Self {
                authority: string.parse().map_err(serde::de::Error::custom)?,
                path: None,
            }),
        }
    }
}

#[derive(Clone, Debug)]
pub enum CentralTargetsKind {
    Static(CentralMapping),
    Dynamic {
        fetch_url: Url,
        prev: Arc<Mutex<(Instant, CentralMapping)>>,
    },
}

#[derive(Clone)]
#[allow(dead_code)]
pub(crate) struct Config {
    pub(crate) proxy_url: Uri,
    pub(crate) my_app_id: AppId,
    pub(crate) proxy_auth: String,
    pub(crate) bind_addr: String,
    pub(crate) targets_local: LocalMapping,
    pub(crate) targets_public: CentralTargetsKind,
    pub(crate) expire: u64,
    pub(crate) client: Client,
    pub(crate) tls_acceptor: Option<Arc<TlsAcceptor>>,
    pub(crate) no_auth: bool,
}

fn load_local_targets(
    broker_id: &str,
    local_target_path: &Option<PathBuf>,
) -> Result<LocalMapping> {
    if let Some(json_file) = local_target_path {
        if json_file.exists() {
            let json_string = std::fs::read_to_string(json_file)?;
            return Ok(LocalMapping {
                entries: serde_json::from_str(&json_string)?,
            });
        }
    }
    Ok(example_targets::example_local(broker_id))
}

impl CentralTargetsKind {
    async fn new(path_or_uri: &PathOrUri, client: &Client) -> Result<Self, BeamConnectError> {
        match path_or_uri {
            PathOrUri::Path(path) => serde_json::from_slice(&std::fs::read(path).map_err(|e| {
                BeamConnectError::ConfigurationError(format!(
                    "Failed to open central config file: {e}"
                ))
            })?)
            .map_err(|e| {
                BeamConnectError::ConfigurationError(format!(
                    "Failed to parse central config file: {e:#}"
                ))
            })
            .map(CentralTargetsKind::Static),
            PathOrUri::Uri(url) => {
                Self::fetch(client, &url.to_string())
                    .await
                    .map(|(cache_for, central_mapping)| CentralTargetsKind::Dynamic {
                        fetch_url: url.to_string().parse().unwrap(),
                        prev: Arc::new(Mutex::new((Instant::now() + cache_for, central_mapping))),
                    })
            }
        }
    }

    async fn fetch(
        client: &Client,
        fetch_url: &str,
    ) -> Result<(Duration, CentralMapping), BeamConnectError> {
        let res = client.get(fetch_url).send().await.map_err(|e| {
            BeamConnectError::ConfigurationError(format!(
                "Cannot retrieve central service discovery configuration: {e}"
            ))
        })?;
        let cache_for = res
            .headers()
            .get(header::CACHE_CONTROL)
            .and_then(|h| {
                h.to_str().ok()?.split(",").map(str::trim).find_map(|s| {
                    s.strip_prefix("no-cache")
                        .or(s.strip_prefix("no-store"))
                        .map(|_| Duration::ZERO)
                        .or_else(|| {
                            s.strip_prefix("max-age=")?
                                .parse::<u64>()
                                .ok()
                                .map(Duration::from_secs)
                        })
                })
            })
            .unwrap_or(Duration::from_hours(1));
        res.json()
            .await
            .map_err(|e| {
                BeamConnectError::ConfigurationError(format!(
                    "Invalid central site discovery response: {e}"
                ))
            })
            .map(|targets| (cache_for, targets))
    }

    pub async fn get(&self, client: &Client) -> CentralMapping {
        match self {
            CentralTargetsKind::Static(targets) => targets.clone(),
            CentralTargetsKind::Dynamic { fetch_url, prev } => {
                let mut prev = prev.lock().await;
                if prev.0 < Instant::now() {
                    let (cache_for, targets) = match Self::fetch(client, fetch_url.as_str()).await {
                        Ok(res) => res,
                        Err(e) => {
                            tracing::warn!(
                                "Failed to fetch central targets: {e:#}.\nUsing cached targets instead."
                            );
                            return prev.1.clone();
                        }
                    };
                    *prev = (Instant::now() + cache_for, targets.clone());
                    return targets;
                }
                prev.1.clone()
            }
        }
    }
}

fn build_tls_config(
    ssl_cert_pem: Option<&String>,
    ssl_cert_key: Option<&String>,
) -> Result<Option<Arc<TlsAcceptor>>> {
    match (
        ssl_cert_pem.filter(|p| !p.is_empty()),
        ssl_cert_key.filter(|k| !k.is_empty()),
    ) {
        (Some(pem), Some(key)) => {
            let identity = Identity::from_pkcs8(
                read_to_string(pem)?.as_bytes(),
                read_to_string(key)?.as_bytes(),
            )?;
            let tls_acceptor = Arc::new(native_tls::TlsAcceptor::new(identity)?.into());
            Ok(Some(tls_acceptor))
        }
        (Some(_), None) | (None, Some(_)) => {
            anyhow::bail!("SSL certificate PEM and key must both be provided")
        }
        _ => Ok(None),
    }
}

fn build_client(tls_cert_dir: Option<&PathBuf>) -> Result<Client> {
    let mut client_builder = Client::builder();
    if let Some(tls_ca_dir) = tls_cert_dir {
        for path_res in tls_ca_dir.read_dir()? {
            if let Ok(path_buf) = path_res {
                if path_buf.path().is_dir() {
                    continue;
                }
                let cert = match Certificate::from_pem(&fs::read(path_buf.path())?) {
                    Ok(cert) => cert,
                    Err(e) => {
                        warn!("Failed to read cert at {path_buf:?}: {e}");
                        continue;
                    }
                };
                client_builder = client_builder.add_root_certificate(cert);
            }
        }
    }
    Ok(client_builder.build()?)
}

impl Config {
    pub(crate) async fn load() -> Result<Self> {
        let args = CliArgs::parse();
        let broker_id = args.app_id.splitn(3, '.').last().ok_or_else(|| {
            BeamConnectError::ConfigurationError(format!("Invalid beam id: {}", args.app_id))
        })?;
        set_broker_id(broker_id.to_owned());
        let app_id = AppId::new(&args.app_id)?;

        let expire = args.expire;
        let client = build_client(args.tls_ca_certificates_dir.as_ref())?;

        let targets_public = CentralTargetsKind::new(&args.discovery_url, &client).await?;
        let targets_local = load_local_targets(&broker_id, &args.local_targets_file)?;
        let tls_acceptor = build_tls_config(
            args.tls_termination_cert_path.as_ref(),
            args.tls_termination_key_path.as_ref(),
        )?;

        Ok(Config {
            proxy_url: args.proxy_url,
            my_app_id: app_id.clone(),
            proxy_auth: format!("ApiKey {} {}", app_id, args.proxy_apikey),
            bind_addr: args.bind_addr,
            no_auth: args.no_auth,
            targets_local,
            targets_public,
            expire,
            client,
            tls_acceptor,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AllowListEntry {
    Glob(glob::Pattern),
    AppOrProxyId(AppOrProxyId),
}

impl<'de> serde::Deserialize<'de> for AllowListEntry {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        match AppOrProxyId::new(&s) {
            Ok(id) => Ok(AllowListEntry::AppOrProxyId(id)),
            Err(e) => match glob::Pattern::new(&s) {
                Ok(p) => Ok(AllowListEntry::Glob(p)),
                Err(e_glob) => Err(serde::de::Error::custom(format!(
                    "Failed to parse allow list entry as either app or proxy id ({e:#}) nor as a glob pattern ({e_glob:#})"
                ))),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use beam_lib::AppId;
    use beam_lib::set_broker_id;

    use super::*;
    use crate::example_targets::example_local;

    #[test]
    fn allow_list_entry_glob() {
        set_broker_id("broker.ccp-it.dktk.dkfz.de".to_owned());
        let [a, b, c]: [AllowListEntry; 3] = serde_json::from_value(serde_json::json!([
            "*.broker.ccp-it.dktk.dkfz.de",
            "a.broker.ccp-it.dktk.dkfz.de",
            "a.a.broker.ccp-it.dktk.dkfz.de",
        ]))
        .unwrap();
        let AllowListEntry::Glob(glob) = a else {
            panic!("a is not a glob pattern")
        };
        assert!(glob.matches("asdf.broker.ccp-it.dktk.dkfz.de"));
        assert!(!glob.matches("asdf.broker"));
        assert!(matches!(
            b,
            AllowListEntry::AppOrProxyId(AppOrProxyId::Proxy(..))
        ));
        assert!(matches!(
            c,
            AllowListEntry::AppOrProxyId(AppOrProxyId::App(..))
        ));
    }

    #[test]
    fn serde_authority() {
        let serialized = r#"{
            "sites": [
              {
                "id": "UKT",
                "name": "Tübingen",
                "virtualhost": "ukt.virtual",
                "beamconnect": "connect.ukt-proxy.broker.ccp-it.dktk.dkfz.de"
              },
              {
                "id": "UKFR",
                "name": "Freiburg",
                "virtualhost": "ukfr.virtual",
                "beamconnect": "connect.ukfr-proxy.broker.ccp-it.dktk.dkfz.de"
              },
              {
                "id": "UKHD",
                "name": "Heidelberg",
                "virtualhost": "ukhd.virtual",
                "beamconnect": "connect.ukhd-proxy.broker.ccp-it.dktk.dkfz.de"
              },
              {
                "id": "UKU",
                "name": "Ulm",
                "virtualhost": "uku.virtual",
                "beamconnect": "connect.uku-proxy.broker.ccp-it.dktk.dkfz.de"
              }
            ]
          }"#;
        set_broker_id("broker.ccp-it.dktk.dkfz.de".to_owned());
        let obj: CentralMapping = serde_json::from_str(serialized).unwrap();
        assert_eq!(obj.sites.len(), 4);
        let mut routes = obj.sites.iter();

        let site = routes.next().unwrap();
        assert_eq!(site.virtualhost.to_string(), "ukt.virtual");
        assert_eq!(
            site.beamconnect.to_string(),
            "connect.ukt-proxy.broker.ccp-it.dktk.dkfz.de"
        );

        let site = routes.next().unwrap();
        assert_eq!(site.virtualhost, "ukfr.virtual");
        assert_eq!(
            site.beamconnect.to_string(),
            "connect.ukfr-proxy.broker.ccp-it.dktk.dkfz.de"
        );
    }

    #[test]
    fn local_target_configuration() {
        let broker_id = "broker.ccp-it.dktk.dkfz.de";
        set_broker_id(broker_id.to_owned());
        let serialized = r#"[
            {"external": "ifconfig.me","internal":"ifconfig.me/asdf","allowed":["connect1.proxy23.broker.ccp-it.dktk.dkfz.de","connect2.proxy23.broker.ccp-it.dktk.dkfz.de"]},
            {"external": "ip-api.com","internal":"ip-api.com","allowed":["connect1.proxy23.broker.ccp-it.dktk.dkfz.de","connect2.proxy23.broker.ccp-it.dktk.dkfz.de"]},
            {"external": "wttr.in","internal":"wttr.in","allowed":["connect1.proxy23.broker.ccp-it.dktk.dkfz.de","connect2.proxy23.broker.ccp-it.dktk.dkfz.de"]},
            {"external": "node23.uk12.network","internal":"host23.internal.network","allowed":["proxy23.broker.ccp-it.dktk.dkfz.de"]}
        ]"#;
        let obj: LocalMapping = LocalMapping {
            entries: serde_json::from_str(serialized).unwrap(),
        };
        let expect = example_local(&broker_id);
        assert_eq!(obj.entries.len(), expect.entries.len());
        assert!(
            obj.get(&hyper::Uri::from_static("http://node23.uk12.network"))
                .unwrap()
                .can_be_accessed_by(
                    &AppId::new("foobar.proxy23.broker.ccp-it.dktk.dkfz.de").unwrap()
                )
        );

        for (entry, ref_entry) in obj.entries.iter().zip(expect.entries.iter()) {
            assert_eq!(entry.needle, ref_entry.needle);
            assert_eq!(entry.replace, ref_entry.replace);
            assert_eq!(entry.allowed, ref_entry.allowed);
        }
    }
}
