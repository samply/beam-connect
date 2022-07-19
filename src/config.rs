use std::error::Error;
use std::collections::HashMap;
use std::path::{PathBuf,Path};
use std::io::{prelude::*, BufReader};

use clap::Parser;
use hyper::Uri;
use shared::beam_id::{AppId, BeamId};
use crate::{structs::InternalHost,errors::BeamConnectError};

/// Settings for Samply.Beam (Shared)
#[derive(Parser,Debug)]
#[clap(author, version, about, long_about = None)]
struct CliArgs {
    // /// samply.pki: URL to HTTPS endpoint
    // #[clap(long, env, value_parser)]
    // pki_address: Uri,

    // /// samply.pki: Authentication realm
    // #[clap(long, env, value_parser, default_value = "samply_pki")]
    // pki_realm: String,

    // /// samply.pki: File containing the authentication token
    // #[clap(long, env, value_parser, default_value = "/run/secrets/pki.secret")]
    // pki_apikey_file: PathBuf,

    // /// samply.pki: Path to own secret key
    // #[clap(long, env, value_parser, default_value = "/run/secrets/privkey.pem")]
    // privkey_file: PathBuf,

    // TODO: The following arguments have been added for compatibility reasons with the proxy config. Find another way to merge configs.
    // /// (included for technical reasons)
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

    /// File containing the endpoint allow list
    #[clap(long, env, value_parser, default_value = "/conf/targets.conf")]
    pub target_conf_file: PathBuf,

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

#[derive(Clone)]
#[allow(dead_code)]
pub(crate) struct Config {
    pub(crate) proxy_url: Uri,
    pub(crate) my_app_id: AppId,
    pub(crate) proxy_auth: String,
    pub(crate) bind_addr: String,
    pub(crate) targets: HashMap<InternalHost, AppId>,
    // pub(crate) pki_address: Uri,
    // pub(crate) pki_realm: String,
    // pub(crate) pki_apikey: String,
    // pub(crate) privkey_rs256: RS256KeyPair,
    // pub(crate) privkey_rsa: RsaPrivateKey,
    // pub(crate) http_proxy: Option<Uri>,
    // // pub(crate) broker_url: Uri,
    // pub(crate) broker_domain: String,
}

impl Config {
    pub(crate) fn load() -> Result<Self,Box<dyn Error>> {
        // for key in ["PKI_ADDRESS", "BROKER_URL"] {
        //     std::env::set_var(key, "http://invalidhost.localhost"); // req'd for shared library (TODO: Improve CLI parsing)
        //     debug!("Setting {}", key);
        // }
        let args = CliArgs::parse();
        AppId::set_broker_id(shared::beam_id::app_to_broker_id(&args.app_id)?);
        let my_app_id = AppId::new(&args.app_id)?;
        let targets = parse_target_conf(&args.target_conf_file)?;
        // let proxy_id = my_app_id.proxy_id();

        Ok(Config {
            proxy_url: args.proxy_url,
            my_app_id: my_app_id.clone(),
            proxy_auth: format!("ApiKey {} {}", my_app_id, args.proxy_apikey),
            bind_addr: args.bind_addr,
            targets,
        })
    }
}

fn parse_target_conf(file: &Path) -> Result<HashMap<InternalHost, AppId>, BeamConnectError> {
    if !file.is_file() {
        return Err(BeamConnectError::InvalidConfiguration(format!("{} is a directory or does not exist", file.to_string_lossy())));
    }
    let file_handle = std::fs::File::open(file)?;
    let reader = BufReader::new(file_handle);
    let mut result = HashMap::new();
    for line in reader.lines() {
        let line = line?;
        let mut split = line.split(',');
        let uri = split.next();
        if uri.is_none() {
            return Err(BeamConnectError::InvalidConfiguration(format!("invalid line in config file {}", file.to_string_lossy())));
        }
        let target = Uri::try_from(uri.unwrap()).unwrap().authority().unwrap().to_owned();
        let app = split.next();
        if app.is_none() {
            return Err(BeamConnectError::InvalidConfiguration(format!("invalid line in config file {}", file.to_string_lossy())));
        }
        let app_id = AppId::new(app.unwrap())?;
        result.insert(target,app_id);
    }
    Ok(result)
}
