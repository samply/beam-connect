use hyper::http::uri::Authority;
use shared::beam_id::{AppId, BeamId};

use crate::config::{CentralMapping, LocalMappingEntry};

pub(crate) fn example_central() -> CentralMapping {
    let app1_id = AppId::new("connect1.proxy23.localhost").unwrap();
    let app2_id = AppId::new("connect2.proxy23.localhost").unwrap();
    let map = [
        ("/site1/", app1_id),
        ("/site2/", app2_id)
    ].map(|(k,v)| (Authority::from_static(k), v))
    .into_iter().collect();
    CentralMapping { map }
}

pub(crate) fn example_local() -> Vec<LocalMappingEntry> {
    let app1_id = AppId::new("connect1.proxy23.localhost").unwrap();
    let app2_id = AppId::new("connect2.proxy23.localhost").unwrap();
    let map = [
        ("ifconfig.me", "ifconfig.me", vec![app1_id.clone(), app2_id.clone()]),
        ("ip-api.com", "ip-api.com", vec![app1_id.clone(), app2_id.clone()]),
        ("wttr.in", "wttr.in", vec![app1_id.clone(), app2_id.clone()]),
        ("internalhost23", "host23.internal.network", vec![app1_id, app2_id])
    ].map(|(needle,replace,allowed)| LocalMappingEntry {
        needle: Authority::from_static(needle),
        replace: Authority::from_static(replace),
        allowed
    })
    .into_iter()
    .collect();

    map
}
