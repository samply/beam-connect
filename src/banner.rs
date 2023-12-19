use tracing::info;

pub fn print_banner() {
    let commit = match env!("GIT_DIRTY") {
        "false" => {
            env!("GIT_COMMIT_SHORT")
        },
        _ => {
            "SNAPSHOT"
        }
    };
    info!("🌈 Samply.Connect ({}) v{} (built {} {}, {}) starting up ...", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"), env!("BUILD_DATE"), env!("BUILD_TIME"), commit);
}

pub(crate) fn print_startup_app_config(config: &crate::Config) {
    info!("Communicating with Proxy {} using AppId {}. There are {} entries in the local and {} entries in the central target configuration.", config.proxy_url, config.my_app_id, config.targets_local.entries.len(), config.targets_public.sites.len());
}
