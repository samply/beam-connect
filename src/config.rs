use std::{error::Error, str::FromStr, path::PathBuf};

use clap::Parser;
use hyper::{Uri, http::uri::Authority, client::HttpConnector, Client};
use hyper_proxy::ProxyConnector;
use hyper_tls::HttpsConnector;
use serde::{Deserialize, Deserializer, de::Visitor};
use shared::{beam_id::{AppId, BeamId, app_to_broker_id, BrokerId}, http_proxy::build_hyper_client};

use crate::{example_targets, errors::BeamConnectError};

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

    /// Path of the local target configuration.
    #[clap(long, env, value_parser)]
    local_targets_file: Option<PathBuf>,

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


#[derive(Clone,Deserialize,Debug)]
pub(crate) struct LocalMapping {
    pub(crate) entries: Vec<LocalMappingEntry>
}
impl LocalMapping {
    pub(crate) fn get(&self, auth: &Authority) -> Option<LocalMappingEntry> {
        for entry in &self.entries {
            if entry.needle == *auth {
                return Some(entry.clone())
            }
        }
        return None
    }
}

#[derive(Clone,Deserialize,Debug)]
pub(crate) struct LocalMappingEntry {
    #[serde(deserialize_with = "deserialize_authority", rename="external")]
    pub(crate) needle: Authority, // Host part of URL
    #[serde(deserialize_with = "deserialize_authority", rename="internal")]
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
    pub(crate) targets_local: LocalMapping,
    pub(crate) targets_public: CentralMapping,
    pub(crate) client: Client<ProxyConnector<HttpsConnector<HttpConnector>>>
}

fn load_local_targets(broker_id: &BrokerId, local_target_path: &Option<PathBuf>) -> Result<LocalMapping,Box<dyn Error>> {
    if let Some(json_file) = local_target_path {
        if json_file.exists() {
            let json_string = std::fs::read_to_string(json_file)?;
            return Ok(serde_json::from_str::<LocalMapping>(&json_string)?);
        }
    }
    Ok(example_targets::example_local(broker_id))
}

async fn load_public_targets(client: &Client<ProxyConnector<HttpsConnector<HttpConnector>>>, url: &Uri) -> Result<CentralMapping,BeamConnectError> {
    let mut response = client.get(url.clone()).await.map_err(|e| BeamConnectError::ConfigurationError(format!("Cannot retreive central service discovery configuration: {}",e)))?;
    let body = response.body_mut();
    let bytes = hyper::body::to_bytes(body).await.map_err(|e|BeamConnectError::ConfigurationError(format!("Invalid central site discovery response: {}",e)))?;
    let deserialized = serde_json::from_slice::<CentralMapping>(&bytes).map_err(|e|BeamConnectError::ConfigurationError(format!("Cannot parse central service discovery configuration: {}", e)))?;
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
        let targets_local = load_local_targets(&broker_id, &args.local_targets_file)?;

        Ok(Config {
            proxy_url: args.proxy_url,
            my_app_id: my_app_id.clone(),
            proxy_auth: format!("ApiKey {} {}", my_app_id, args.proxy_apikey),
            bind_addr: args.bind_addr,
            targets_local,
            targets_public,
            client
        })
    }
}

#[cfg(test)]
mod tests {
    use super::CentralMapping;
    use super::LocalMappingEntry;
    use crate::example_targets::example_local;
    use shared::beam_id::{BrokerId,BeamId,app_to_broker_id};

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

    #[test]
    fn local_target_configuration() {
        let broker_id = app_to_broker_id("foo.bar.broker.example").unwrap();
        BrokerId::set_broker_id(&broker_id);
        let broker_id = BrokerId::new(&broker_id).unwrap();
        let serialized = r#"[
            {"external": "ifconfig.me","internal":"ifconfig.me","allowed":["connect1.proxy23.broker.example","connect2.proxy23.broker.example"]},
            {"external": "ip-api.com","internal":"ip-api.com","allowed":["connect1.proxy23.broker.example","connect2.proxy23.broker.example"]},
            {"external": "wttr.in","internal":"wttr.in","allowed":["connect1.proxy23.broker.example","connect2.proxy23.broker.example"]},
            {"external": "node23.uk12.network","internal":"host23.internal.network","allowed":["connect1.proxy23.broker.example","connect2.proxy23.broker.example"]}
        ]"#;
        let obj: LocalMapping = serde_json::from_str(serialized).unwrap();
        let expect = example_local(&broker_id);
        assert_eq!(obj.len(), expect.len());

        for (entry,ref_entry) in obj.entries.iter().zip(expect.iter()) {
            assert_eq!(entry.needle,ref_entry.needle);
            assert_eq!(entry.replace,ref_entry.replace);
            assert_eq!(entry.allowed,ref_entry.allowed);
        }
    }
}
