use std::{error::Error, str::FromStr, path::PathBuf};

use clap::Parser;
use hyper::{Uri, http::uri::Authority, client::HttpConnector, Client};
use hyper_proxy::ProxyConnector;
use hyper_tls::HttpsConnector;
use serde::{Deserialize, Deserializer, de::Visitor};
use shared::{beam_id::{AppId, BeamId, app_to_broker_id, BrokerId}, http_proxy::build_hyper_client};

use crate::{example_targets};

/// Settings for Samply.Beam (Shared)
#[derive(Parser,Debug)]
#[clap(author, version, about, long_about = None)]
struct CliArgs {
    #[clap(long, env, value_parser)]
    proxy_url: Uri,

    /// Your short App ID (e.g. connect1)
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
    discovery_url: Uri,

    /// Outgoing HTTP proxy: Directory with CA certificates to trust for TLS connections (e.g. /etc/samply/cacerts/)
    #[clap(long, env, value_parser)]
    tls_ca_certificates_dir: Option<PathBuf>,
}

#[derive(Deserialize,Clone,Debug)]
pub(crate) struct CentralMapping {
    pub(crate) sites: Vec<Site>
}

impl CentralMapping {
    pub(crate) fn get(&self, auth: &Authority) -> Option<Site> {
        for site in &self.sites {
            if site.virtualhost == *auth {
                return Some(site.clone())
            }
        }
        return None
    }
}

#[derive(Deserialize,Clone,Debug)]
pub(crate) struct Site {
    pub(crate) id: String,
    pub(crate) name: String,
    #[serde(deserialize_with = "deserialize_authority")]
    pub(crate) virtualhost: Authority,
    pub(crate) beamconnect: AppId,
}

fn deserialize_authority<'de, D>(deserializer: D) -> Result<Authority, D::Error> 
where D: Deserializer<'de> {
    deserializer.deserialize_str(AuthorityVisitor {})
}

struct AuthorityVisitor { }

impl<'de> Visitor<'de> for AuthorityVisitor {
    type Value = Authority;

    fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(formatter, "Authority part of a URL")
    }

    fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
        where
            E: serde::de::Error, {
        let auth = Authority::from_str(v)
            .map_err(|e| serde::de::Error::invalid_value(serde::de::Unexpected::Str(v), &self))?;
        Ok(auth)
    }
}

#[cfg(test)]
mod tests {
    use super::CentralMapping;

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
        let obj: CentralMapping = serde_json::from_str(serialized).unwrap();
        assert_eq!(obj.sites.len(), 4);
        let mut routes = obj.sites.iter();

        let site = routes.next().unwrap();
        assert_eq!(site.virtualhost.to_string(), "ukt.virtual");
        assert_eq!(site.beamconnect.to_string(), "connect.ukt-proxy.broker.ccp-it.dktk.dkfz.de");

        let site = routes.next().unwrap();
        assert_eq!(site.virtualhost, "ukfr.virtual");
        assert_eq!(site.beamconnect.to_string(), "connect.ukfr-proxy.broker.ccp-it.dktk.dkfz.de");
    }
}

#[derive(Clone)]
pub(crate) struct LocalMappingEntry {
    pub(crate) needle: Authority, // Host part of URL
    pub(crate) replace: Authority,
    pub(crate) allowed: Vec<AppId>
}

#[derive(Clone)]
#[allow(dead_code)]
pub(crate) struct Config {
    pub(crate) proxy_url: Uri,
    pub(crate) my_app_id: AppId,
    pub(crate) proxy_auth: String,
    pub(crate) bind_addr: String,
    pub(crate) targets_local: Vec<LocalMappingEntry>,
    pub(crate) targets_public: CentralMapping,
    pub(crate) client: Client<ProxyConnector<HttpsConnector<HttpConnector>>>
}

fn load_local_targets(broker_id: &BrokerId) -> Vec<LocalMappingEntry> {
    example_targets::example_local(broker_id) //TODO: Read from env, file, etc.
}

async fn load_public_targets(client: &Client<ProxyConnector<HttpsConnector<HttpConnector>>>, url: &Uri) -> Result<CentralMapping,Box<dyn Error>> {
    let mut response = client.get(url.clone()).await?;
    let body = response.body_mut();
    let bytes = hyper::body::to_bytes(body).await?;
    let deserialized = serde_json::from_slice::<CentralMapping>(&bytes)?;
    Ok(deserialized)
}

impl Config {
    pub(crate) async fn load() -> Result<Self,Box<dyn Error>> {
        let args = CliArgs::parse();
        let broker_id = app_to_broker_id(&args.app_id)?;
        AppId::set_broker_id(&broker_id);
        let my_app_id = AppId::new(&args.app_id)?;
        let broker_id = BrokerId::new(&broker_id)?;

        let tls_ca_certificates = shared::crypto::load_certificates_from_dir(args.tls_ca_certificates_dir)?;
        let client = build_hyper_client(tls_ca_certificates)?;

        let targets_public = load_public_targets(&client, &args.discovery_url).await?;

        Ok(Config {
            proxy_url: args.proxy_url,
            my_app_id: my_app_id.clone(),
            proxy_auth: format!("ApiKey {} {}", my_app_id, args.proxy_apikey),
            bind_addr: args.bind_addr,
            targets_local: load_local_targets(&broker_id),
            targets_public,
            client
        })
    }
}

