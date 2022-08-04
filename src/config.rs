use std::{error::Error, collections::HashMap};

use clap::Parser;
use hyper::{Uri, http::uri::Authority};
use shared::beam_id::{AppId, BeamId, app_to_broker_id, BrokerId};

use crate::{example_targets};

/// Settings for Samply.Beam (Shared)
#[derive(Parser,Debug)]
#[clap(author, version, about, long_about = None)]
struct CliArgs {
    /// samply.pki: URL to HTTPS endpoint
    // #[clap(long, env, value_parser)]
    // pki_address: Uri,

    /// samply.pki: Authentication realm
    // #[clap(long, env, value_parser, default_value = "samply_pki")]
    // pki_realm: String,

    /// samply.pki: File containing the authentication token
    // #[clap(long, env, value_parser, default_value = "/run/secrets/pki.secret")]
    // pki_apikey_file: PathBuf,

    /// samply.pki: Path to own secret key
    // #[clap(long, env, value_parser, default_value = "/run/secrets/privkey.pem")]
    // privkey_file: PathBuf,

    // TODO: The following arguments have been added for compatibility reasons with the proxy config. Find another way to merge configs.
    /// (included for technical reasons)
    // #[clap(long, env, value_parser)]
    // broker_url: Uri,

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

    // /// (included for technical reasons)
    // #[clap(long, env, value_parser)]
    // proxy_id: Option<String>,

    // /// (included for technical reasons)
    // #[clap(action)]
    // examples: Option<String>,

    // /// (included for technical reasons)
    // #[clap(long,hide(true))]
    // test_threads: Option<String>
}

type UrlBeginning = String;

#[derive(Clone)]
pub(crate) struct CentralMapping {
    pub(crate) map: HashMap<Authority, AppId>
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
    // pub(crate) pki_address: Uri,
    // pub(crate) pki_realm: String,
    // pub(crate) pki_apikey: String,
    // pub(crate) privkey_rs256: RS256KeyPair,
    // pub(crate) privkey_rsa: RsaPrivateKey,
    // pub(crate) http_proxy: Option<Uri>,
    // // pub(crate) broker_url: Uri,
    // pub(crate) broker_domain: String,
}

fn load_local_targets(broker_id: &BrokerId) -> Vec<LocalMappingEntry> {
    example_targets::example_local(broker_id) //TODO: Read from env, file, etc.
}

fn load_public_targets(broker_id: &BrokerId) -> CentralMapping {
    example_targets::example_central(broker_id) //TODO: Read from env, file, etc.
}

impl Config {
    pub(crate) fn load() -> Result<Self,Box<dyn Error>> {
        // for key in ["PKI_ADDRESS", "BROKER_URL"] {
        //     std::env::set_var(key, "http://invalidhost.localhost"); // req'd for shared library (TODO: Improve CLI parsing)
        //     debug!("Setting {}", key);
        // }
        let args = CliArgs::parse();
        let broker_id = app_to_broker_id(&args.app_id)?;
        AppId::set_broker_id(&broker_id);
        let my_app_id = AppId::new(&args.app_id)?;
        let broker_id = BrokerId::new(&broker_id)?;
        // let proxy_id = my_app_id.proxy_id();

        Ok(Config {
            proxy_url: args.proxy_url,
            my_app_id: my_app_id.clone(),
            proxy_auth: format!("ApiKey {} {}", my_app_id, args.proxy_apikey),
            bind_addr: args.bind_addr,
            targets_local: load_local_targets(&broker_id),
            targets_public: load_public_targets(&broker_id)
        })
    }
}

